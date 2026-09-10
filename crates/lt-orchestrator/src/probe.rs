//! 供应商连接探测（D-85）：翻译页每行「测试」按钮的执行面。
//!
//! 设计要点（docs/translator-probe-hotswap.md §4.3）：
//! - **不依赖 ASR 线程**：由组合根经监督器起一次性线程调用 [`run_probe`]，
//!   管道繁忙/待命/未装配都能测；
//! - **判据与生产同源**：走同一条回退阶梯（[`crate::pipeline::run_ladder`]）
//!   与同一个参数构造点（[`crate::pipeline::translator_params`]）——关不掉思考的
//!   模型在生产里能出译文，探测就不该判失败；
//! - **只读观察**：不写也不读会话记忆（learned）与降级标记
//!   （degraded_notified），也不动 transcript —— 探测不改变生产装置的任何状态；
//! - **有时限**：总预算 10 秒（用户裁决 B），每次尝试的超时 = 用户超时与剩余预算取小；
//! - **可中断**：取消令牌直达流式读取循环（≤150ms 察觉）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lt_proto::{ProbeOutcome, UiEvent};

use crate::event_artery::EventSink;
use crate::pipeline::{run_ladder, translator_params, Halt, RunCtl};
use crate::settings_bus::SettingsBus;
use crate::Msg;

/// 单次探测的总预算（D-85 决策 B；常量真源在 lt-proto，UI 看门狗同源）
pub const PROBE_TOTAL_BUDGET: Duration = Duration::from_secs(lt_proto::PROBE_TOTAL_BUDGET_SECS);
/// 单步超时上限（用户超时可更短，不可更长）
pub const PROBE_STEP_TIMEOUT_CAP: u32 = lt_proto::PROBE_STEP_TIMEOUT_CAP_SECS;
/// 探测文本（与旧实现一致，避免改变判据语义；英文句便于任何目标语言出译文）
pub const PROBE_TEXT: &str = "Livetranslate test";
/// 回执摘要上限（字符）
pub const PROBE_PREVIEW_MAX_CHARS: usize = 60;

/// 执行一次连接探测；结果经 `sink` 回执 [`UiEvent::TestTranslatorResult`]。
///
/// `probe_id` 由 UI 发号（回执原样带回，供 UI 丢弃迟到/被取代的结果）。
/// 本函数在所有路径上都会**恰好推送一次**回执——不存在"静默失败"。
pub fn run_probe(
    probe_id: u64,
    config: &lt_proto::ModelConfig,
    bus: &Arc<SettingsBus>,
    cancel: Arc<AtomicBool>,
    msg: &Msg,
    sink: &EventSink,
) {
    run_probe_with_budget(
        probe_id,
        config,
        bus,
        cancel,
        msg,
        sink,
        PROBE_TOTAL_BUDGET,
    );
}

