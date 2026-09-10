//! Translator 集成测试：本地 mock OpenAI 兼容服务器（原始 TCP），验证
//! 流式 byot 全链路（R-13 运行时验证）、先带后撤、usage、json 提取、
//! 重复检测、超时与错误分类、上下文历史。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lt_translate::{Translator, TranslatorParams};
use serde_json::{json, Value};

/// 每连接一个 handler：读完整请求（头+体），按请求文本决定响应字节。
type Handler = Arc<dyn Fn(&str) -> Vec<u8> + Send + Sync>;

struct MockServer {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl MockServer {
    fn start(handler: Handler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind 失败");
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let req_clone = requests.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut stream) = conn else { break };
                let req_clone = req_clone.clone();
                let handler = handler.clone();
                std::thread::spawn(move || {
                    let Ok(text) = read_request(&mut stream) else {
                        return;
                    };
                    req_clone.lock().unwrap().push(text.clone());
                    let resp = handler(&text);
                    let _ = stream.write_all(&resp);
                    let _ = stream.flush();
                    // 给客户端留出读响应的时间，再关闭
                    std::thread::sleep(Duration::from_millis(200));
                });
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            requests,
        }
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<String> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(pos) = find_header_end(&data) {
            let body_len: usize = parse_content_length(&data[..pos]).unwrap_or(0);
            if data.len() >= pos + 4 + body_len {
                break;
            }
        }
    }
    Ok(String::from_utf8_lossy(&data).into_owned())
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_content_length(headers: &[u8]) -> Option<usize> {
    let text = String::from_utf8_lossy(headers);
    for line in text.lines() {
        if let Some(v) = line
            .strip_prefix("content-length:")
            .or_else(|| line.strip_prefix("Content-Length:"))
        {
            return v.trim().parse().ok();
        }
    }
    None
}

/// SSE 响应（带 Content-Length，写完即留窗后关闭）
fn sse_response(events: &[String]) -> Vec<u8> {
    let mut body = String::new();
    for e in events {
        body.push_str(&format!("data: {e}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes()
    .into_iter()
    .chain(body.into_bytes())
    .collect()
}

/// OpenAI 流式 chunk（content delta）
fn chunk_delta(content: &str) -> String {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "test-model",
        "choices": [{"index": 0, "delta": {"content": content}, "finish_reason": null}],
    })
    .to_string()
}

/// usage 统计 chunk（choices 为空）
fn chunk_usage(pt: u64, ct: u64) -> String {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "test-model",
        "choices": [],
        "usage": {"prompt_tokens": pt, "completion_tokens": ct, "total_tokens": pt + ct},
    })
    .to_string()
}

/// usage chunk（带推理量明细；W2 体检输入）
fn chunk_usage_detailed(pt: u64, ct: u64, reasoning: u64) -> String {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "test-model",
        "choices": [],
        "usage": {
            "prompt_tokens": pt,
            "completion_tokens": ct,
            "total_tokens": pt + ct,
            "completion_tokens_details": {"reasoning_tokens": reasoning},
        },
    })
    .to_string()
}

/// 结束标记 chunk（只有 finish_reason，不含 content 与 usage）
fn chunk_finish(reason: &str) -> String {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "test-model",
        "choices": [{"index": 0, "delta": {}, "finish_reason": reason}],
    })
    .to_string()
}

fn non_streaming_response(status_line: &str, body: &Value) -> Vec<u8> {    let body = body.to_string();
    format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn completion_response(content: &str, pt: u64, ct: u64) -> Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 1,
        "model": "test-model",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": content}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": pt, "completion_tokens": ct, "total_tokens": pt + ct},
    })
}

fn error_response(status: u16, message: &str) -> Vec<u8> {
    let status_line = match status {
        400 => "400 Bad Request",
        401 => "401 Unauthorized",
        _ => "500 Internal Server Error",
    };
    non_streaming_response(
        status_line,
        &json!({"error": {"message": message, "type": "invalid_request_error"}}),
    )
}

fn translator(base_url: &str) -> Translator {
    Translator::new(TranslatorParams {
        api_base: base_url.into(),
        api_key: "test-key".into(),
        model: "test-model".into(),
        ..TranslatorParams::default()
    })
    .unwrap()
}

