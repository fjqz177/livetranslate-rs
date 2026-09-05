//! SenseVoice 引擎（sherpa-onnx 离线识别；原版 asr_sensevoice.py 的 Rust 等价）。
//!
//! 后处理 1:1：首命中语言标签定语言并移除 → 正则清除全部 `<|...|>` 标签
//! （情感/事件/BGM）→ trim → 空文本视为无结果。

use crate::engine::AsrEngine;
use lt_proto::{AsrResult, EngineError};
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig,
};
use std::path::{Path, PathBuf};

/// 语言标签表（原版 LANG_MAP；首个命中定语言）
const LANG_TAGS: [(&str, &str); 5] = [
    ("<|zh|>", "zh"),
    ("<|en|>", "en"),
    ("<|ja|>", "ja"),
    ("<|ko|>", "ko"),
    ("<|yue|>", "yue"),
];

pub const SAMPLE_RATE: u32 = 16000;
pub const DEFAULT_PAD_SECONDS: f32 = 0.5;

pub struct SenseVoiceEngine {
    recognizer: OfflineRecognizer,
    model_path: PathBuf,
    tokens_path: PathBuf,
    /// None = auto；变更时懒重建识别器（sherpa 语言是创建期参数）
    language: Option<String>,
    /// pad 桶样本数（0 = 禁用）；对齐原版 int(round(16000*pad))
    pad_quantum: usize,
}

impl SenseVoiceEngine {
    /// 从模型目录加载：`model.int8.onnx`（或 `model.onnx`）+ `tokens.txt`
    pub fn load(model_dir: &Path, pad_seconds: Option<f32>, language: &str) -> Result<Self, EngineError> {
        let model = pick_model_file(model_dir)?;
        let tokens = model_dir.join("tokens.txt");
        if !tokens.is_file() {
            return Err(EngineError::Load(format!("tokens.txt 不存在: {}", tokens.display())));
        }
        let language = normalize_language(language);
        let pad = pad_seconds.unwrap_or(DEFAULT_PAD_SECONDS);
        let pad_quantum = (SAMPLE_RATE as f32 * pad).round() as usize;
        let recognizer = build_recognizer(&model, &tokens, language.as_deref())?;
        tracing::info!(
            "SenseVoice loaded: {} (pad_quantum={pad_quantum}, language={})",
            model_dir.display(),
            language.as_deref().unwrap_or("auto")
        );
        Ok(Self {
            recognizer,
            model_path: model,
            tokens_path: tokens,
            language,
            pad_quantum,
        })
    }

    /// 语言标签剥离（§5.2；单测覆盖）
    fn postprocess(raw: &str) -> Option<(String, String)> {
        let mut detected = "auto";
        let mut text = raw.to_string();
        for (tag, lang) in LANG_TAGS {
            if text.contains(tag) {
                detected = lang;
                text = text.replace(tag, "");
                break;
            }
        }
        // 剔除全部 <|...|>（情感/事件/BGM 等）
        let mut out = String::with_capacity(text.len());
        let mut rest = text.as_str();
        while let Some(start) = rest.find("<|") {
            out.push_str(&rest[..start]);
            match rest[start..].find("|>") {
                Some(end) => rest = &rest[start + end + 2..],
                None => {
                    out.push_str(&rest[start..]);
                    rest = "";
                    break;
                }
            }
        }
        out.push_str(rest);
        let out = out.trim().to_string();
        if out.is_empty() {
            None
        } else {
            Some((out, detected.to_string()))
        }
    }
}

