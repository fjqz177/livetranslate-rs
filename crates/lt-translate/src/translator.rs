//! Translator（对照原版 translator.py 1:1）。
//!
//! 线程模型：原版用同步 openai 客户端跑在翻译线程池上；Rust 侧 async-openai
//! 跑在 crate 内共享 tokio 运行时上，对外暴露阻塞 API（消费方都是普通线程）。
//!
//! 超时语义（对应 httpx.Timeout(t, connect=5.0)）：
//! - 连接超时 5s 固定（client 级）；
//! - 流式：每 chunk 读超时 t（pump 内 tokio timeout）+ 总 deadline t（消费侧
//!   单调钟检查，等价原版 `if time.monotonic() > deadline`）；
//! - 非流式：整体 tokio timeout t（原版为每 socket 读超时 t；单读场景等价，
//!   已知实现级偏差）。

use std::collections::BTreeMap;
use std::sync::{mpsc, Arc, LazyLock};
use std::time::{Duration, Instant};

use async_openai::config::OpenAIConfig;
use async_openai::types::chat::{CreateChatCompletionResponse, CreateChatCompletionStreamResponse};
use parking_lot::Mutex;
use serde_json::{json, Map, Value};

use crate::error::TranslateError;
use crate::thinking::{resolve_thinking_plan, thinking_disable_body, ThinkingPlan};
// E3/ADR-10：提示词/覆写键上移 lt-proto 契约层（与 lt-ui 同源——原 UI 依赖
// 整个本 crate 仅为取常量的边已裁除）
use lt_proto::{DEFAULT_PROMPT, OVERRIDE_KEYS};

/// crate 内共享 tokio 运行时（所有 Translator 复用；worker=2，纯 I/O 负载）
static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime 初始化失败")
});

pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    &RUNTIME
}

/// 语言代码 → 英文显示名（对照原版 LANGUAGE_DISPLAY 全表）
static LANGUAGE_DISPLAY: &[(&str, &str)] = &[
    ("en", "English"),
    ("ja", "Japanese"),
    ("zh", "Chinese"),
    ("ko", "Korean"),
    ("fr", "French"),
    ("de", "German"),
    ("es", "Spanish"),
    ("ru", "Russian"),
    ("pt", "Portuguese"),
    ("it", "Italian"),
    ("nl", "Dutch"),
    ("pl", "Polish"),
    ("tr", "Turkish"),
    ("ar", "Arabic"),
    ("th", "Thai"),
    ("vi", "Vietnamese"),
    ("id", "Indonesian"),
    ("ms", "Malay"),
    ("hi", "Hindi"),
    ("uk", "Ukrainian"),
    ("cs", "Czech"),
    ("ro", "Romanian"),
    ("el", "Greek"),
    ("hu", "Hungarian"),
    ("sv", "Swedish"),
    ("da", "Danish"),
    ("fi", "Finnish"),
    ("no", "Norwegian"),
    ("he", "Hebrew"),
];

pub(crate) fn lang_display(code: &str) -> &str {
    LANGUAGE_DISPLAY
        .iter()
        .find(|(k, _)| *k == code)
        .map_or(code, |(_, v)| v)
}

/// Translator 构造参数（默认值 = 原版签名默认值）
#[derive(Debug, Clone)]
pub struct TranslatorParams {
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    /// W1（方案 §2.1）：None = **不发送**该参数（长度上限交给服务端默认）
    pub max_tokens: Option<u32>,
    /// W1（方案 §2.1）：None = **不发送**该参数
    pub temperature: Option<f64>,
    pub streaming: bool, // true
    pub system_prompt: Option<String>,
    pub proxy: String, // "none" | "system" | URL
    pub no_system_role: bool,
    /// W1（方案 §2.3）：关闭模型思考总开关，默认 true
    pub disable_thinking: bool,
    /// 关闭方式（方案 §2.3）：None/"auto" 自动首选；deepseek/qwen/vllm/openai 只用该一种
    pub thinking_style: Option<String>,
    pub json_response: bool,
    pub overrides: Option<BTreeMap<String, Value>>,
    pub extra_body: Option<Value>,
}