/// 带预算注入的探测实现（`run_probe` = 以契约常量调用它）。
///
/// 预算作为形参的唯一动机是**可测性**：10 秒的契约预算无法在单测里真等。
/// 对外行为面与 docs §4.3 完全一致。
pub(crate) fn run_probe_with_budget(
    probe_id: u64,
    config: &lt_proto::ModelConfig,
    bus: &Arc<SettingsBus>,
    cancel: Arc<AtomicBool>,
    msg: &Msg,
    sink: &EventSink,
    budget: Duration,
) {
    let name = config.name.clone();
    let t0 = Instant::now();
    let finish = |outcome: ProbeOutcome, step_note: Option<String>, preview: Option<String>| {
        sink.push(UiEvent::TestTranslatorResult {
            probe_id,
            name: name.clone(),
            outcome,
            ms: t0.elapsed().as_millis() as u64,
            step_note,
            preview,
        });
    };

    // 起步即被取消（用户点了中断又立刻重测/退出）
    if cancel.load(Ordering::Relaxed) {
        finish(ProbeOutcome::Cancelled, None, None);
        return;
    }

    let eff = bus.load();
    let params = translator_params(config, &eff);
    let step_timeout = eff.tl.timeout.clamp(1, PROBE_STEP_TIMEOUT_CAP);
    let target = eff.tl.target_language.clone();
    tracing::info!(
        "连接测试开始 #{probe_id}：{name}（{} / {}，单步超时 {step_timeout}s，总预算 {}s）",
        config.api_base,
        config.model,
        budget.as_secs()
    );

    let base = match lt_translate::Translator::new(params) {
        Ok(t) => t,
        Err(e) => {
            let detail = format!("{name}: {e:#}");
            tracing::warn!("连接测试 #{probe_id} 配置无法构建：{detail}");
            finish(
                ProbeOutcome::Failed {
                    kind: lt_proto::FailureKind::NotReady,
                    detail,
                },
                None,
                None,
            );
            return;
        }
    };

    // 起点形态与"体检驱动降级"许可：与 TlRig::from_effective 同一判据
    let start = lt_translate::first_step(base.thinking_plan());
    let explicit = config.disable_thinking
        && !config.thinking_unavailable
        && matches!(
            config.thinking_style.as_deref(),
            Some(s) if !s.is_empty() && s != "auto"
        );
    let allow_advance = config.disable_thinking && !config.thinking_unavailable && !explicit;

    let translator = base.with_cancel(cancel.clone());
    let ctl = RunCtl {
        cancel: Some(cancel.clone()),
        deadline: Some(t0 + budget),
    };
    let out = run_ladder(
        &translator,
        start,
        allow_advance,
        PROBE_TEXT,
        "auto",
        &target,
        step_timeout,
        &ctl,
        sink,
        0,
        0,
        false, // 探测不留幽灵"翻译中"消息
    );

    // ── 结论映射（顺序即优先级；docs §4.3 步骤 7）──
    // ① 取消：可能发生在两次尝试之间（halted），也可能发生在**一次尝试进行中**
    //    （流式读取返回 Cancelled，此时 halted 为 None）——两者都要如实回报
    let in_attempt_cancel = matches!(
        out.attempt.error,
        Some(lt_translate::TranslateError::Cancelled)
    );
    if out.halted == Some(Halt::Cancelled) || in_attempt_cancel {
        tracing::warn!(
            "连接测试 #{probe_id} 已中断（已尝试 {} 种请求形态，{} ms）",
            out.attempted,
            t0.elapsed().as_millis()
        );
        finish(ProbeOutcome::Cancelled, None, None);
        return;
    }
    // ② 成功（**先于预算判定**：恰好卡在预算线上成功的那次是真成功，
    //    不该被算成"未定论"）
    let text = out.attempt.text.clone().unwrap_or_default();
    if out.attempt.succeeded() && !text.trim().is_empty() {
        // 空白归一（换行/CRLF → 单空格、去首尾）后按**字符**截断：
        // 旧实现把 \n 与 \r 各自替换成空格，CRLF 会产出双空格
        let preview: String = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(PROBE_PREVIEW_MAX_CHARS)
            .collect();
        // 实际生效的形态：等于起点则不必说明；降级了要告知（"关不掉思考"的可见面）
        let step_note = if out.step == start {
            None
        } else if matches!(out.step, lt_translate::RequestStep::Minimal) {
            Some(msg.t("probe_step_minimal"))
        } else {
            Some(msg.t("probe_step_degraded"))
        };
        tracing::info!(
            "连接测试完成 #{probe_id}：成功（{} ms，台阶 {}）",
            t0.elapsed().as_millis(),
            lt_translate::step_name(out.step)
        );
        finish(ProbeOutcome::Ok, step_note, Some(preview));
        return;
    }

    // ③ 预算耗尽（不是失败）：阶梯被预算截停，**或**最后一次尝试本身吃满了
    //    预算（单次尝试超时按剩余预算向上取整，两者往往同时成立）
    if out.halted == Some(Halt::Budget) || Instant::now() >= t0 + budget {
        tracing::warn!(
            "连接测试 #{probe_id} 预算耗尽（{} 秒内未取得结论，已尝试 {} 种请求形态）",
            budget.as_secs(),
            out.attempted
        );
        finish(
            ProbeOutcome::Inconclusive {
                attempted: out.attempted,
            },
            None,
            None,
        );
        return;
    }

    // ④ 失败：分类 + 详情原文
    let (kind, detail) = match &out.attempt.error {
        Some(e) => (e.failure_kind(), e.ui_text()),
        None => {
            let kind = match out.attempt.verdict {
                Some(lt_translate::ResponseVerdict::EmptyTruncated) => {
                    lt_proto::FailureKind::Truncated
                }
                _ => lt_proto::FailureKind::Empty,
            };
            (kind, format!("verdict={:?}", out.attempt.verdict))
        }
    };
    tracing::warn!(
        "连接测试完成 #{probe_id}：失败（{} ms，台阶 {}，{}）",
        t0.elapsed().as_millis(),
        lt_translate::step_name(out.step),
        detail
    );
    finish(ProbeOutcome::Failed { kind, detail }, None, None);
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_artery::EventArtery;
    use lt_proto::{FailureKind, ModelConfig, Settings, UiEvent};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// 极简 mock 服务端：每次请求按处理器返回整段响应（原始 TCP，够用即止）
    type Handler = Arc<dyn Fn(&str) -> Vec<u8> + Send + Sync>;

    fn start_server(handler: Handler) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut stream) = conn else { break };
                let handler = handler.clone();
                std::thread::spawn(move || {
                    let mut data = Vec::new();
                    let mut buf = [0u8; 4096];
                    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                    while let Ok(n) = stream.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        data.extend_from_slice(&buf[..n]);
                        if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&data[..pos]).to_lowercase();
                            let len: usize = head
                                .lines()
                                .find_map(|l| l.strip_prefix("content-length:"))
                                .and_then(|v| v.trim().parse().ok())
                                .unwrap_or(0);
                            if data.len() >= pos + 4 + len {
                                break;
                            }
                        }
                    }
                    let resp = handler(&String::from_utf8_lossy(&data));
                    let _ = stream.write_all(&resp);
                    let _ = stream.flush();
                    std::thread::sleep(Duration::from_millis(50));
                });
            }
        });
        format!("http://127.0.0.1:{port}/v1")
    }

    /// 成功的流式响应（一个 delta + usage + [DONE]）
    fn sse_ok(content: &str) -> Vec<u8> {
        let delta = serde_json::json!({
            "id":"c","object":"chat.completion.chunk","created":1,"model":"m",
            "choices":[{"index":0,"delta":{"content":content},"finish_reason":null}]
        });
        let usage = serde_json::json!({
            "id":"c","object":"chat.completion.chunk","created":1,"model":"m",
            "choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}
        });
        let body = format!("data: {delta}\n\ndata: {usage}\n\ndata: [DONE]\n\n");
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    /// 空正文 + 推理量（体检 EmptyReasoningBudget：思考吃光预算的形态）
    fn sse_empty_reasoning() -> Vec<u8> {
        let usage = serde_json::json!({
            "id":"c","object":"chat.completion.chunk","created":1,"model":"m",
            "choices":[],
            "usage":{"prompt_tokens":10,"completion_tokens":50,"total_tokens":60,
                     "completion_tokens_details":{"reasoning_tokens":50}}
        });
        let body = format!("data: {usage}

data: [DONE]

");
        format!(
            "HTTP/1.1 200 OK
Content-Type: text/event-stream
Content-Length: {}
Connection: close

{body}",
            body.len()
        )
        .into_bytes()
    }

    /// 空正文 + finish_reason=length（体检 EmptyTruncated：触发"补发上限重试"）
    fn sse_empty_truncated() -> Vec<u8> {
        let chunk = serde_json::json!({
            "id":"c","object":"chat.completion.chunk","created":1,"model":"m",
            "choices":[{"index":0,"delta":{},"finish_reason":"length"}]
        });
        let body = format!("data: {chunk}\n\ndata: [DONE]\n\n");
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    /// 错误响应（状态码 + OpenAI 形状错误体）
    fn http_error(code: u16, message: &str) -> Vec<u8> {
        let line = match code {
            400 => "400 Bad Request",
            401 => "401 Unauthorized",
            _ => "500 Internal Server Error",
        };
        let body = serde_json::json!({"error": {"message": message}}).to_string();
        format!(
            "HTTP/1.1 {line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    fn test_config(base: &str) -> ModelConfig {
        ModelConfig {
            name: "probe-test".into(),
            api_base: base.into(),
            api_key: "k".into(),
            model: "m".into(),
            ..ModelConfig::default()
        }
    }

    fn test_bus() -> Arc<SettingsBus> {
        let s = Settings {
            timeout: 5,
            target_language: "zh".into(),
            ..Settings::default()
        };
        Arc::new(SettingsBus::new(s))
    }

    /// 跑一次探测并取回**全部**回流事件（含非回执事件——用于"恰好一次"断言）
    fn run_all(config: &ModelConfig, cancel: Arc<AtomicBool>, budget: Duration) -> Vec<UiEvent> {
        let sink = EventArtery::new();
        let msg = Msg::new(|k| k.to_string(), || "zh".into());
        let bus = test_bus();
        run_probe_with_budget(1, config, &bus, cancel, &msg, &sink, budget);
        let mut batch = Vec::new();
        let mut all = Vec::new();
        while sink.drain_batch(&mut batch, Duration::from_millis(30)) {
            all.append(&mut batch);
        }
        all
    }

    /// 取唯一回执（顺带钉住"恰好一条"——多一条少一条都算失败）
    fn run_with_budget(
        config: &ModelConfig,
        cancel: Arc<AtomicBool>,
        budget: Duration,
    ) -> UiEvent {
        let all = run_all(config, cancel, budget);
        let results: Vec<&UiEvent> = all
            .iter()
            .filter(|e| matches!(e, UiEvent::TestTranslatorResult { .. }))
            .collect();
        assert_eq!(results.len(), 1, "探测必须恰好回执一次，实际 {} 条", results.len());
        results[0].clone()
    }

    fn run(config: &ModelConfig, cancel: Arc<AtomicBool>) -> UiEvent {
        run_with_budget(config, cancel, PROBE_TOTAL_BUDGET)
    }

    fn outcome_of(ev: &UiEvent) -> (&ProbeOutcome, u64) {
        match ev {
            UiEvent::TestTranslatorResult {
                outcome,
                ms,
                probe_id,
                ..
            } => {
                assert_eq!(*probe_id, 1, "回执必须带回发号");
                (outcome, *ms)
            }
            other => panic!("期望 TestTranslatorResult，实际 {other:?}"),
        }
    }

    /// 打通：Ok + 回执摘要 + 起点形态即成功（无 step_note）
    #[test]
    fn probe_ok_reports_preview_and_ms() {
        let base = start_server(Arc::new(|_| sse_ok("你好，世界")));
        let ev = run(&test_config(&base), Arc::new(AtomicBool::new(false)));
        let (outcome, ms) = outcome_of(&ev);
        assert_eq!(outcome, &ProbeOutcome::Ok);
        match &ev {
            UiEvent::TestTranslatorResult {
                preview, step_note, ..
            } => {
                assert_eq!(preview.as_deref(), Some("你好，世界"));
                assert!(step_note.is_none(), "起点形态即成功不该报降级");
            }
            _ => unreachable!(),
        }
        assert!(ms < 5_000, "ms 应为真实耗时，实际 {ms}");
    }

    /// 首个形态"正文为空但推理吃满 token"（体检 EmptyReasoningBudget —— 真实世界的
    /// "关不掉思考"）→ 阶梯降级后成功 → step_note 非空。
    ///
    /// 不用 400 造降级：流式请求的"先带后撤"会把首个 400 吞掉重试一次
    /// （`pump_stream` 语义），一轮里未必能落到阶梯上。
    #[test]
    fn probe_ladder_reports_degraded_step() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = calls.clone();
        let base = start_server(Arc::new(move |_| {
            if c.fetch_add(1, Ordering::SeqCst) == 0 {
                sse_empty_reasoning()
            } else {
                sse_ok("降级后成功")
            }
        }));
        let ev = run(&test_config(&base), Arc::new(AtomicBool::new(false)));
        assert_eq!(outcome_of(&ev).0, &ProbeOutcome::Ok);
        match &ev {
            UiEvent::TestTranslatorResult { step_note, .. } => {
                assert_eq!(step_note.as_deref(), Some("probe_step_degraded"));
            }
            _ => unreachable!(),
        }
        assert!(calls.load(Ordering::SeqCst) >= 2, "应至少降级一次");
    }

    /// 401 → 失败分类为 Auth（中文文案由 UI 按 kind 取 i18n 键渲染）
    #[test]
    fn probe_reports_auth_failure_kind() {
        let base = start_server(Arc::new(|_| http_error(401, "invalid key")));
        let ev = run(&test_config(&base), Arc::new(AtomicBool::new(false)));
        match outcome_of(&ev).0.clone() {
            ProbeOutcome::Failed { kind, detail } => {
                assert_eq!(kind, FailureKind::Auth);
                assert!(detail.contains("401"), "详情应保留原始信息: {detail}");
            }
            other => panic!("期望 Failed，实际 {other:?}"),
        }
    }

    /// 装置无法构建（代理地址非法）→ NotReady，且**立即**回执（不白等网络）
    #[test]
    fn probe_reports_unbuildable_config_as_not_ready() {
        let mut cfg = test_config("http://127.0.0.1:1/v1");
        cfg.proxy = "http://[::1".into(); // 非法代理 URL → make_openai_client 报错
        let t0 = Instant::now();
        let ev = run(&cfg, Arc::new(AtomicBool::new(false)));
        match outcome_of(&ev).0.clone() {
            ProbeOutcome::Failed { kind, .. } => assert_eq!(kind, FailureKind::NotReady),
            other => panic!("期望 Failed(NotReady)，实际 {other:?}"),
        }
        assert!(t0.elapsed() < Duration::from_secs(2), "不得白等网络");
    }

    /// 总预算耗尽 → Inconclusive（**不是失败**）
    #[test]
    fn probe_budget_exhausted_is_inconclusive() {
        // 服务端读走请求后长时间不回包
        let base = start_server(Arc::new(|_| {
            std::thread::sleep(Duration::from_secs(5));
            sse_ok("迟到")
        }));
        let ev = run_with_budget(
            &test_config(&base),
            Arc::new(AtomicBool::new(false)),
            Duration::from_millis(200),
        );
        match outcome_of(&ev).0.clone() {
            ProbeOutcome::Inconclusive { attempted } => {
                assert!(attempted >= 1, "应如实报告已尝试的形态数");
            }
            other => panic!("期望 Inconclusive，实际 {other:?}"),
        }
    }

    /// 起始即被取消 → Cancelled（不触网、不误报失败）
    #[test]
    fn probe_cancel_before_start_is_reported() {
        let flag = Arc::new(AtomicBool::new(true));
        let t0 = Instant::now();
        let ev = run(&test_config("http://127.0.0.1:1/v1"), flag);
        assert_eq!(outcome_of(&ev).0, &ProbeOutcome::Cancelled);
        assert!(t0.elapsed() < Duration::from_millis(500));
    }

    /// **一次尝试进行中**被取消：`halted` 为 None 而错误是 Cancelled——
    /// 映射表首行必须识别它（漏了会显示成"连接失败·已中断"）
    #[test]
    fn probe_cancel_during_attempt_is_reported_as_cancelled() {
        // 服务端读走请求后长时间不回包 → 消费端停在等待中
        let base = start_server(Arc::new(|_| {
            std::thread::sleep(Duration::from_secs(5));
            sse_ok("迟到")
        }));
        let flag = Arc::new(AtomicBool::new(false));
        let f2 = flag.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            f2.store(true, Ordering::SeqCst);
        });
        let t0 = Instant::now();
        let ev = run(&test_config(&base), flag);
        assert_eq!(outcome_of(&ev).0, &ProbeOutcome::Cancelled);
        assert!(
            t0.elapsed() < Duration::from_secs(3),
            "取消应立即收口，实际 {:?}",
            t0.elapsed()
        );
    }

    /// D-85 评审修复：截断补发（同轮内的第二次尝试）**吃剩余预算**，不是裸单步超时。
    /// 服务端首个请求返回"空+截断"、第二个请求永不回包：探测必须在 ~1 秒预算内
    /// 收口为 Inconclusive，且 attempted = 2（补发那次也计数）。
    /// 旧实现在此会以裸 timeout（5s）跑第二次请求 → 探测突破 10 秒封顶、
    /// attempted 少报 1。
    #[test]
    fn probe_truncation_retry_respects_remaining_budget() {
        use std::sync::atomic::AtomicU64;
        let hits = Arc::new(AtomicU64::new(0));
        let h = hits.clone();
        let base = start_server(Arc::new(move |_| {
            if h.fetch_add(1, Ordering::SeqCst) == 0 {
                sse_empty_truncated()
            } else {
                // 补发请求永不回包：只有"按剩余预算钳制"才能在预算内收口
                std::thread::sleep(Duration::from_secs(30));
                sse_empty_truncated()
            }
        }));
        let t0 = Instant::now();
        let ev = run_with_budget(
            &test_config(&base),
            Arc::new(AtomicBool::new(false)),
            Duration::from_secs(1),
        );
        assert_eq!(hits.load(Ordering::SeqCst), 2, "截断补发应真的发起一次");
        match outcome_of(&ev).0.clone() {
            ProbeOutcome::Inconclusive { attempted } => {
                assert_eq!(attempted, 2, "补发那次也要计入 attempted（方案 §4.4 计数口径）");
            }
            other => panic!("期望 Inconclusive，实际 {other:?}"),
        }
        assert!(
            t0.elapsed() < Duration::from_secs(3),
            "补发必须吃剩余预算（≤~1s），不得用满单步超时，实际 {:?}",
            t0.elapsed()
        );
    }

    /// 回归：预算充足时截断补发仍然照常工作（修 halt 检查不得把补发功能关掉）
    #[test]
    fn probe_truncation_retry_still_succeeds_within_budget() {
        use std::sync::atomic::AtomicU64;
        let hits = Arc::new(AtomicU64::new(0));
        let h = hits.clone();
        let base = start_server(Arc::new(move |_| {
            if h.fetch_add(1, Ordering::SeqCst) == 0 {
                sse_empty_truncated()
            } else {
                sse_ok("补发成功")
            }
        }));
        let ev = run(&test_config(&base), Arc::new(AtomicBool::new(false)));
        match outcome_of(&ev).0 {
            ProbeOutcome::Ok => {}
            other => panic!("期望 Ok，实际 {other:?}"),
        }
        assert_eq!(hits.load(Ordering::SeqCst), 2, "补发应发生且成功");
        match &ev {
            UiEvent::TestTranslatorResult { preview, .. } => {
                assert_eq!(preview.as_deref(), Some("补发成功"));
            }
            _ => unreachable!(),
        }
    }

    /// 探测只读（不接收 learned/degraded/transcript——签名即保证）：
    /// 可观测面只剩"恰好回执一次"
    #[test]
    fn probe_emits_exactly_one_receipt() {
        let base = start_server(Arc::new(|_| sse_ok("x")));
        let all = run_all(
            &test_config(&base),
            Arc::new(AtomicBool::new(false)),
            PROBE_TOTAL_BUDGET,
        );
        let n = all
            .iter()
            .filter(|e| matches!(e, UiEvent::TestTranslatorResult { .. }))
            .count();
        assert_eq!(n, 1, "探测必须恰好回执一次（不多不少），全部事件：{all:?}");
    }
}
