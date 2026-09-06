//! Whisper 引擎（whisper-rs 0.16 / whisper.cpp 纯 CPU；原版 asr_engine.py 的
//! faster-whisper 语义对齐）。
//!
//! 参数对照（原版 → 本版）：
//! - `beam_size=5` → `SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 }`
//!   【API 差异】whisper-rs 0.16 的 BeamSearch 必填 `patience`（whisper.cpp 未实现，
//!   官方默认 -1.0，语义与不传一致）
//! - `language=auto→None` → `set_language(Option<&str>)`；检测：whisper.cpp 在
//!   full() 期间自动完成，`full_lang_id_from_state()` + `get_lang_str()` 转 ISO 码
//!   【API 差异】PLAN §2.7 预估的 `state.detect_language()` 不存在
//! - task=transcribe → whisper.cpp 默认即 transcribe（0.16 无 set_task/Task）
//! - suppress non-speech → `set_suppress_nst(true)`
//!   （PLAN §2.7 的 `set_suppress_non_speech_tokens` 已改名；原版 faster-whisper
//!   未显式传该参，其默认 false——此处按 PLAN「对齐原版体验」取 true，M5 实测定）
//! - pad 桶 → 与 SenseVoice 相同的尾部补零至 quantum 整倍（原版 _prepare_audio_input）
//! - 输出：段文本逐段 trim 后空格连接（原版 `" ".join(seg.text.strip())`）；
//!   空文本视为无结果；`language_name` 查 `lt_proto::language_display`
//!
//! stdout 纪律（E-07）：worker 的 stdout 是协议通道，whisper.cpp 的进度/时间戳
//! 打印全部关闭（print_* 四项 false）；C++ 侧残留日志走 stderr，由父进程重定向。

use crate::engine::AsrEngine;
use crate::worker::WorkerConfig;
use lt_proto::{language_display, AsrResult, EngineError};
use std::path::{Path, PathBuf};
use whisper_rs::{
    get_lang_str, FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
    WhisperState,
};

pub const SAMPLE_RATE: u32 = 16000;
pub const DEFAULT_PAD_SECONDS: f32 = 0.5;
/// 原版 faster-whisper `beam_size=5`
pub const BEAM_SIZE: i32 = 5;

pub struct WhisperEngine {
    /// 声明序决定 drop 序：state 先于 ctx 释放（whisper.cpp 约束）
    state: WhisperState,
    _ctx: WhisperContext,
    #[allow(dead_code)]
    model_path: PathBuf,
    /// None = auto（full() 时由 whisper.cpp 检测）
    language: Option<String>,
    /// pad 桶样本数（0 = 禁用）；对齐原版 int(round(16000*pad))
    pad_quantum: usize,
}

impl WhisperEngine {
    /// 加载 GGML 模型（worker 进程内调用，180s ready 超时内完成）
    pub fn load(model_path: &Path, pad_seconds: Option<f32>, language: &str) -> Result<Self, EngineError> {
        if !model_path.is_file() {
            return Err(EngineError::Load(format!("模型文件不存在: {}", model_path.display())));
        }
        // 纯 CPU 硬约束：显式关 GPU（whisper-rs 编译期也未开任何 GPU feature）
        let mut ctx_params = WhisperContextParameters::new();
        ctx_params.use_gpu = false;
        let ctx = WhisperContext::new_with_params(model_path, ctx_params)
            .map_err(|e| EngineError::Load(format!("whisper 上下文创建失败（{}）: {e}", model_path.display())))?;
        let state = ctx
            .create_state()
            .map_err(|e| EngineError::Load(format!("whisper state 创建失败: {e}")))?;
        let language = normalize_language(language);
        let pad = pad_seconds.unwrap_or(DEFAULT_PAD_SECONDS);
        let pad_quantum = (SAMPLE_RATE as f32 * pad).round() as usize;
        tracing::info!(
            "Whisper loaded: {} (beam={BEAM_SIZE}, pad_quantum={pad_quantum}, language={})",
            model_path.display(),
            language.as_deref().unwrap_or("auto"),
        );
        Ok(Self {
            state,
            _ctx: ctx,
            model_path: model_path.to_path_buf(),
            language,
            pad_quantum,
        })
    }