/// 从捕获的原始请求中取 JSON 体
fn request_body(req: &str) -> Value {
    let pos = find_header_end(req.as_bytes()).expect("请求含头");
    serde_json::from_str(req[pos + 4..].trim()).expect("请求体为 JSON")
}

// ── 流式全链路 ──

#[allow(clippy::while_let_on_iterator)]
#[test]
fn streaming_yields_partials_then_final_with_usage() {
    let server = MockServer::start(Arc::new(|_| {
        sse_response(&[
            chunk_delta("你好"),
            chunk_delta("，"),
            chunk_delta("世界"),
            chunk_usage(11, 7),
        ])
    }));
    let t = translator(&server.base_url);
    let mut partials = Vec::new();
    let mut final_text = String::new();
    let mut it = t.translate_iter("hello world", "en", "zh", 10, 0);
    while let Some(item) = it.next() {
        match item {
            Ok(p) => {
                final_text = p.clone();
                partials.push(p);
            }
            Err(e) => panic!("流式失败: {e}"),
        }
    }
    // 3 个 delta → 3 次部分产出 + 1 次最终值
    assert_eq!(partials.len(), 4, "partials: {partials:?}");
    assert_eq!(partials[0], "你好");
    assert_eq!(partials[1], "你好，");
    assert_eq!(partials[2], "你好，世界");
    assert_eq!(final_text, "你好，世界");
    // 用量随迭代器返回（不再经共享态回传）
    assert_eq!(it.usage(), (11, 7));
    assert!(it.usage_known(), "服务端回了 usage，应标记为已知");
    // 请求体形状
    let body = request_body(&server.requests()[0]);
    assert_eq!(body["model"], "test-model");
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    // W1/方案 §2.1 + 2026-09-10 裁决：默认不发送长度上限，**也不发送温度**
    // （高级参数默认全部不发送，只有用户手动指定才发）
    assert!(body.get("max_tokens").is_none(), "默认不应发送 max_tokens");
    assert!(body.get("temperature").is_none(), "默认不应发送 temperature");
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"].as_array().unwrap().len(), 2);
}

/// W2/INV-F：写进 content 的思考块（含跨分片标签）不得出现在任何增量或最终译文
// while let 是刻意的：for 会移走迭代器，之后读不到 verdict()/usage()（方案 §4）
#[allow(clippy::while_let_on_iterator)]
#[test]
fn streaming_strips_inline_reasoning_never_leaks() {
    // 标签用转义书写：源码中的完整标签字面量曾被编辑环节间歇性改写
    let open = "\u{3c}think\u{3e}".to_string();
    let close = "\u{3c}/think\u{3e}".to_string();
    let server = MockServer::start(Arc::new(move |_| {
        sse_response(&[
            chunk_delta(&open),                    // 开标签单独一片
            chunk_delta("先分析一下用户的问题"),   // 思考正文
            chunk_delta(&close),                   // 闭标签单独一片
            chunk_delta("译文在这里"),
            chunk_usage(10, 20),
        ])
    }));
    let t = translator(&server.base_url);
    let mut partials = Vec::new();
    let mut it = t.translate_iter("hello", "en", "zh", 10, 0);
    // 必须用 while let（for 会移走迭代器，之后读不到体检结论）
    while let Some(item) = it.next() {
        match item {
            Ok(p) => partials.push(p),
            Err(e) => panic!("流式失败: {e}"),
        }
    }
    for p in &partials {
        assert!(!p.contains("分析"), "增量泄露了思考: {p:?}");
        assert!(!p.contains("think"), "增量泄露了标签: {p:?}");
    }
    assert_eq!(partials.last().map(String::as_str), Some("译文在这里"));
    // W2：体检结论与用量随迭代器返回（不再经共享态）
    assert_eq!(it.verdict(), Some(lt_translate::ResponseVerdict::Ok));
    assert_eq!(it.usage(), (10, 20));
}