impl Default for TranslatorParams {
    fn default() -> Self {
        Self {
            api_base: "http://127.0.0.1:1234/v1".into(),
            api_key: String::new(),
            model: String::new(),
            max_tokens: None,
            temperature: Some(lt_proto::DEFAULT_TEMPERATURE),
            streaming: true,
            system_prompt: None,
            proxy: "none".into(),
            no_system_role: false,
            disable_thinking: true,
            thinking_style: None,
            json_response: false,
            overrides: None,
            extra_body: None,
        }
    }
}

/// 会话态（W4：目标语言/超时不再是模型运行时状态——构架 2.0 §3.2.3，
/// 设置总线派生视图，**提交翻译前读 `load().tl`**，逐调用参数传入；
/// 以下全部为会话/用量态，跨调用存活，由实例独占）
struct MutableState {
    context_turns: u32,
    history: Vec<(String, String)>,
    prompt_tokens: u64,
    completion_tokens: u64,
}

/// LLM 翻译器（OpenAI 兼容 API）。
///
/// `&self` 多读并发安全（原版各 setter 也是随时可调）；可变状态在 Mutex 内，
/// 绝不持锁跨 await。
pub struct Translator {
    client: async_openai::Client<OpenAIConfig>,
    model: String,
    streaming: bool,
    json_response: bool,
    /// W1（方案 §2.3）：本次装置实际要发送的关闭形态（构造期一次性解析）
    thinking: ThinkingPlan,
    no_system_role: bool,
    max_tokens: Option<u32>,
    temperature: Option<f64>,
    overrides: BTreeMap<String, Value>,
    extra_body: Value,
    system_prompt_template: String,
    state: Arc<Mutex<MutableState>>,
}

