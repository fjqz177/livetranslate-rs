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

fn non_streaming_response(status_line: &str, body: &Value) -> Vec<u8> {
    let body = body.to_string();
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
    for item in t.translate_iter("hello world", "en") {
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
    assert_eq!(t.last_usage(), (11, 7));
    // 请求体形状
    let body = request_body(&server.requests()[0]);
    assert_eq!(body["model"], "test-model");
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(body["max_tokens"], 256);
    assert_eq!(body["temperature"], 0.3);
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"].as_array().unwrap().len(), 2);
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
    let result: Result<String, lt_translate::TranslateError> = t.translate("hello", "en");
    let text = result.expect("重试后应成功");
    assert_eq!(text, "ok");
    assert_eq!(t.last_usage(), (3, 2));
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
    let items: Vec<_> = t.translate_iter("hello", "en").collect();
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
    let text = t.translate("hello", "en").unwrap();
    assert_eq!(text, "你好世界"); // trim 语义
    assert_eq!(t.last_usage(), (20, 10));
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
    let t = Translator::new(TranslatorParams {
        api_base: server.base_url.clone(),
        timeout: 1,
        ..TranslatorParams::default()
    })
    .unwrap();
    let err = t.translate("hello", "en").expect_err("应超时");
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
    let err = t.translate("hello", "en").expect_err("应检出重复");
    assert!(
        matches!(err, lt_translate::TranslateError::Repetition(_)),
        "实际: {err:?}"
    );
}

#[test]
fn auth_error_classified() {
    let server = MockServer::start(Arc::new(|_| error_response(401, "Invalid API key")));
    let t = translator(&server.base_url);
    let err = t.translate("hello", "en").expect_err("应失败");
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
    t.translate("第一句", "en").unwrap();
    t.translate("第二句", "en").unwrap();
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
    t.translate("hello", "en").unwrap();
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
        target_language: "zh".into(),
        ..TranslatorParams::default()
    })
    .unwrap();
    t.translate("hello", "ja").unwrap();
    let body = request_body(&server.requests()[0]);
    let sys = body["messages"][0]["content"].as_str().unwrap();
    assert!(
        sys.contains("Translate Japanese into Chinese"),
        "system prompt: {sys}"
    );
}

#[test]
fn target_language_switch_affects_next_request() {
    let server = MockServer::start(Arc::new(|_| {
        sse_response(&[chunk_delta("x"), chunk_usage(1, 1)])
    }));
    let t = translator(&server.base_url);
    t.set_target_language("ja");
    t.translate("hello", "en").unwrap();
    let body = request_body(&server.requests()[0]);
    let sys = body["messages"][0]["content"].as_str().unwrap();
    assert!(sys.contains("into Japanese"), "system prompt: {sys}");
}

// ── overrides / response_format ──

#[test]
fn overrides_appear_in_request_body() {
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
    t.translate("hello", "en").unwrap();
    let body = request_body(&server.requests()[0]);
    assert_eq!(body["temperature"], 0.7);
    assert_eq!(body["top_p"], 0.9);
    assert_eq!(body["max_tokens"], 128);
    assert_eq!(body["seed"], 42);
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
    t.translate("hello", "en").unwrap();
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
    // api_base 无 deepseek/glm 关键词 → qwen 风格（enable_thinking=false）；
    // legacy no_think=true → auto → 按端点解析
    let t = Translator::new(TranslatorParams {
        api_base: "http://127.0.0.1:1/v1".into(), // 死端口：仅本地构造请求体，不触网
        no_think: true,
        ..TranslatorParams::default()
    })
    .unwrap();
    let body = t.build_request_body("system", "text", true, false);
    assert_eq!(body["enable_thinking"], false);
}

#[test]
fn with_target_language_shares_settings_but_fresh_history() {
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
    t.translate("hi", "en").unwrap();
    let clone = t.with_target_language("fr");
    assert_eq!(clone.target_language(), "fr");
    assert_eq!(clone.last_usage(), (0, 0), "克隆后用量清零");
    // 原实例历史不受克隆影响，克隆历史为空
    clone.translate("bonjour", "en").unwrap();
    let body = request_body(&server.requests()[1]);
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2, "克隆实例无历史: {msgs:?}");
}

// ── bench 冒烟（mock 服务器） ──

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
}