/// W2/方案 §4.4：正文恒空 + 推理量 > 0 → EmptyReasoningBudget（本次故障的判定）
#[allow(clippy::while_let_on_iterator)]
#[test]
fn verdict_reports_reasoning_budget_burn() {
    let server = MockServer::start(Arc::new(|_| {
        sse_response(&[chunk_usage_detailed(142, 256, 256), chunk_delta("")])
    }));
    let t = translator(&server.base_url);
    let mut it = t.translate_iter("hello", "en", "zh", 10, 0);
    let mut last: Option<Result<String, _>> = None;
    while let Some(item) = it.next() {
        last = Some(item);
    }
    assert_eq!(last.unwrap().unwrap(), "", "正文为空");
    assert_eq!(
        it.verdict(),
        Some(lt_translate::ResponseVerdict::EmptyReasoningBudget)
    );
    assert!(!it.verdict().unwrap().has_text());
    assert_eq!(it.usage(), (142, 256));
}

#[test]
fn stream_options_retracted_when_rejected() {
    // 带 stream_options 的请求 → 400 拒绝；重试（不带）→ SSE 成功
    let server = MockServer::start(Arc::new(|req| {
        if req.contains("stream_options") {
            error_response(400, "Unknown field: stream_options")
        } else {
            sse_response(&[chunk_delta("ok"), chunk_usage(3, 2)])
        }
    }));
    let t = translator(&server.base_url);
    let mut it = t.translate_iter("hello", "en", "zh", 10, 0);
    let mut text = String::new();
    #[allow(clippy::while_let_on_iterator)]
    // while let 是刻意的：跑完还要读 verdict()/usage()（方案 §4）
    while let Some(item) = it.next() {
        text = item.expect("重试后应成功");
    }
    assert_eq!(text, "ok");
    assert_eq!(it.usage(), (3, 2));
    let reqs = server.requests();
    assert_eq!(reqs.len(), 2, "应发起两次请求: {reqs:?}");
    assert!(request_body(&reqs[0]).get("stream_options").is_some());
    assert!(request_body(&reqs[1]).get("stream_options").is_none());
}

#[test]
fn json_response_mode_yields_only_final() {
    let server = MockServer::start(Arc::new(|_| {
        sse_response(&[
            chunk_delta("{\"t\": \""),
            chunk_delta("译文内容\"}"),
            chunk_usage(5, 9),
        ])
    }));
    let t = Translator::new(TranslatorParams {
        api_base: server.base_url.clone(),
        json_response: true,
        ..TranslatorParams::default()
    })
    .unwrap();
    let items: Vec<_> = t.translate_iter("hello", "en", "zh", 10, 0).collect();
    assert_eq!(items.len(), 1, "json 模式只产最终值: {items:?}");
    assert_eq!(items.into_iter().next().unwrap().unwrap(), "译文内容");
}

#[test]
fn sync_translate_returns_content_and_usage() {
    let server = MockServer::start(Arc::new(|_| {
        non_streaming_response("200 OK", &completion_response("  你好世界  ", 20, 10))
    }));
    let t = Translator::new(TranslatorParams {
        api_base: server.base_url.clone(),
        streaming: false,
        ..TranslatorParams::default()
    })
    .unwrap();
    let mut it = t.translate_iter("hello", "en", "zh", 10, 0);
    let mut text = String::new();
    #[allow(clippy::while_let_on_iterator)]
    // while let 是刻意的：跑完还要读 verdict()/usage()（方案 §4）
    while let Some(item) = it.next() {
        text = item.expect("非流式应成功");
    }
    assert_eq!(text, "你好世界"); // trim 语义
    assert_eq!(it.usage(), (20, 10));
    assert!(it.usage_known(), "响应带 usage → 已知");
    let body = request_body(&server.requests()[0]);
    // 原版 kwargs：stream=false 时不写 stream 键（SDK 默认非流式）
    assert!(body.get("stream").is_none());
    assert!(body.get("stream_options").is_none());
}

#[test]
fn timeout_classified_when_server_stalls() {
    let server = MockServer::start(Arc::new(|_| {
        std::thread::sleep(Duration::from_secs(3));
        sse_response(&[chunk_delta("late")])
    }));
    let t = translator(&server.base_url);
    // W4：超时逐调用传入（设置总线 tl.timeout 生效值）
    let err = t.translate("hello", "en", "zh", 1, 0).expect_err("应超时");
    assert!(
        matches!(err, lt_translate::TranslateError::Timeout(_)),
        "实际: {err:?}"
    );
    assert!(err.is_expected());
    assert_eq!(
        err.ui_text(),
        "[error: Translation exceeded 1s total timeout]"
    );
}