impl Translator {
    pub fn new(params: TranslatorParams) -> Result<Self, TranslateError> {
        let client = make_openai_client(&params.api_base, &params.api_key, &params.proxy)?;
        // W1（方案 §2.3）：总开关 + 方式 → 本次装置的唯一关闭形态
        let thinking = resolve_thinking_plan(
            params.disable_thinking,
            params.thinking_style.as_deref(),
            &params.api_base,
            &params.model,
        );
        if thinking != ThinkingPlan::None {
            tracing::info!(
                "Translator: thinking disabled for {} via {} ({})",
                params.model,
                thinking.name(),
                thinking_disable_body(thinking)
            );
        }
        if params.json_response {
            tracing::info!("Translator: json_response enabled for {}", params.model);
        }
        // 值为 null 的 override 键剔除（原版 `if v is not None`）
        let overrides: BTreeMap<String, Value> = params
            .overrides
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, v)| !v.is_null())
            .collect();
        let extra_body = match params.extra_body {
            Some(Value::Object(m)) => Value::Object(m),
            Some(other) => {
                tracing::warn!("Translator: extra_body 非 JSON object，忽略: {other}");
                Value::Object(Map::new())
            }
            None => Value::Object(Map::new()),
        };
        if !overrides.is_empty() {
            let ov: Map<String, Value> = overrides.clone().into_iter().collect();
            tracing::info!("Translator overrides: {}", serde_json::Value::Object(ov));
        }
        if !extra_body.as_object().unwrap().is_empty() {
            tracing::info!("Translator extra_body: {extra_body}");
        }
        Ok(Self {
            client,
            model: params.model,
            streaming: params.streaming,
            json_response: params.json_response,
            thinking,
            no_system_role: params.no_system_role,
            max_tokens: params.max_tokens,
            temperature: params.temperature,
            overrides,
            extra_body,
            system_prompt_template: params
                .system_prompt
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_PROMPT.to_string()),
            state: Arc::new(Mutex::new(MutableState {
                context_turns: 0,
                history: Vec::new(),
                prompt_tokens: 0,
                completion_tokens: 0,
            })),
        })
    }

    /// 最近一次 translate 的 (prompt_tokens, completion_tokens)
    pub fn last_usage(&self) -> (u64, u64) {
        let st = self.state.lock();
        (st.prompt_tokens, st.completion_tokens)
    }

    pub fn set_context_turns(&self, n: u32) {
        let mut st = self.state.lock();
        st.context_turns = n;
        if n == 0 {
            st.history.clear();
        }
    }

    pub fn clear_history(&self) {
        self.state.lock().history.clear();
    }

    /// 共享同一 client 的新 Translator（历史/用量清零；目标语言/超时随每次
    /// 调用参数传入，W4——旧 with_target_language 的目标语言面随镜像退役）
    pub fn share_client(&self) -> Translator {
        Translator {
            client: self.client.clone(),
            model: self.model.clone(),
            streaming: self.streaming,
            json_response: self.json_response,
            thinking: self.thinking,
            no_system_role: self.no_system_role,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            overrides: self.overrides.clone(),
            extra_body: self.extra_body.clone(),
            system_prompt_template: self.system_prompt_template.clone(),
            state: Arc::new(Mutex::new(MutableState {
                context_turns: 0,
                history: Vec::new(),
                prompt_tokens: 0,
                completion_tokens: 0,
            })),
        }
    }

    // ── prompt / messages 组装 ──

    fn format_context(&self, context_turns: u32, history: &[(String, String)]) -> String {
        if context_turns == 0 || history.is_empty() {
            return String::new();
        }
        let take = (context_turns as usize).min(history.len());
        let mut out = String::new();
        for (src, tgt) in &history[history.len() - take..] {
            out.push_str(&format!("Source: {src}\nTranslation: {tgt}\n\n"));
        }
        // rstrip
        out.trim_end().to_string()
    }

    fn build_system_prompt(&self, source_lang: &str, target_lang: &str) -> String {
        let src = lang_display(source_lang);
        let tgt = lang_display(target_lang);
        let st = self.state.lock();
        let context = self.format_context(st.context_turns, &st.history);
        let prompt = match format_prompt_template(&self.system_prompt_template, src, tgt, &context)
        {
            Some(p) => p,
            None => {
                tracing::warn!("Bad prompt template, falling back to default");
                format_prompt_template(DEFAULT_PROMPT, src, tgt, "")
                    .expect("DEFAULT_PROMPT 占位符固定合法")
            }
        };
        let mut prompt = prompt;
        if self.json_response {
            prompt.push_str("\nRespond in JSON format: {\"t\": \"translated text\"}");
        }
        prompt
    }

    fn build_messages(&self, system_prompt: &str, text: &str) -> Value {
        if self.no_system_role {
            return json!([{ "role": "user", "content": format!("{system_prompt}\n{text}") }]);
        }
        let mut msgs = vec![json!({"role": "system", "content": system_prompt})];
        let st = self.state.lock();
        // 模板含 {context} 时上下文已并入 system prompt，不再追加历史消息
        if st.context_turns > 0
            && !st.history.is_empty()
            && !self.system_prompt_template.contains("{context}")
        {
            let take = (st.context_turns as usize).min(st.history.len());
            for (src, tgt) in &st.history[st.history.len() - take..] {
                msgs.push(json!({"role": "user", "content": src}));
                msgs.push(json!({"role": "assistant", "content": tgt}));
            }
        }
        drop(st);
        msgs.push(json!({"role": "user", "content": text}));
        json!(msgs)
    }

    fn append_history(&self, text: &str, result: &str) {
        append_history_locked(&mut self.state.lock(), text, result);
    }

    // ── 请求体组装（byot：直接产 JSON，等价原版 _build_request_kwargs） ──

    /// 合并后的 extra 请求体（自动 thinking 关闭体 + 用户 extra_body，后者覆盖
    /// 同名键）；为空时 Null（= 不发）。镜像原版 kwargs["extra_body"]，供测试观察。
    pub(crate) fn merged_extra_body(&self) -> Value {
        let mut extra = Map::new();
        if let Value::Object(m) = thinking_disable_body(self.thinking) {
            extra.extend(m);
        }
        if let Value::Object(m) = &self.extra_body {
            extra.extend(m.clone());
        }
        if extra.is_empty() {
            Value::Null
        } else {
            Value::Object(extra)
        }
    }

    /// 组装请求体（byot JSON；`include_usage` 仅流式首次尝试携带）。
    /// 公开供测试快照与调试使用。
    pub fn build_request_body(
        &self,
        system_prompt: &str,
        text: &str,
        stream: bool,
        include_usage: bool,
    ) -> Value {
        let mut body = Map::new();
        body.insert("model".into(), json!(self.model));
        body.insert("messages".into(), self.build_messages(system_prompt, text));
        // W1（方案 §2.1）：长度上限与温度皆可缺席——None 时不发送该键
        // （长度上限交给服务端默认：应用强加上限会把"先想再答"的模型憋死）
        if let Some(max_tokens) = self.max_tokens {
            body.insert("max_tokens".into(), json!(max_tokens));
        }
        if let Some(temperature) = self.temperature {
            body.insert("temperature".into(), json!(temperature));
        }
        for key in OVERRIDE_KEYS {
            if let Some(v) = self.overrides.get(key) {
                body.insert(key.into(), v.clone());
            }
        }
        // extra_body：自动 thinking 关闭体在前，用户 extra_body 覆盖同名键
        if let Value::Object(m) = self.merged_extra_body() {
            for (k, v) in m {
                body.insert(k, v);
            }
        }
        if self.json_response {
            body.insert(
                "response_format".into(),
                json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": "translation",
                        "strict": true,
                        "schema": {
                            "type": "object",
                            "properties": {"t": {"type": "string"}},
                            "required": ["t"],
                            "additionalProperties": false,
                        },
                    },
                }),
            );
        }
        if stream {
            body.insert("stream".into(), json!(true));
            if include_usage {
                body.insert("stream_options".into(), json!({"include_usage": true}));
            }
        }
        Value::Object(body)
    }

    // ── 同步（非流式） ──

    fn translate_sync(
        &self,
        system_prompt: &str,
        text: &str,
        timeout_secs: u64,
    ) -> Result<String, TranslateError> {
        let body = self.build_request_body(system_prompt, text, false, false);
        let resp: CreateChatCompletionResponse =
            self.timeout_block(self.client.chat().create_byot(body), timeout_secs)?;
        {
            let mut st = self.state.lock();
            st.prompt_tokens = 0;
            st.completion_tokens = 0;
            if let Some(usage) = &resp.usage {
                st.prompt_tokens = usage.prompt_tokens as u64;
                st.completion_tokens = usage.completion_tokens as u64;
            }
        }
        let mut result = resp
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default()
            .trim()
            .to_string();
        if self.json_response {
            result = extract_json_translation(&result);
        }
        warn_if_thinking_burned(&result, self.last_usage().1, self.thinking.name());
        Ok(result)
    }

    /// 在共享运行时上执行带超时的 future（阻塞调用线程）
    /// W4：超时逐调用传入（设置总线 `tl.timeout` 生效值；不再存实例状态）
    fn timeout_block<F, T>(&self, fut: F, timeout_secs: u64) -> Result<T, TranslateError>
    where
        F: std::future::Future<Output = Result<T, async_openai::error::OpenAIError>>,
    {
        // timeout(...) 必须在 runtime 上下文内求值（Sleep 需要 timer 句柄）
        match runtime().block_on(async {
            tokio::time::timeout(Duration::from_secs(timeout_secs), fut).await
        }) {
            Ok(inner) => inner.map_err(TranslateError::from),
            Err(_) => Err(TranslateError::Timeout(format!(
                "Translation exceeded {timeout_secs}s total timeout"
            ))),
        }
    }

    // ── 对外主入口 ──

    /// 翻译并返回完整结果（原版 translate：按 streaming 配置走流式或同步）。
    /// W4：`target_lang`/`timeout_secs` 为调用方从设置总线读出的生效值（TlView）。
    pub fn translate(
        &self,
        text: &str,
        source_language: &str,
        target_lang: &str,
        timeout_secs: u32,
    ) -> Result<String, TranslateError> {
        let mut last: Option<Result<String, TranslateError>> = None;
        for item in self.translate_iter(text, source_language, target_lang, timeout_secs) {
            last = Some(Ok(item?));
        }
        // translate_iter 必产至少一个值
        last.unwrap_or_else(|| Err(TranslateError::Other("no translation produced".into())))
    }

    /// 流式翻译迭代器：流式模式下产出累积部分文本，最后一个值是完整译文；
    /// 非流式/json 模式只产出最终值。中途错误以 Err 项出现（此后迭代结束）。
    /// 消费方应迭代到底并以最后一个 Ok 作为最终结果（与原版生成器语义一致）。
    /// W4：`target_lang`/`timeout_secs` 逐调用传入（原版 set_target_language/
    /// set_timeout 的运行时可变面已退役——目标语言/超时来自设置总线）。
    pub fn translate_iter(
        &self,
        text: &str,
        source_language: &str,
        target_lang: &str,
        timeout_secs: u32,
    ) -> TranslateStream {
        let system_prompt = self.build_system_prompt(source_language, target_lang);
        if !self.streaming {
            let result = self
                .translate_sync(&system_prompt, text, timeout_secs as u64)
                .inspect(|r| {
                    self.append_history(text, r);
                });
            return TranslateStream::sync(result);
        }

        let body = self.build_request_body(&system_prompt, text, true, false);
        let body_with_usage = {
            let mut m = body.as_object().expect("body 是 object").clone();
            m.insert("stream_options".into(), json!({"include_usage": true}));
            Value::Object(m)
        };
        let read_timeout = Duration::from_secs(timeout_secs as u64);
        let (tx, rx) = mpsc::channel();
        let client = self.client.clone();
        runtime().spawn(async move {
            pump_stream(client, body_with_usage, body, read_timeout, tx).await;
        });
        let deadline = Instant::now() + read_timeout;
        TranslateStream {
            inner: StreamInner::Streaming {
                rx,
                deadline,
                acc: String::new(),
                text: text.to_string(),
                timeout_secs: read_timeout.as_secs(),
            },
            json_response: self.json_response,
            thinking: self.thinking,
            state: self.state.clone(),
            finished: false,
        }
    }
}