fn pick_model_file(dir: &Path) -> Result<PathBuf, EngineError> {
    for name in ["model.int8.onnx", "model.onnx"] {
        let p = dir.join(name);
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(EngineError::Load(format!(
        "model.int8.onnx / model.onnx 均不存在于 {}",
        dir.display()
    )))
}

fn normalize_language(language: &str) -> Option<String> {
    match language {
        "" | "auto" | "Auto" => None,
        l => Some(l.to_string()),
    }
}

fn build_recognizer(
    model: &Path,
    tokens: &Path,
    language: Option<&str>,
) -> Result<OfflineRecognizer, EngineError> {
    let mut cfg = OfflineRecognizerConfig::default();
    cfg.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: Some(model.to_string_lossy().into_owned()),
        language: Some(language.unwrap_or("auto").to_string()),
        use_itn: true,
    };
    cfg.model_config.tokens = Some(tokens.to_string_lossy().into_owned());
    cfg.model_config.num_threads = 1;
    OfflineRecognizer::create(&cfg)
        .ok_or_else(|| EngineError::Load(format!("sherpa 创建识别器失败（model={}）", model.display())))
}

/// 尾部补零至 target（target ≥ len）
fn pad_to_len(audio: &[f32], target: usize) -> Vec<f32> {
    let mut out = audio.to_vec();
    out.resize(target, 0.0);
    out
}

impl AsrEngine for SenseVoiceEngine {
    fn transcribe(&mut self, audio: &[f32], _word_timestamps: bool) -> Result<AsrResult, EngineError> {
        if audio.is_empty() {
            return Ok(AsrResult::default());
        }
        // pad 桶（原版 _prepare_audio_input：尾部补零至 quantum 整倍）
        let padded;
        let audio: &[f32] = if self.pad_quantum > 0 && audio.len() % self.pad_quantum != 0 {
            let target = (audio.len() / self.pad_quantum + 1) * self.pad_quantum;
            padded = pad_to_len(audio, target);
            &padded
        } else {
            audio
        };

        let stream = self.recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE as i32, audio);
        self.recognizer.decode(&stream);
        let Some(result) = stream.get_result() else {
            return Ok(AsrResult::default());
        };

        match Self::postprocess(&result.text) {
            None => Ok(AsrResult::default()),
            Some((text, lang)) => Ok(AsrResult {
                text,
                language: lang.clone(),
                language_name: lang, // 原版：language_name = language（不查显示名表）
                words: None,
            }),
        }
    }

    fn set_language(&mut self, language: &str) -> Result<(), EngineError> {
        // sherpa 语言是创建期参数 → 变更即重建识别器（原版每次 generate 传参，
        // 语义等价；切换很少见，重建成本可接受）
        let new_lang = normalize_language(language);
        if new_lang != self.language {
            self.recognizer =
                build_recognizer(&self.model_path, &self.tokens_path, new_lang.as_deref())?;
            self.language = new_lang;
        }
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
    fn postprocess_strips_tags_and_detects_language() {
        assert_eq!(
            SenseVoiceEngine::postprocess("<|zh|>你好世界"),
            Some(("你好世界".into(), "zh".into()))
        );
        assert_eq!(
            SenseVoiceEngine::postprocess("<|en|>hello <|HAPPY|>world"),
            Some(("hello world".into(), "en".into()))
        );
        // 无语言标签但含事件标签
        assert_eq!(
            SenseVoiceEngine::postprocess("<|BGM|><|Speech|>music talking"),
            Some(("music talking".into(), "auto".into()))
        );
        // 纯标签 → None
        assert_eq!(SenseVoiceEngine::postprocess("<|BGM|>"), None);
        assert_eq!(SenseVoiceEngine::postprocess("   "), None);
        // 首命中按 LANG_MAP 迭代序（zh,en,ja,ko,yue）而非文本位置——
        // 原版 Python dict.items() 同序：含 <|en|> 与 <|ja|> 时命中的是 en
        assert_eq!(
            SenseVoiceEngine::postprocess("<|ja|>テスト<|en|>test"),
            Some(("テストtest".into(), "en".into()))
        );
    }

    #[test]
    fn load_error_when_dir_missing() {
        let err = SenseVoiceEngine::load(Path::new("Z:/no/such/dir"), None, "auto");
        assert!(matches!(err, Err(EngineError::Load(_))));
    }
}