#[test]
fn repetition_error_detected() {
    // 60 字符同字符串：plen=8 时 text[8:16] == text[:8] 必然成立
    let loop_text = "a".repeat(60);
    let server = MockServer::start(Arc::new(move |_| {
        sse_response(&[chunk_delta(&loop_text), chunk_usage(9, 9)])
    }));
    let t = translator(&server.base_url);
    let err = t.translate("hello", "en", "zh", 10, 0).expect_err("应检出重复");
    assert!(
        matches!(err, lt_translate::TranslateError::Repetition(_)),
        "实际: {err:?}"
    );
}

#[test]
fn auth_error_classified() {
    let server = MockServer::start(Arc::new(|_| error_response(401, "Invalid API key")));
    let t = translator(&server.base_url);
    let err = t.translate("hello", "en", "zh", 10, 0).expect_err("应失败");
    assert!(
        matches!(err, lt_translate::TranslateError::Auth { code: 401, .. }),
        "实际: {err:?}"
    );
}

// ── prompt / 上下文 ──

#[test]
fn context_history_appended_to_messages() {
    let server = MockServer::start(Arc::new(|_| {
        non_streaming_response("200 OK", &completion_response("译文", 1, 1))
    }));
    let t = Translator::new(TranslatorParams {
        api_base: server.base_url.clone(),
        streaming: false,
        ..TranslatorParams::default()
    })
    .unwrap();
    t.set_context_turns(2);
    t.translate("第一句", "en", "zh", 10, 1).unwrap();
    t.translate("第二句", "en", "zh", 10, 2).unwrap();
    let body = request_body(&server.requests()[1]);
    let msgs = body["messages"].as_array().unwrap();
    // system + (user=第一句, assistant=译文) + user=第二句
    assert_eq!(msgs.len(), 4, "messages: {msgs:?}");
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs[1]["content"], "第一句");
    assert_eq!(msgs[2]["role"], "assistant");
    assert_eq!(msgs[2]["content"], "译文");
    assert_eq!(msgs[3]["content"], "第二句");
}

#[test]
fn no_system_role_merges_prompt_into_user() {
    let server = MockServer::start(Arc::new(|_| {
        non_streaming_response("200 OK", &completion_response("x", 1, 1))
    }));
    let t = Translator::new(TranslatorParams {
        api_base: server.base_url.clone(),
        streaming: false,
        no_system_role: true,
        ..TranslatorParams::default()
    })
    .unwrap();
    t.translate("hello", "en", "zh", 10, 0).unwrap();
    let body = request_body(&server.requests()[0]);
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["role"], "user");
    let content = msgs[0]["content"].as_str().unwrap();
    assert!(content.contains("real-time subtitle translator"));
    assert!(content.ends_with("hello"));
}

#[test]
fn language_display_names_in_prompt() {
    let server = MockServer::start(Arc::new(|_| {
        sse_response(&[chunk_delta("x"), chunk_usage(1, 1)])
    }));
    let t = Translator::new(TranslatorParams {
        api_base: server.base_url.clone(),
        ..TranslatorParams::default()
    })
    .unwrap();
    t.translate("hello", "ja", "zh", 10, 0).unwrap();
    let body = request_body(&server.requests()[0]);
    let sys = body["messages"][0]["content"].as_str().unwrap();
    assert!(
        sys.contains("Translate Japanese into Chinese"),
        "system prompt: {sys}"
    );
}

/// W4：目标语言逐调用传入（此调用用「ja」→ prompt 变 Japanese；
/// 下一次调用传回「zh」即恢复——不再有 set_target_language 运行时可变面）
#[test]
fn target_language_per_call_affects_request() {
    let server = MockServer::start(Arc::new(|_| {
        sse_response(&[chunk_delta("x"), chunk_usage(1, 1)])
    }));
    let t = translator(&server.base_url);
    t.translate("hello", "en", "ja", 10, 0).unwrap();
    let body = request_body(&server.requests()[0]);
    let sys = body["messages"][0]["content"].as_str().unwrap();
    assert!(sys.contains("into Japanese"), "system prompt: {sys}");
    // 下一次调用传回 zh → 不复用上一调的 ja
    t.translate("hallo", "en", "zh", 10, 1).unwrap();
    let body = request_body(&server.requests()[1]);
    let sys = body["messages"][0]["content"].as_str().unwrap();
    assert!(sys.contains("into Chinese"), "system prompt: {sys}");
}