/// "先带后撤"流式请求：先带 stream_options.include_usage，服务端拒绝则不带重试
/// （原版 try/except 语义，两次失败才向调用方报错）。
async fn pump_stream(
    client: async_openai::Client<OpenAIConfig>,
    with_usage: Value,
    plain: Value,
    read_timeout: Duration,
    tx: mpsc::Sender<StreamMsg>,
) {
    use futures::StreamExt;

    let mut stream = match client
        .chat()
        .create_stream_byot::<Value, CreateChatCompletionStreamResponse>(with_usage)
        .await
    {
        Ok(s) => s,
        Err(_) => match client
            .chat()
            .create_stream_byot::<Value, CreateChatCompletionStreamResponse>(plain)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(StreamMsg::Err(TranslateError::from(e)));
                return;
            }
        },
    };

    let (mut pt, mut ct) = (0u64, 0u64);
    loop {
        match tokio::time::timeout(read_timeout, stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                if let Some(usage) = &chunk.usage {
                    pt = usage.prompt_tokens as u64;
                    ct = usage.completion_tokens as u64;
                }
                if let Some(delta) = chunk
                    .choices
                    .first()
                    .and_then(|c| c.delta.content.as_deref())
                    .filter(|s| !s.is_empty())
                {
                    if tx.send(StreamMsg::Delta(delta.to_string())).is_err() {
                        // 消费方已放弃（超时/提前终止），停止拉流
                        return;
                    }
                }
            }
            Ok(Some(Err(e))) => {
                let _ = tx.send(StreamMsg::Err(TranslateError::from(e)));
                return;
            }
            Ok(None) => break,
            Err(_) => {
                let _ = tx.send(StreamMsg::Err(TranslateError::Timeout(format!(
                    "stream read timed out after {}s",
                    read_timeout.as_secs()
                ))));
                return;
            }
        }
    }
    let _ = tx.send(StreamMsg::End {
        prompt_tokens: pt,
        completion_tokens: ct,
    });
}