    /// worker 工厂：从 WorkerConfig.options 解析 model_path（装配层约定）
    pub fn from_config(cfg: &WorkerConfig) -> Result<Self, EngineError> {
        let model_path = cfg
            .options
            .get("model_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| EngineError::Load("options 缺少 model_path".into()))?;
        Self::load(Path::new(model_path), cfg.pad_seconds, &cfg.language)
    }

    /// lang_id → ISO 码（whisper.cpp 静态表；检测不到回退 "auto"）
    fn lang_iso(lang_id: i32) -> String {
        get_lang_str(lang_id).unwrap_or("auto").to_string()
    }

    /// 段文本拼接（原版 `" ".join(text_parts).strip()`）；全空 → None
    fn join_segments(state: &WhisperState) -> Option<String> {
        let n = state.full_n_segments();
        let mut parts: Vec<String> = Vec::new();
        for i in 0..n {
            if let Some(seg) = state.get_segment(i) {
                let t = match seg.to_str_lossy() {
                    Ok(t) => t.trim().to_string(),
                    Err(_) => continue,
                };
                if !t.is_empty() {
                    parts.push(t);
                }
            }
        }
        let joined = parts.join(" ").trim().to_string();
        (!joined.is_empty()).then_some(joined)
    }
}

/// "auto"/"" → None（auto 检测）；其余原样
fn normalize_language(language: &str) -> Option<String> {
    match language {
        "" | "auto" | "Auto" => None,
        l => Some(l.to_string()),
    }
}

/// pad 桶（原版 _prepare_audio_input：尾部补零至 quantum 整倍）。
/// 返回 None 表示无需补零（quantum=0 或已整倍）。
fn pad_samples(audio: &[f32], quantum: usize) -> Option<Vec<f32>> {
    if quantum == 0 || audio.is_empty() || audio.len().is_multiple_of(quantum) {
        return None;
    }
    let target = (audio.len() / quantum + 1) * quantum;
    let mut out = audio.to_vec();
    out.resize(target, 0.0);
    Some(out)
}

impl AsrEngine for WhisperEngine {
    fn transcribe(&mut self, audio: &[f32], _word_timestamps: bool) -> Result<AsrResult, EngineError> {
        if audio.is_empty() {
            return Ok(AsrResult::default());
        }
        // pad 桶
        let padded;
        let audio: &[f32] = match pad_samples(audio, self.pad_quantum) {
            Some(v) => {
                padded = v;
                &padded
            }
            None => audio,
        };

        // 参数按次构造（语言字符串借用 self.language 字段，与 self.state 不冲突）
        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: BEAM_SIZE,
            patience: -1.0,
        });
        params.set_language(self.language.as_deref()); // None = auto 检测
        // task 默认 Transcribe（0.16 无 set_task；translate 默认 false）
        params.set_suppress_nst(true);
        // stdout 纪律：全部打印关闭（worker stdout 是协议通道）
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        self.state
            .full(params, audio)
            .map_err(|e| EngineError::Runtime { message: format!("whisper 推理失败: {e}"), recoverable: true })?;