// ── overrides / response_format ──

/// 第二轮评审 ②：界面已撤下的覆写键（温度/输出上限/seed）**一律不参与请求**——
/// 老档案里的残值不得偷偷改行为（否则"不再发送 max_tokens"的修复会被旧值反杀）
#[test]
fn hidden_override_keys_are_ignored() {
    let server = MockServer::start(Arc::new(|_| {
        sse_response(&[chunk_delta("x"), chunk_usage(1, 1)])
    }));
    let overrides = [
        ("temperature".to_string(), json!(0.7)),
        ("top_p".to_string(), json!(0.9)),
        ("max_tokens".to_string(), json!(128)),
        ("seed".to_string(), json!(42)),
    ]
    .into_iter()
    .collect();
    let t = Translator::new(TranslatorParams {
        api_base: server.base_url.clone(),
        overrides: Some(overrides),
        ..TranslatorParams::default()
    })
    .unwrap();
    t.translate("hello", "en", "zh", 10, 0).unwrap();
    let body = request_body(&server.requests()[0]);
    // 仍在界面上的键照发
    assert_eq!(body["top_p"], 0.9);
    // 已撤出界面的键一律不发（温度走一等字段，默认未指定）
    assert!(body.get("temperature").is_none(), "隐藏温度不得反杀: {body}");
    assert!(body.get("max_tokens").is_none(), "隐藏上限不得反杀: {body}");
    assert!(body.get("seed").is_none(), "隐藏 seed 不得参与: {body}");
}

/// 一等字段是温度的**唯一**生效路径（用户手动指定才发）
#[test]
fn first_class_temperature_wins_over_nothing() {
    let server = MockServer::start(Arc::new(|_| {
        sse_response(&[chunk_delta("x"), chunk_usage(1, 1)])
    }));
    let t = Translator::new(TranslatorParams {
        api_base: server.base_url.clone(),
        temperature: Some(0.7),
        ..TranslatorParams::default()
    })
    .unwrap();
    t.translate("hello", "en", "zh", 10, 0).unwrap();
    let body = request_body(&server.requests()[0]);
    assert_eq!(body["temperature"], 0.7);
}

#[test]
fn json_response_sends_json_schema_format() {
    let server = MockServer::start(Arc::new(|_| {
        sse_response(&[chunk_delta("x"), chunk_usage(1, 1)])
    }));
    let t = Translator::new(TranslatorParams {
        api_base: server.base_url.clone(),
        json_response: true,
        ..TranslatorParams::default()
    })
    .unwrap();
    t.translate("hello", "en", "zh", 10, 0).unwrap();
    let body = request_body(&server.requests()[0]);
    let sys = body["messages"][0]["content"].as_str().unwrap();
    assert!(
        sys.contains("Respond in JSON format"),
        "system prompt: {sys}"
    );
    let rf = &body["response_format"];
    assert_eq!(rf["type"], "json_schema");
    assert_eq!(rf["json_schema"]["name"], "translation");
    assert_eq!(rf["json_schema"]["strict"], true);
    assert_eq!(rf["json_schema"]["schema"]["required"][0], "t");
}

#[test]
fn thinking_body_merged_into_request() {
    // W1/方案 §2.3：默认总开关开 + 自动首选 → 本机端点发 reasoning_effort:"none"
    // （旧行为误发 enable_thinking，实测对本机 LM Studio 无效）
    let t = Translator::new(TranslatorParams {
        api_base: "http://127.0.0.1:1/v1".into(), // 死端口：仅本地构造请求体，不触网
        ..TranslatorParams::default()
    })
    .unwrap();
    let body = t.build_request_body("system", "text", true, false, 0);
    assert_eq!(body["reasoning_effort"], "none");
    assert!(body.get("enable_thinking").is_none(), "不得再盲发 enable_thinking");
}