enum StreamMsg {
    Delta(String),
    End {
        prompt_tokens: u64,
        completion_tokens: u64,
    },
    Err(TranslateError),
}

enum StreamInner {
    Sync(Option<Result<String, TranslateError>>),
    Streaming {
        rx: mpsc::Receiver<StreamMsg>,
        deadline: Instant,
        acc: String,
        text: String,
        timeout_secs: u64,
    },
}

/// translate_iter 的消费端迭代器
pub struct TranslateStream {
    inner: StreamInner,
    json_response: bool,
    /// 本次装置的关闭形态（仅用于告警文案；W2 起由体检结论取代）
    thinking: ThinkingPlan,
    state: Arc<Mutex<MutableState>>,
    finished: bool,
}

impl TranslateStream {
    fn sync(result: Result<String, TranslateError>) -> Self {
        Self {
            inner: StreamInner::Sync(Some(result)),
            json_response: false,
            thinking: ThinkingPlan::None,
            state: Arc::new(Mutex::new(MutableState {
                context_turns: 0,
                history: Vec::new(),
                prompt_tokens: 0,
                completion_tokens: 0,
            })),
            finished: false,
        }
    }
}

impl Iterator for TranslateStream {
    type Item = Result<String, TranslateError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        match &mut self.inner {
            StreamInner::Sync(item) => {
                self.finished = true;
                item.take()
            }
            StreamInner::Streaming {
                rx,
                deadline,
                acc,
                text,
                timeout_secs,
            } => loop {
                let now = Instant::now();
                let timeout_msg = format!("Translation exceeded {timeout_secs}s total timeout");
                let Some(remaining) = deadline.checked_duration_since(now) else {
                    self.finished = true;
                    return Some(Err(TranslateError::Timeout(timeout_msg)));
                };
                match rx.recv_timeout(remaining) {
                    Ok(StreamMsg::Delta(d)) => {
                        acc.push_str(&d);
                        if self.json_response {
                            // json 模式中途不出部分结果，继续消费直到流结束
                            continue;
                        }
                        return Some(Ok(acc.clone()));
                    }
                    Ok(StreamMsg::End {
                        prompt_tokens,
                        completion_tokens,
                    }) => {
                        {
                            let mut st = self.state.lock();
                            st.prompt_tokens = prompt_tokens;
                            st.completion_tokens = completion_tokens;
                        }
                        let mut result = acc.trim().to_string();
                        if self.json_response {
                            result = extract_json_translation(&result);
                        }
                        warn_if_thinking_burned(&result, completion_tokens, self.thinking.name());
                        if check_repetition(&result) {
                            self.finished = true;
                            return Some(Err(TranslateError::Repetition(result)));
                        }
                        let text = std::mem::take(text);
                        append_history_locked(&mut self.state.lock(), &text, &result);
                        self.finished = true;
                        return Some(Ok(result));
                    }
                    Ok(StreamMsg::Err(e)) => {
                        self.finished = true;
                        return Some(Err(e));
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        self.finished = true;
                        return Some(Err(TranslateError::Timeout(timeout_msg)));
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        // pump 保证先发 End/Err 再退出，此分支仅防御
                        self.finished = true;
                        return None;
                    }
                }
            },
        }
    }
}

