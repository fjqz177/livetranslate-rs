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
use parking_lot::Mutex;
use serde_json::{json, Map, Value};

use crate::error::TranslateError;
use crate::reasoning::{strip_reasoning, ReasoningStripper};
use crate::thinking::{resolve_thinking_plan, thinking_disable_body, ThinkingPlan};
use crate::verdict::{classify_response, finish_kind, FinishKind, ResponseVerdict};
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

/// 已撤出界面、**不再参与请求**的覆写键（第二轮评审 ②）：温度已升为一等字段、
/// 输出上限应用不再发送（会憋死思考模型）、seed 冷门——界面上看不见的键绝不允许
/// 影响实际请求（"界面所见 = 实际所发"）。档案里的旧值由 `Settings::sanitize`
/// 一次性迁移清除，此处是构造期的第二道闸。
const OVERRIDE_KEYS_HIDDEN: [&str; 3] = ["temperature", "max_tokens", "seed"];

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
    /// 采样温度（2026-09-10 裁决）：None = **不发送**——默认即 None。
    /// 高级参数默认一律不发送，只有用户在界面上手动指定才发。
    pub temperature: Option<f64>,
    pub streaming: bool, // true
    pub system_prompt: Option<String>,
    pub proxy: String, // "none" | "system" | URL
    pub no_system_role: bool,
    /// W1（方案 §2.3）：关闭模型思考总开关，默认 true
    pub disable_thinking: bool,
    /// 第二轮评审 item 5：该模型已确认**无法关闭思维链**（持久化在模型配置里）
    /// ——true 时不再尝试注入任何关闭参数，直接走"照常思考 + 隔离器兜底"
    pub thinking_unavailable: bool,
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
            temperature: None,
            streaming: true,
            system_prompt: None,
            proxy: "none".into(),
            no_system_role: false,
            disable_thinking: true,
            thinking_unavailable: false,
            thinking_style: None,
            json_response: false,
            overrides: None,
            extra_body: None,
        }
    }
}