#[test]
fn request_body_omits_optional_params_when_absent() {
    // W1/方案 §2.1 + 2026-09-10 裁决：长度上限与温度**默认都不发送**
    // （应用强加上限会把"先想再答"的模型憋死；高级参数默认全部不发）
    let t = Translator::new(TranslatorParams {
        api_base: "http://127.0.0.1:1/v1".into(),
        ..TranslatorParams::default()
    })
    .unwrap();
    let body = t.build_request_body("system", "text", true, false, 0);
    assert!(body.get("max_tokens").is_none(), "默认不应发送 max_tokens");
    assert!(
        body.get("temperature").is_none(),
        "默认不应发送 temperature（高级参数默认关闭）"
    );

    // 用户手动指定 → 照发
    let t = Translator::new(TranslatorParams {
        api_base: "http://127.0.0.1:1/v1".into(),
        temperature: Some(0.2),
        max_tokens: Some(1024),
        ..TranslatorParams::default()
    })
    .unwrap();
    let body = t.build_request_body("system", "text", true, false, 0);
    assert_eq!(body["temperature"], 0.2);
    assert_eq!(body["max_tokens"], 1024);

    // 总开关关（= 旧 "off" 语义）→ 不发任何推理参数
    let t = Translator::new(TranslatorParams {
        api_base: "http://127.0.0.1:1234/v1".into(),
        disable_thinking: false,
        ..TranslatorParams::default()
    })
    .unwrap();
    let body = t.build_request_body("system", "text", true, false, 0);
    assert!(body.get("reasoning_effort").is_none());
    assert!(body.get("enable_thinking").is_none());

    // 已确认关不掉的模型（持久化标记）→ 同样不发
    let t = Translator::new(TranslatorParams {
        api_base: "http://127.0.0.1:1234/v1".into(),
        thinking_unavailable: true,
        ..TranslatorParams::default()
    })
    .unwrap();
    let body = t.build_request_body("system", "text", true, false, 0);
    assert!(body.get("reasoning_effort").is_none());
    assert!(body.get("thinking").is_none());
}

/// 审计 R2：最小请求必须连**已撤出界面的** `json_response` 一起丢掉——
/// 它会往请求里塞 response_format 并改写系统提示词，端点拒绝时"最小"也无路可退
#[test]
fn minimal_drops_hidden_json_response() {
    let t = Translator::new(TranslatorParams {
        api_base: "http://127.0.0.1:1234/v1".into(),
        model: "test-model".into(),
        json_response: true,
        ..TranslatorParams::default()
    })
    .unwrap();
    let normal = t.build_request_body("system", "text", true, false, 0);
    assert!(
        normal.get("response_format").is_some(),
        "常规请求仍按其配置发送: {normal}"
    );

    let minimal = t.minimal();
    let body = minimal.build_request_body("system", "text", true, false, 0);
    assert!(
        body.get("response_format").is_none(),
        "最小请求不得带 response_format: {body}"
    );
    // 系统提示词的 "Respond in JSON format" 追加发生在 build_system_prompt（调用路径），
    // 由 mock 服务器用例 `json_response_sends_json_schema_format` 从请求体侧钉住；
    // 这里只需确认 minimal 装置的 json_response 配置面已关（上面 response_format 即证）
}

/// 最小请求：只剩「模型 + 系统提示词 + 文本 + 流式」，可选参数一个不带
#[test]
fn minimal_request_drops_every_optional_field() {
    let t = Translator::new(TranslatorParams {
        api_base: "http://127.0.0.1:1234/v1".into(),
        model: "test-model".into(),
        temperature: Some(0.7),
        max_tokens: Some(512),
        extra_body: Some(json!({"top_k": 5})),
        ..TranslatorParams::default()
    })
    .unwrap();
    let normal = t.build_request_body("system", "text", true, false, 0);
    assert_eq!(normal["temperature"], 0.7);

    let minimal = t.minimal();
    let body = minimal.build_request_body("system", "text", true, false, 0);
    assert!(body.get("temperature").is_none(), "最小请求不带温度: {body}");
    assert!(body.get("max_tokens").is_none(), "最小请求不带输出上限");
    assert!(body.get("top_k").is_none(), "最小请求不带用户额外参数");
    assert!(body.get("reasoning_effort").is_none(), "最小请求不带推理参数");
    assert!(body.get("thinking").is_none());
    // 翻译所必需的三件仍在：模型名 + 系统提示词 + 待译文本
    assert_eq!(body["model"], "test-model");
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][1]["content"], "text");
}