/// 历史追加与裁剪（原版 _append_history：context_turns>0 且结果非空才记；
/// 超过 context_turns+2 条裁到最近 context_turns 条）
fn append_history_locked(st: &mut MutableState, text: &str, result: &str) {
    if st.context_turns > 0 && !result.is_empty() {
        st.history.push((text.to_string(), result.to_string()));
        let max_keep = st.context_turns as usize + 2;
        if st.history.len() > max_keep {
            let keep = st.context_turns as usize;
            let start = st.history.len() - keep;
            st.history = st.history.split_off(start);
        }
    }
}

/// 从 JSON 响应中提取译文（原版 _extract_json_translation：解析失败回退原文）
pub(crate) fn extract_json_translation(raw: &str) -> String {
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(raw) {
        if let Some(t) = map.get("t") {
            if let Some(s) = t.as_str() {
                return s.to_string();
            }
        }
    }
    raw.to_string()
}

fn warn_if_thinking_burned(result: &str, completion_tokens: u64, plan: &str) {
    if result.is_empty() && completion_tokens > 0 {
        tracing::warn!(
            "Empty translation but {completion_tokens} completion tokens were used - the model \
             likely spent the whole output budget on reasoning; keep 「关闭模型思考」enabled in \
             the model editor (current plan: {plan})"
        );
    }
}