        let Some(text) = Self::join_segments(&self.state) else {
            return Ok(AsrResult::default());
        };
        // 语言：显式设置用设置值；auto 用本次 full() 的检测结果（ISO 码）
        let lang = match &self.language {
            Some(l) => l.clone(),
            None => {
                let id = self.state.full_lang_id_from_state();
                Self::lang_iso(id)
            }
        };
        Ok(AsrResult {
            text,
            language: lang.clone(),
            language_name: language_display(&lang).to_string(),
            // word_timestamps：token 级 API 保留（PLAN §2.7），当前管道恒传 false，暂不产出
            words: None,
        })
    }

    fn set_language(&mut self, language: &str) -> Result<(), EngineError> {
        // whisper 语言是每次推理的参数（faster-whisper 同语义）：直接换值即可
        self.language = normalize_language(language);
        Ok(())
    }

    fn set_input_padding(&mut self, pad_seconds: f32) -> Result<(), EngineError> {
        self.pad_quantum = (SAMPLE_RATE as f32 * pad_seconds).round() as usize;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_language_semantics() {
        assert_eq!(normalize_language("auto"), None);
        assert_eq!(normalize_language(""), None);
        assert_eq!(normalize_language("Auto"), None);
        assert_eq!(normalize_language("en"), Some("en".into()));
        assert_eq!(normalize_language("zh"), Some("zh".into()));
    }

    #[test]
    fn lang_iso_uses_whisper_static_table() {
        // whisper.cpp 静态语言表：0=en（与模型无关，纯函数可离线测）
        assert_eq!(WhisperEngine::lang_iso(0), "en");
        assert_eq!(WhisperEngine::lang_iso(1), "zh");
        // 越界/未知 id 回退 "auto"
        assert_eq!(WhisperEngine::lang_iso(-1), "auto");
        assert_eq!(WhisperEngine::lang_iso(9999), "auto");
    }

    #[test]
    fn pad_bucket_matches_original_semantics() {
        // quantum=0 禁用；整倍不补；余数补齐至下一整倍
        assert_eq!(pad_samples(&[0.1, 0.2], 0), None);
        assert_eq!(pad_samples(&[0.1, 0.2, 0.3, 0.4], 2), None);
        assert_eq!(
            pad_samples(&[0.1, 0.2, 0.3], 2),
            Some(vec![0.1, 0.2, 0.3, 0.0])
        );
        // 空音频不补（transcribe 对空输入直接短路，不进这里，防御一致）
        assert_eq!(pad_samples(&[], 2), None);
    }

    #[test]
    fn from_config_requires_model_path_option() {
        let cfg = WorkerConfig {
            engine: "whisper".into(),
            display_name: "Whisper tiny".into(),
            language: "auto".into(),
            pad_seconds: Some(0.5),
            options: serde_json::json!({}),
        };
        let err = WhisperEngine::from_config(&cfg);
        assert!(matches!(err, Err(EngineError::Load(_))));
        // model_path 指向不存在文件 → Load 错误（不 panic）
        let cfg = WorkerConfig {
            options: serde_json::json!({ "model_path": "Z:/no/such/ggml-tiny-q5_1.bin" }),
            ..cfg
        };
        let err = WhisperEngine::from_config(&cfg);
        assert!(matches!(err, Err(EngineError::Load(_))));
    }

    /// 真模型冒烟（默认跳过）：LT_WHISPER_MODEL 指向 ggml .bin（如 tiny q5_1）
    /// 时启用。不下载、不跑大模型推理，符合"单测全离线"约束。
    /// 运行：`LT_WHISPER_MODEL=<path> cargo test -p lt-asr real_model_smoke -- --ignored`
    #[test]
    #[ignore = "需真实模型：设 LT_WHISPER_MODEL 指向 ggml .bin 后加 --ignored 运行"]
    fn real_model_smoke() {
        let Ok(model) = std::env::var("LT_WHISPER_MODEL") else {
            return; // 未提供模型 → 静默跳过
        };
        let mut engine = WhisperEngine::load(Path::new(&model), Some(0.5), "auto").expect("加载");
        // 1s 的 440Hz 正弦（幅度低，期望空/近空结果，只验证链路不崩）
        let audio: Vec<f32> = (0..16000)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 16000.0) * 0.05)
            .collect();
        let res = engine.transcribe(&audio, false).expect("推理");
        // 语言必须回 ISO 码或 auto
        assert!(res.language.is_empty() || res.language == "auto" || res.language.len() == 2);
        engine.set_language("en").expect("set_language");
        engine.set_input_padding(0.0).expect("set_padding");
        let _ = engine.transcribe(&audio, true).expect("推理2");
    }

    /// 真语音转写端到端（默认跳过）：LT_WHISPER_MODEL（ggml .bin）+
    /// LT_WHISPER_SPEECH_WAV（16kHz 单声道 16-bit PCM wav，如 whisper.cpp 仓
    /// samples/jfk.wav）→ 断言英文转写命中预期关键词、语言检测为 en。
    /// 运行：`LT_WHISPER_MODEL=... LT_WHISPER_SPEECH_WAV=... \
    ///        cargo test -p lt-asr real_model_speech -- --ignored`
    #[test]
    #[ignore = "需真实模型与语音样本：设 LT_WHISPER_MODEL/LT_WHISPER_SPEECH_WAV 后 --ignored 运行"]
    fn real_model_speech() {
        let (Ok(model), Ok(wav_path)) =
            (std::env::var("LT_WHISPER_MODEL"), std::env::var("LT_WHISPER_SPEECH_WAV"))
        else {
            return; // 未提供 → 静默跳过
        };
        let audio = read_wav_mono_16k(Path::new(&wav_path)).expect("读取 wav");
        assert!(audio.len() > 16000, "语音样本应长于 1s: {} 样本", audio.len());

        let mut engine = WhisperEngine::load(Path::new(&model), Some(0.0), "auto").expect("加载");
        let res = engine.transcribe(&audio, false).expect("推理");
        let text = res.text.to_lowercase();
        assert!(
            text.contains("fellow americans") || text.contains("my fellow"),
            "转写未命中预期关键词: {text:?}"
        );
        assert_eq!(res.language, "en", "auto 检测应为 en");
    }

    /// 最小 WAV 读取（测试专用）：仅支持 PCM 单声道 16kHz/16-bit → f32 归一化。
    /// jfk.wav（whisper.cpp 仓 samples）即该格式。
    fn read_wav_mono_16k(path: &Path) -> Result<Vec<f32>, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err("非 RIFF/WAVE".into());
        }
        let mut pos = 12usize;
        let (mut rate, mut channels, mut bits) = (0u32, 0u16, 0u16);
        let mut pcm: Option<Vec<i16>> = None;
        while pos + 8 <= bytes.len() {
            let id = &bytes[pos..pos + 4];
            let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
            let body = &bytes[pos + 8..(pos + 8 + size).min(bytes.len())];
            match id {
                b"fmt " => {
                    channels = u16::from_le_bytes(body[2..4].try_into().unwrap());
                    rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                    bits = u16::from_le_bytes(body[14..16].try_into().unwrap());
                }
                b"data" => {
                    pcm = Some(
                        body.chunks_exact(2)
                            .map(|c| i16::from_le_bytes(c.try_into().unwrap()))
                            .collect(),
                    );
                }
                _ => {}
            }
            pos += 8 + size + (size & 1); // chunk 按字对齐
        }
        if rate != 16000 || channels != 1 || bits != 16 {
            return Err(format!("仅支持 16kHz/单声道/16-bit，实为 {rate}Hz/{channels}ch/{bits}bit"));
        }
        Ok(pcm
            .ok_or("缺少 data chunk")?
            .into_iter()
            .map(|s| s as f32 / 32768.0)
            .collect())
    }
}