/// 第二轮评审 ①（回归守卫）：派生副本（回退阶梯每级一个）必须**共享会话记忆**——
/// 旧实现给每次提交派生"空白状态"的副本，导致「上下文数」整体失效
#[test]
fn derived_translator_shares_conversation_memory() {
    let server = MockServer::start(Arc::new(|_| {
        non_streaming_response("200 OK", &completion_response("译文", 2, 3))
    }));
    let t = Translator::new(TranslatorParams {
        api_base: server.base_url.clone(),
        streaming: false,
        ..TranslatorParams::default()
    })
    .unwrap();
    t.set_context_turns(2);
    t.translate("hi", "en", "zh", 10, 1).unwrap();
    // 派生副本（share_client / 阶梯每级）看得见原实例的历史
    let clone = t.with_plan(lt_translate::ThinkingPlan::None);
    clone.translate("bonjour", "en", "fr", 10, 2).unwrap();
    let body = request_body(&server.requests()[1]);
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 4, "派生副本应带上历史: {msgs:?}");
    assert_eq!(msgs[1]["content"], "hi");
    assert_eq!(msgs[2]["content"], "译文");
}

/// 并发语义（第二轮评审 ①）：上下文只取**提交序号更早**的句子，
/// 绝不把后提交的"未来句"当上下文
#[test]
fn context_never_uses_later_sequences() {
    let server = MockServer::start(Arc::new(|_| {
        non_streaming_response("200 OK", &completion_response("译文", 1, 1))
    }));
    let t = Translator::new(TranslatorParams {
        api_base: server.base_url.clone(),
        streaming: false,
        ..TranslatorParams::default()
    })
    .unwrap();
    t.set_context_turns(2);
    // 乱序完成：先落 3 号（"第三句"），再翻 2 号
    t.translate("第三句", "en", "zh", 10, 3).unwrap();
    t.translate("第二句", "en", "zh", 10, 2).unwrap();
    let body = request_body(&server.requests()[1]);
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(
        msgs.len(),
        2,
        "3 号是本次(2 号)之后的句子，不得作为上下文: {msgs:?}"
    );
}

/// 第二轮评审 ⑩：服务端完全不给 usage 时，用量标记为"未知"（界面据此显示"—"）
#[test]
fn missing_usage_is_reported_as_unknown() {
    let server = MockServer::start(Arc::new(|_| {
        // 无 usage 分片：只有正文与结束标记
        sse_response(&[chunk_delta("译文"), chunk_finish("stop")])
    }));
    let t = translator(&server.base_url);
    let mut it = t.translate_iter("hello", "en", "zh", 10, 0);
    let mut last = String::new();
    #[allow(clippy::while_let_on_iterator)]
    // while let 是刻意的：跑完还要读 verdict()/usage()（方案 §4）
    while let Some(item) = it.next() {
        last = item.expect("应成功");
    }
    assert_eq!(last, "译文");
    assert!(!it.usage_known(), "服务端未返回 usage → 未知");
    assert_eq!(it.usage(), (0, 0));
}

// ── bench 冒烟（mock 服务器） ──