/// 会话态（W4：目标语言/超时不再是模型运行时状态——构架 2.0 §3.2.3，
/// 设置总线派生视图，**提交翻译前读 `load().tl`**，逐调用参数传入；
/// 以下为会话记忆，跨调用存活，由实例与它的派生副本**共享**）
///
/// 2026-09-10 修正（第二轮评审 ①）：`Arc<Mutex<MutableState>>` 的共享是**特性
/// 不是缺陷**——W3 为"并发不串台"给每次提交派生空白状态的副本，代价是
/// 「上下文数」整体失效（历史恒空、`context_turns` 恒 0）。用量账本早已改为
/// 随迭代器返回（INV-B），因此这里只留**确实应当跨调用共享**的上下文记忆。
struct MutableState {
    context_turns: u32,
    /// 已完成的句子（提交序号 → 原文/译文）。用序号索引的原因：8 个 worker
    /// 并发时入史顺序 = **完成序**，Vec 会让后提交的句子成为早提交句子的
    /// 上下文（"未来句"）；按序号只取"小于本次序号"的最近 N 条才是正确语义
    /// （方案 §4.6）。
    history: BTreeMap<u64, (String, String)>,
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
            params.thinking_unavailable,
            params.thinking_style.as_deref(),
            &params.api_base,
            &params.model,
        );
        if thinking != ThinkingPlan::None {
            // 日志可带形状名（排障必需），但用户可见文案一律由 UI 生成——
            // 降为 debug，避免日志窗直接显示供应商概念（INV-E）
            tracing::debug!(
                "Translator: thinking-disable shape {:?} for {} ({})",
                thinking,
                params.model,
                thinking_disable_body(thinking)
            );
        }
        if params.json_response {
            tracing::info!("Translator: json_response enabled for {}", params.model);
        }
        // 值为 null 的 override 键剔除（原版 `if v is not None`）；
        // 第二轮评审 ②：**已升为一等字段/应用不再发送的键一律剔除**——
        // 界面看不见的键绝不允许影响实际请求（"界面所见 = 实际所发"）
        let overrides: BTreeMap<String, Value> = params
            .overrides
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, v)| !v.is_null())
            .filter(|(k, _)| {
                let drop = OVERRIDE_KEYS_HIDDEN.contains(&k.as_str());
                if drop {
                    tracing::warn!(
                        "Translator: 忽略档案中已撤出界面的覆写键 '{k}'（不再参与请求）"
                    );
                }
                !drop
            })
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
        // 高级参数默认不发送（2026-09-10 裁决）：未手动指定即为 None/空
        if let (None, None) = (params.temperature, params.max_tokens) {
            tracing::debug!("Translator: 采样参数与输出上限均未指定（按最小请求发送）");
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
                history: BTreeMap::new(),
            })),
        })
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

    /// 复用同一 HTTP 客户端（基准测试的逐 chunk 计时需要自己驱动流，
    /// 但请求构造必须与生产同源——方案 §2.5 规则 6）
    pub(crate) fn client(&self) -> &async_openai::Client<OpenAIConfig> {
        &self.client
    }

    /// 本次装置实际采用的关闭形态（回退阶梯据此推进 / 记录学习结果）
    pub fn thinking_plan(&self) -> ThinkingPlan {
        self.thinking
    }

    /// 共享同一 client **与同一会话记忆**、仅覆盖"关闭形态"的新装置——
    /// 回退阶梯每上一级派生一个。历史/上下文是会话级的，必须一路传承
    /// （否则「上下文数」再次失效，这正是第二轮评审 ① 的回归）。
    pub fn with_plan(&self, thinking: ThinkingPlan) -> Translator {
        Translator {
            thinking,
            ..self.share_client()
        }
    }

    /// 共享同一 client 与会话记忆、仅覆盖"输出上限"的副本——体检判定为
    /// "被截断"时补发上限重试一次（方案 §4.4）
    pub fn with_max_tokens(&self, max_tokens: u32) -> Translator {
        Translator {
            max_tokens: Some(max_tokens),
            ..self.share_client()
        }
    }

    /// 最小请求形态（用户裁决）：不带任何可选参数——温度、覆写表、额外参数、
    /// 输出上限全部清空，关闭形态固定为"不发送"。只剩
    /// 「模型 + 系统提示词 + 待译文本 + 流式开关」（+ 流式用量探测）。
    pub fn minimal(&self) -> Translator {
        // 白名单构造（不是"复制再清几个"）：`json_response` 会往请求里塞
        // `response_format` 并改写系统提示词，而它已撤出界面——老档案带着它时
        // 最小请求就不再"最小"，被拒也再无退路（对抗审计实证）。保留的只有
        // 翻译必需项 + 用户可见的传输开关。
        Translator {
            client: self.client.clone(),
            model: self.model.clone(),
            streaming: self.streaming,
            json_response: false,
            thinking: ThinkingPlan::None,
            no_system_role: self.no_system_role,
            max_tokens: None,
            temperature: None,
            overrides: BTreeMap::new(),
            extra_body: Value::Object(Map::new()),
            system_prompt_template: self.system_prompt_template.clone(),
            state: self.state.clone(),
        }
    }

    /// 共享同一 client 与会话记忆的新 Translator（会话状态一路传承，
    /// 仅目标语言/超时随每次调用参数传入，W4）
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
            state: self.state.clone(),
        }
    }

    // ── prompt / messages 组装 ──

    /// 上下文里的最近 N 句（**只取提交序号小于本次的**——并发下绝不用"未来句"）
    fn recent_context(
        context_turns: u32,
        history: &BTreeMap<u64, (String, String)>,
        seq: u64,
    ) -> Vec<(String, String)> {
        if context_turns == 0 || history.is_empty() {
            return Vec::new();
        }
        let mut prior: Vec<&(String, String)> = history
            .range(..seq)
            .map(|(_, v)| v)
            .collect();
        let take = (context_turns as usize).min(prior.len());
        if take == 0 {
            return Vec::new();
        }
        prior.drain(..prior.len() - take);
        prior.into_iter().cloned().collect()
    }

    fn format_context(
        context_turns: u32,
        history: &BTreeMap<u64, (String, String)>,
        seq: u64,
    ) -> String {
        let ctx = Self::recent_context(context_turns, history, seq);
        if ctx.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        for (src, tgt) in &ctx {
            out.push_str(&format!("Source: {src}\nTranslation: {tgt}\n\n"));
        }
        out.trim_end().to_string()
    }

    fn build_system_prompt(&self, source_lang: &str, target_lang: &str, seq: u64) -> String {
        let src = lang_display(source_lang);
        let tgt = lang_display(target_lang);
        let st = self.state.lock();
        let context = Self::format_context(st.context_turns, &st.history, seq);
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

    fn build_messages(&self, system_prompt: &str, text: &str, seq: u64) -> Value {
        if self.no_system_role {
            return json!([{ "role": "user", "content": format!("{system_prompt}\n{text}") }]);
        }
        let mut msgs = vec![json!({"role": "system", "content": system_prompt})];
        // 模板含 {context} 时上下文已并入 system prompt，不再追加历史消息
        if !self.system_prompt_template.contains("{context}") {
            let st = self.state.lock();
            let ctx = Self::recent_context(st.context_turns, &st.history, seq);
            drop(st);
            for (src, tgt) in ctx {
                msgs.push(json!({"role": "user", "content": src}));
                msgs.push(json!({"role": "assistant", "content": tgt}));
            }
        }
        msgs.push(json!({"role": "user", "content": text}));
        json!(msgs)
    }

    fn append_history(&self, seq: u64, text: &str, result: &str) {
        append_history_locked(&mut self.state.lock(), seq, text, result);
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
    /// 公开供测试快照、调试与编排域的"请求体预览"使用（预览与实发**同一构造函数**，
    /// 方案 §2.5 规则 6）。`seq` = 本次翻译的提交序号（决定上下文取哪几句）。
    pub fn build_request_body(
        &self,
        system_prompt: &str,
        text: &str,
        stream: bool,
        include_usage: bool,
        seq: u64,
    ) -> Value {
        let mut body = Map::new();
        body.insert("model".into(), json!(self.model));
        body.insert("messages".into(), self.build_messages(system_prompt, text, seq));
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
        seq: u64,
    ) -> SyncOutcome {
        let body = self.build_request_body(system_prompt, text, false, false, seq);
        let resp: wire::ChatResponse = match self
            .timeout_block(self.client.chat().create_byot(body), timeout_secs)
        {
            Ok(r) => r,
            Err(e) => return SyncOutcome::failed(e),
        };
        let (pt, ct) = resp.usage_tokens();
        let usage_known = resp.usage.is_some();
        let reasoning_tokens = resp.reasoning_tokens();
        let choice = resp.choices.first();
        // W2/INV-F：先做思维链隔离，再 trim/JSON 提取/重复检测
        let cleaned = strip_reasoning(
            &choice
                .and_then(|c| c.message.content.clone())
                .unwrap_or_default(),
        );
        let mut result = cleaned.trim().to_string();
        if self.json_response {
            result = extract_json_translation(&result);
        }
        let verdict = classify_response(
            &result,
            finish_kind(choice.and_then(|c| c.finish_reason.as_deref())),
            reasoning_tokens,
        );
        warn_if_thinking_burned(&result, ct, self.thinking.name());
        SyncOutcome {
            result: Ok(result),
            verdict: Some(verdict),
            usage: (pt, ct),
            usage_known,
        }
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
    /// W4：`target_lang`/`timeout_secs` 为调用方从设置总线读出的生效值（TlView）；
    /// `seq` = 提交序号（上下文只取此序号之前的句子）。
    pub fn translate(
        &self,
        text: &str,
        source_language: &str,
        target_lang: &str,
        timeout_secs: u32,
        seq: u64,
    ) -> Result<String, TranslateError> {
        let mut last: Option<Result<String, TranslateError>> = None;
        for item in self.translate_iter(text, source_language, target_lang, timeout_secs, seq) {
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
        seq: u64,
    ) -> TranslateStream {
        let system_prompt = self.build_system_prompt(source_language, target_lang, seq);
        if !self.streaming {
            let outcome = self.translate_sync(&system_prompt, text, timeout_secs as u64, seq);
            if let Ok(r) = &outcome.result {
                self.append_history(seq, text, r);
            }
            // 体检结论与用量随 SyncOutcome 一并带出（W2）
            return TranslateStream::sync(outcome);
        }

        let body = self.build_request_body(&system_prompt, text, true, false, seq);
        let body_with_usage = {
            let mut m = body.as_object().expect("body 是 object").clone();
            m.insert("stream_options".into(), json!({"include_usage": true}));
            Value::Object(m)
        };
        let read_timeout = Duration::from_secs(timeout_secs as u64);
        let (tx, rx) = mpsc::channel();
        let client = self.client.clone();
        let handle = runtime().spawn(async move {
            pump_stream(client, body_with_usage, body, read_timeout, tx).await;
        });
        let deadline = Instant::now() + read_timeout;
        TranslateStream {
            inner: StreamInner::Streaming {
                rx,
                deadline,
                text: text.to_string(),
                timeout_secs: read_timeout.as_secs(),
                handle,
            },
            json_response: self.json_response,
            thinking: self.thinking,
            state: self.state.clone(),
            seq,
            finished: false,
            stripper: ReasoningStripper::new(),
            verdict: None,
            usage: (0, 0),
            usage_known: false,
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
        .create_stream_byot::<Value, wire::ChatChunk>(with_usage)
        .await
    {
        Ok(s) => s,
        Err(_) => match client
            .chat()
            .create_stream_byot::<Value, wire::ChatChunk>(plain)
            .await
    {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(StreamMsg::Err(TranslateError::from(e)));
                return;
            }
        },
    };

    // W2：除用量外，同时记录推理量与结束原因（体检输入，方案 §4.4）
    let (mut pt, mut ct) = (0u64, 0u64);
    let mut reasoning_tokens: Option<u32> = None;
    let mut finish: Option<FinishKind> = None;
    let mut usage_known = false;
    loop {
        match tokio::time::timeout(read_timeout, stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                if let Some(usage) = &chunk.usage {
                    // 用量只在末块出现（DeepSeek 无独立 usage chunk，挂最后一个内容块；
                    // Kimi 另发一个 choices 为空的 usage-only chunk）——两者都落这里
                    usage_known = true;
                    pt = usage.prompt_tokens.unwrap_or(0);
                    ct = usage.completion_tokens.unwrap_or(0);
                    reasoning_tokens = usage.reasoning_tokens();
                }
                if let Some(f) = chunk.finish_reason() {
                    finish = Some(f);
                }
                if let Some(delta) = chunk.delta_content().filter(|s| !s.is_empty()) {
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
        reasoning_tokens,
        finish,
        usage_known,
    });
}

enum StreamMsg {
    Delta(String),
    End {
        prompt_tokens: u64,
        completion_tokens: u64,
        reasoning_tokens: Option<u32>,
        finish: Option<FinishKind>,
        usage_known: bool,
    },
    Err(TranslateError),
}

/// 非流式一次调用的完整产出（W2：体检结论与用量随结果返回，
/// 不再经 `Translator` 的共享态回传——方案 §4.6 账本归属）
struct SyncOutcome {
    result: Result<String, TranslateError>,
    verdict: Option<ResponseVerdict>,
    usage: (u64, u64),
    /// 服务端是否返回了用量对象（第二轮评审 ⑩：不返回时界面显示"—"而非 0）
    usage_known: bool,
}

impl SyncOutcome {
    fn failed(e: TranslateError) -> Self {
        Self {
            result: Err(e),
            verdict: None,
            usage: (0, 0),
            usage_known: false,
        }
    }
}

enum StreamInner {
    Sync(Option<SyncOutcome>),
    Streaming {
        rx: mpsc::Receiver<StreamMsg>,
        deadline: Instant,
        text: String,
        timeout_secs: u64,
        /// W5/INV-C：pump 任务句柄——消费方放弃时中止，服务端不再空转
        /// （本机模型尤其重要：推理型模型一个 content 增量都不发，
        /// 旧实现要等它把整段生成完才能发现接收端已关闭）
        handle: tokio::task::JoinHandle<()>,
    },
}

/// translate_iter 的消费端迭代器
pub struct TranslateStream {
    inner: StreamInner,
    json_response: bool,
    /// 本次装置的关闭形态（仅用于告警文案；体检结论见 [`TranslateStream::verdict`]）
    thinking: ThinkingPlan,
    state: Arc<Mutex<MutableState>>,
    /// 本次翻译的提交序号（历史按序号入档，并发下不产生"未来句"上下文）
    seq: u64,
    finished: bool,
    /// W2/INV-F：流式思维链隔离器——写进 content 的思考块绝不外泄
    stripper: ReasoningStripper,
    /// W2：本次调用的体检结论（迭代结束后有效）
    verdict: Option<ResponseVerdict>,
    /// W2：本次调用的 (prompt, completion) 用量
    usage: (u64, u64),
    /// 第二轮评审 ⑩：服务端是否真的返回了用量（false → 界面显示"—"而不是 0）
    usage_known: bool,
}

impl TranslateStream {
    fn sync(outcome: SyncOutcome) -> Self {
        let verdict = outcome.verdict;
        let usage = outcome.usage;
        let usage_known = outcome.usage_known;
        Self {
            inner: StreamInner::Sync(Some(outcome)),
            json_response: false,
            thinking: ThinkingPlan::None,
            state: Arc::new(Mutex::new(MutableState {
                context_turns: 0,
                history: BTreeMap::new(),
            })),
            seq: 0,
            finished: false,
            stripper: ReasoningStripper::new(),
            verdict,
            usage,
            usage_known,
        }
    }

    /// W2：本次调用的体检结论（迭代结束后有效；请求层错误时为 None）
    pub fn verdict(&self) -> Option<ResponseVerdict> {
        self.verdict
    }

    /// W2：本次调用的 (prompt_tokens, completion_tokens)
    pub fn usage(&self) -> (u64, u64) {
        self.usage
    }

    /// 第二轮评审 ⑩：服务端是否提供了用量统计（不提供时调用方不得把 0 当真实值展示）
    pub fn usage_known(&self) -> bool {
        self.usage_known
    }
}

/// W5/INV-C：迭代器被丢弃（超时放弃、测试连接提前收手、切模型、停机）即中止
/// pump 任务——HTTP 连接随之关闭，服务端停止生成
impl Drop for TranslateStream {
    fn drop(&mut self) {
        if let StreamInner::Streaming { handle, .. } = &self.inner {
            handle.abort();
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
                item.take().map(|o| o.result)
            }
            StreamInner::Streaming {
                rx,
                deadline,
                text,
                timeout_secs,
                ..
            } => loop {
                let now = Instant::now();
                let timeout_msg = format!("Translation exceeded {timeout_secs}s total timeout");
                let Some(remaining) = deadline.checked_duration_since(now) else {
                    self.finished = true;
                    return Some(Err(TranslateError::Timeout(timeout_msg)));
                };
                match rx.recv_timeout(remaining) {
                    Ok(StreamMsg::Delta(d)) => {
                        // W2/INV-F：先过思维链隔离器，再产出可见增量
                        let visible = self.stripper.push(&d);
                        if self.json_response {
                            // json 模式中途不出部分结果，继续消费直到流结束
                            continue;
                        }
                        return Some(Ok(visible));
                    }
                    Ok(StreamMsg::End {
                        prompt_tokens,
                        completion_tokens,
                        reasoning_tokens,
                        finish,
                        usage_known,
                    }) => {
                        self.usage = (prompt_tokens, completion_tokens);
                        self.usage_known = usage_known;
                        // 流结束：隔离器 flush（未闭合的思考块整块丢弃）
                        let stripper = std::mem::take(&mut self.stripper);
                        let mut result = stripper.finish().trim().to_string();
                        if self.json_response {
                            result = extract_json_translation(&result);
                        }
                        self.verdict =
                            Some(classify_response(&result, finish, reasoning_tokens));
                        warn_if_thinking_burned(&result, completion_tokens, self.thinking.name());
                        if check_repetition(&result) {
                            self.finished = true;
                            return Some(Err(TranslateError::Repetition(result)));
                        }
                        let text = std::mem::take(text);
                        append_history_locked(&mut self.state.lock(), self.seq, &text, &result);
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
/// 超过 context_turns+2 条裁到最近 context_turns 条）。按**提交序号**入档：
/// 并发 8 worker 下完成序与提交序不同，按序号才能保证上下文只含"更早的句子"。
fn append_history_locked(st: &mut MutableState, seq: u64, text: &str, result: &str) {
    if st.context_turns > 0 && !result.is_empty() {
        st.history.insert(seq, (text.to_string(), result.to_string()));
        let max_keep = st.context_turns as usize + 2;
        while st.history.len() > max_keep {
            let Some((&oldest, _)) = st.history.iter().next() else {
                break;
            };
            st.history.remove(&oldest);
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
            "Empty translation but {completion_tokens} completion tokens were used - the model              likely spent the whole output budget on reasoning (shape: {plan}); the fallback              ladder will step automatically"
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

/// 响应线格式的**宽容**类型（第二轮评审 ⑧）：只声明我们真正要用的字段，
/// 认不出的取值一律忽略——而不是让一个陌生的枚举取值（如 DeepSeek 的
/// `insufficient_system_resource` / `aborted`、网关自定义字段）打死整段响应。
/// 未知字段由 serde 默认忽略；`finish_reason` 保持字符串、由
/// [`crate::verdict::finish_kind`] 归一化，未知值落 `FinishKind::Other`。
pub(crate) mod wire {
    use serde::Deserialize;
    use crate::verdict::{finish_kind, FinishKind};

    #[derive(Debug, Deserialize, Default)]
    pub struct Delta {
        #[serde(default)]
        pub content: Option<String>,
    }

    #[derive(Debug, Deserialize, Default)]
    pub struct StreamChoice {
        #[serde(default)]
        pub delta: Delta,
        #[serde(default)]
        pub finish_reason: Option<String>,
    }

    #[derive(Debug, Deserialize, Default)]
    pub struct CompletionTokensDetails {
        #[serde(default)]
        pub reasoning_tokens: Option<u32>,
    }

    #[derive(Debug, Deserialize, Default)]
    pub struct Usage {
        #[serde(default)]
        pub prompt_tokens: Option<u64>,
        #[serde(default)]
        pub completion_tokens: Option<u64>,
        #[serde(default)]
        pub completion_tokens_details: Option<CompletionTokensDetails>,
    }

    impl Usage {
        pub fn reasoning_tokens(&self) -> Option<u32> {
            self.completion_tokens_details
                .as_ref()
                .and_then(|d| d.reasoning_tokens)
        }
    }

    /// 流式分片
    #[derive(Debug, Deserialize, Default)]
    pub struct ChatChunk {
        #[serde(default)]
        pub choices: Vec<StreamChoice>,
        #[serde(default)]
        pub usage: Option<Usage>,
    }

    impl ChatChunk {
        pub fn delta_content(&self) -> Option<&str> {
            self.choices
                .first()
                .and_then(|c| c.delta.content.as_deref())
        }

        pub fn finish_reason(&self) -> Option<FinishKind> {
            self.choices
                .first()
                .and_then(|c| c.finish_reason.as_deref())
                .map(|s| finish_kind(Some(s)))
                .unwrap_or(None)
        }
    }

    #[derive(Debug, Deserialize, Default)]
    pub struct RespMessage {
        #[serde(default)]
        pub content: Option<String>,
    }

    #[derive(Debug, Deserialize, Default)]
    pub struct RespChoice {
        #[serde(default)]
        pub message: RespMessage,
        #[serde(default)]
        pub finish_reason: Option<String>,
    }

    /// 非流式响应
    #[derive(Debug, Deserialize, Default)]
    pub struct ChatResponse {
        #[serde(default)]
        pub choices: Vec<RespChoice>,
        #[serde(default)]
        pub usage: Option<Usage>,
    }

    impl ChatResponse {
        pub fn usage_tokens(&self) -> (u64, u64) {
            self.usage
                .as_ref()
                .map(|u| (u.prompt_tokens.unwrap_or(0), u.completion_tokens.unwrap_or(0)))
                .unwrap_or((0, 0))
        }

        pub fn reasoning_tokens(&self) -> Option<u32> {
            self.usage.as_ref().and_then(Usage::reasoning_tokens)
        }
    }

    /// 宽容解码的守卫（第二轮评审 ⑧ 的回归面）：DeepSeek 文档列出的
    /// `insufficient_system_resource`/`aborted`、网关自定义 `service_tier`
    /// 都不得让整段响应解码失败
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn unknown_finish_reason_and_extra_fields_are_tolerated() {
            let raw = r#"{
                "id":"x","object":"chat.completion.chunk","created":1,"model":"m",
                "service_tier":"scale_v2",
                "choices":[{"index":0,"delta":{"content":"译文"},"finish_reason":"insufficient_system_resource"}]
            }"#;
            let chunk: ChatChunk = serde_json::from_str(raw).expect("不得因陌生取值解码失败");
            assert_eq!(chunk.delta_content(), Some("译文"));
            assert_eq!(chunk.finish_reason(), Some(FinishKind::Other));
        }

        #[test]
        fn null_usage_and_missing_fields_are_tolerated() {
            let raw = r#"{"choices":[{"index":0,"delta":{"content":"a"},"finish_reason":null}],"usage":null}"#;
            let chunk: ChatChunk = serde_json::from_str(raw).expect("usage=null 可解");
            assert_eq!(chunk.delta_content(), Some("a"));
            assert_eq!(chunk.finish_reason(), None);
            let raw = r#"{"choices":[]}"#;
            let chunk: ChatChunk = serde_json::from_str(raw).expect("空 choices 可解");
            assert_eq!(chunk.delta_content(), None);
        }

        #[test]
        fn non_streaming_response_is_tolerated() {
            let raw = r#"{"choices":[{"message":{"content":"你好","reasoning_content":"思考"},"finish_reason":"aborted"}],
                          "usage":{"prompt_tokens":3,"completion_tokens":5,"completion_tokens_details":{"reasoning_tokens":2}}}"#;
            let resp: ChatResponse = serde_json::from_str(raw).expect("非流式可解");
            assert_eq!(resp.usage_tokens(), (3, 5));
            assert_eq!(resp.reasoning_tokens(), Some(2));
            assert_eq!(
                resp.choices.first().and_then(|c| c.message.content.as_deref()),
                Some("你好")
            );
        }
    }
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

#[cfg(test)]
mod drop_tests {
    use super::*;

    /// W5/INV-C：丢弃迭代器必须中止 pump 任务（否则连不上的端点会一直挂着
    /// 直到连接超时，推理型模型更会白跑完整段生成）
    #[test]
    fn dropping_stream_aborts_pump_task() {
        // 黑洞地址：连接不会被立即拒绝，pump 保持挂起（否则任务可能自行结束，
        // 测不出 abort 的效果）
        let t = Translator::for_test("http://10.255.255.1:9/v1", "m", true, None, None);
        let it = t.translate_iter("hi", "en", "zh", 30, 0);
        let (abort, finished_before) = match &it.inner {
            StreamInner::Streaming { handle, .. } => (handle.abort_handle(), handle.is_finished()),
            _ => panic!("应为流式迭代器"),
        };
        assert!(!finished_before, "pump 任务应仍在挂起");
        drop(it);
        // abort 异步生效：给调度器一点时间
        let deadline = Instant::now() + Duration::from_secs(2);
        while !abort.is_finished() {
            assert!(Instant::now() < deadline, "drop 后 pump 任务未被中止");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// 非流式分支没有 pump 任务，drop 应无害
    #[test]
    fn dropping_sync_stream_is_harmless() {
        let _t = Translator::for_test("http://10.255.255.1:9/v1", "m", true, None, None);
        let it = TranslateStream::sync(SyncOutcome {
            result: Ok("x".into()),
            verdict: Some(ResponseVerdict::Ok),
            usage: (1, 2),
            usage_known: true,
        });
        drop(it);
    }
}