/// 检测模型输出中的重复循环（原版 _check_repetition：字符级，与 Python 切片语义一致）
pub fn check_repetition(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    if len < 40 {
        return false;
    }
    for plen in 8..=(len / 2) {
        if chars[plen..plen * 2] == chars[..plen] {
            return true;
        }
    }
    false
}

/// 构造 OpenAI 客户端（对照原版 make_openai_client：代理三模式 + 连接超时 5s）。
/// 不设总超时——读超时在请求层按原版 httpx 语义逐 chunk 实施。
pub fn make_openai_client(
    api_base: &str,
    api_key: &str,
    proxy: &str,
) -> Result<async_openai::Client<OpenAIConfig>, TranslateError> {
    let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(5));
    match proxy {
        "system" => {}
        "none" | "" => builder = builder.no_proxy(),
        url => {
            builder = builder.proxy(
                reqwest::Proxy::all(url)
                    .map_err(|e| TranslateError::Other(format!("invalid proxy: {e}")))?,
            )
        }
    }
    let http = builder
        .build()
        .map_err(|e| TranslateError::Other(format!("http client build failed: {e}")))?;
    let config = OpenAIConfig::new()
        .with_api_base(api_base)
        .with_api_key(api_key);
    Ok(async_openai::Client::build(http, config))
}

/// 迷你 str.format：支持 {{ }} 转义与 {source_lang}/{target_lang}/{context} 命名占位。
/// 出现位置参数、未知名字或格式规格（`:`）→ None（等价原版捕获
/// KeyError/IndexError/ValueError 后回退默认模板）。
pub(crate) fn format_prompt_template(
    tmpl: &str,
    source: &str,
    target: &str,
    context: &str,
) -> Option<String> {
    let mut out = String::with_capacity(tmpl.len());
    let mut chars = tmpl.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '{' {
            if c == '}' {
                // 孤立 '}'：Python 中 `}}` 是转义；单个 } 在无未配对 { 时报 ValueError。
                if chars.peek() == Some(&'}') {
                    chars.next();
                    out.push('}');
                    continue;
                }
                return None;
            }
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('{') => out.push('{'),
            Some(first) => {
                let mut field = String::new();
                let mut cur = first;
                loop {
                    if cur == '}' {
                        break;
                    }
                    if cur == ':' || cur == '!' {
                        // 格式规格/转换：不支持 → 视为坏模板（保守回退）
                        return None;
                    }
                    field.push(cur);
                    let Some(n) = chars.next() else {
                        return None; // 未闭合
                    };
                    cur = n;
                }
                match field.as_str() {
                    "source_lang" => out.push_str(source),
                    "target_lang" => out.push_str(target),
                    "context" => out.push_str(context),
                    _ => return None,
                }
            }
            None => return None, // 尾部孤立 '{'
        }
    }
    Some(out)
}

#[cfg(test)]
impl Translator {
    /// 测试构造：不触网（client 构建不发起连接）
    pub(crate) fn for_test(
        api_base: &str,
        model: &str,
        disable_thinking: bool,
        thinking_style: Option<&str>,
        extra_body: Option<Value>,
    ) -> Self {
        Self::new(TranslatorParams {
            api_base: api_base.into(),
            api_key: "test-key".into(),
            model: model.into(),
            disable_thinking,
            thinking_style: thinking_style.map(str::to_string),
            extra_body,
            ..TranslatorParams::default()
        })
        .expect("测试客户端构建必成功")
    }
}