/// 方案 §2.5 规则 6（第三轮复核补齐）：**预览 == 实发**——预览体必须由与真实
/// 请求同一个构造函数产出，逐键一致（含关闭思考参数、高级参数政策、流式选项）
#[test]
fn preview_matches_actual_body() {
    let server = MockServer::start(Arc::new(|_| {
        sse_response(&[chunk_delta("好"), chunk_usage(3, 1)])
    }));
    let t = Translator::new(TranslatorParams {
        api_base: server.base_url.clone(),
        model: "test-model".into(),
        temperature: Some(0.7),
        ..TranslatorParams::default()
    })
    .unwrap();
    let mut it = t.translate_iter("hello", "en", "zh", 10, 1);
    while it.next().is_some() {}

    let actual = request_body(&server.requests()[0]);
    // 预览：用实发请求里的 system prompt 回灌同一构造函数（键集与全部非消息字段
    // 必须逐键一致——任何"只在一侧加参数"的漂移都会被这条挡下）
    let sys = actual["messages"][0]["content"].as_str().unwrap().to_string();
    let preview = t.build_request_body(&sys, "hello", true, true, 1);
    assert_eq!(preview, actual, "预览体与实发体不一致");
    // 顺带钉住参数面：关闭思考参数在、温度照发、无输出上限
    assert_eq!(actual["reasoning_effort"], "none");
    assert_eq!(actual["temperature"], 0.7);
    assert!(actual.get("max_tokens").is_none());
    assert_eq!(actual["stream_options"]["include_usage"], true);
}

/// 基准与生产同判据（第三轮复核）：强制思考模型（对关闭参数回 400）在基准页
/// 不该显示 FAILED——按同一条阶梯退级后必须测出数据
#[test]
fn benchmark_degrades_like_production_on_param_rejection() {
    let server = MockServer::start(Arc::new(|req| {
        if req.contains("reasoning_effort") {
            error_response(400, "Unknown field: reasoning_effort")
        } else if req.contains("\"stream\":true") || req.contains("\"stream\": true") {
            sse_response(&[chunk_delta("答"), chunk_delta("案"), chunk_usage(2, 2)])
        } else {
            non_streaming_response("200 OK", &completion_response("答案", 2, 2))
        }
    }));
    let model = lt_translate::bench::BenchModel {
        name: "forced-thinking".into(),
        api_base: server.base_url.clone(),
        api_key: "k".into(),
        model: "test-model".into(),
        proxy: "none".into(),
        no_system_role: false,
        disable_thinking: true,
        thinking_style: None,
    };
    let results = lt_translate::bench::run_benchmark_blocking(&[model], "en", 5, "translate: {text}");
    assert_eq!(results.len(), 1);
    assert!(
        results[0].error.is_none(),
        "参数被拒时应退级重测而非直接 FAILED: {:?}",
        results[0].error
    );
    assert_eq!(results[0].rounds.len(), 5);
    // 退级后的请求不再带关闭参数（第二级形状）
    let bodies: Vec<Value> = server
        .requests()
        .iter()
        .map(|r| request_body(r))
        .filter(|b| b.get("enable_thinking").is_some())
        .collect();
    assert!(!bodies.is_empty(), "应发出退级后的请求");
}

#[test]
fn benchmark_blocking_against_mock() {
    let server = MockServer::start(Arc::new(|req| {
        if req.contains("\"stream\":true") || req.contains("\"stream\": true") {
            sse_response(&[chunk_delta("答"), chunk_delta("案"), chunk_usage(2, 2)])
        } else {
            non_streaming_response("200 OK", &completion_response("答案", 2, 2))
        }
    }));
    let model = lt_translate::bench::BenchModel {
        name: "mock".into(),
        api_base: server.base_url.clone(),
        api_key: "k".into(),
        model: "test-model".into(),
        proxy: "none".into(),
        no_system_role: false,
        disable_thinking: true,
        thinking_style: None,
    };
    let results = lt_translate::bench::run_benchmark_blocking(
        &[model],
        "en",
        5,
        "translate {source_lang} to {target_lang}: {text}",
    );
    assert_eq!(results.len(), 1);
    assert!(results[0].error.is_none(), "error: {:?}", results[0].error);
    assert_eq!(results[0].rounds.len(), 5, "5 轮句组");
    assert!(results[0].avg_total > 0.0);
    // 第二轮评审 ⑫：基准必须走与生产**同一套请求构造**——不再硬编码
    // max_tokens:256 / temperature:0.3，且本地端点应带上关闭思考参数
    let body = request_body(&server.requests()[0]);
    assert!(body.get("max_tokens").is_none(), "基准不得自带输出上限: {body}");
    assert!(body.get("temperature").is_none(), "基准不得自带温度: {body}");
    assert_eq!(
        body["reasoning_effort"], "none",
        "本地端点的关闭思考参数应随生产构造一并生效: {body}"
    );
    assert!(results[0].avg_total > 0.0);
}
