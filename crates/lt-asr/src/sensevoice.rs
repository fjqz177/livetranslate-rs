//! SenseVoice 引擎（sherpa-onnx 离线识别；原版 asr_sensevoice.py 的 Rust 等价）。
//!
//! 后处理 1:1：首命中语言标签定语言并移除 → 正则清除全部 `<|...|>` 标签
//! （情感/事件/BGM）→ trim → 空文本视为无结果。
//!
//! D-30（2026-09-08 实机取证）：sherpa-onnx 转换包（csukuangfj 的
//! sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17）解码输出**不含任何
//! `<|...|>` 标签**（auto 与显式语言均无）——标签提取分支是实际死代码，
//! 恒回 "auto" 会让下游「同语言免翻译」失灵、显式语言被过滤整段丢弃。
//! 语言最终由 [`resolve_language`] 三优先决策：显式设置 > 模型标签 > 启发式。

use crate::engine::AsrEngine;
use crate::engines::{describe_model_files, guess_language, sherpa_create_failure_hint};
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

    /// 语言标签剥离（§5.2；单测覆盖）。
    /// 返回 `(text, tagged)`：tagged 为模型输出标签检出值，未命中恒 "auto"——
    /// **"auto" 只是"未检出"，不是语言结论**；最终语言由 [`resolve_language`]
    /// 三优先决策（D-30：sherpa-onnx 输出确实无标签，标签分支仅作未来模型变体保留）。
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

/// 语言最终决策（D-30 三优先）：**显式设置 > 模型标签 > 启发式兜底**。
/// - 显式设置优先：下游语言过滤（pipeline `reject_segment`/`commit_text` 的
///   `detected_lang != asr_language`）要求语言字段与设置自洽，否则显式语言时
///   一切段被误丢（本缺陷的另一面，9-8 实机确认 sherpa 输出无标签→恒 "auto"）；
/// - 模型标签次之：postprocess 剥离出的 `<|zh|>` 等（当前转换包不产出，为未来
///   模型变体保留——标签路径可区分 yue/zh）；
/// - 启发式兜底：与 nano/qwen3 共享的字符统计 [`guess_language`]——保证非空
///   文本恒有 ISO 码，auto 模式下下游「同语言免翻译」才能触发。
fn resolve_language(setting: Option<&str>, tagged: &str, text: &str) -> String {
    match setting {
        Some(lang) => lang.to_string(),
        None if tagged != "auto" => tagged.to_string(),
        None => guess_language(text),
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
    OfflineRecognizer::create(&cfg).ok_or_else(|| {
        EngineError::Load(format!(
            "sherpa 创建识别器失败（model={}；文件 [{}]）——{}",
            model.display(),
            describe_model_files(
                model.parent().unwrap_or(model),
                &[model.file_name().and_then(|s| s.to_str()).unwrap_or("model.onnx"), "tokens.txt"],
            ),
            sherpa_create_failure_hint(),
        ))
    })
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
        let audio: &[f32] = if self.pad_quantum > 0 && !audio.len().is_multiple_of(self.pad_quantum) {
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
            Some((text, tagged)) => {
                // D-30：语言三优先——sherpa 输出无标签时 tagged 恒 "auto"，
                // 恒 "auto" 会令下游同语言免翻译失灵（用户主诉）与显式语言
                // 整段误丢，故此处必须落到 ISO 码或显式设置值
                let language = resolve_language(self.language.as_deref(), &tagged, &text);
                Ok(AsrResult {
                    text,
                    language: language.clone(),
                    language_name: language, // 原版：language_name = language（不查显示名表）
                    words: None,
                })
            }
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
        // 无语言标签但含事件标签——postprocess 无标签即回 "auto"（"未检出"，
        // 最终语言由 resolve_language 三优先决策，见下方单测）
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
    fn resolve_language_three_way_priority() {
        // ① 显式设置优先——与下游语言过滤自洽是硬要求（否则显式 yue/zh 会被
        //    `detected_lang != asr_language` 整段误丢，D-30 隐性缺陷）
        assert_eq!(resolve_language(Some("yue"), "auto", "根据碰撞理论"), "yue");
        assert_eq!(resolve_language(Some("zh"), "en", "hello world"), "zh");
        assert_eq!(resolve_language(Some("zh"), "yue", "根据碰撞理论"), "zh");
        // ② auto + 标签命中：标签优先于启发式（模型 LID 更准，保留 yue/zh 区分力）
        assert_eq!(resolve_language(None, "yue", "根据碰撞理论"), "yue");
        assert_eq!(resolve_language(None, "zh", "你好世界"), "zh");
        // ③ auto + 无标签：共享启发式兜底（nano/qwen3 同款边界语义）
        assert_eq!(resolve_language(None, "auto", "你好世界"), "zh");
        assert_eq!(resolve_language(None, "auto", "hello world"), "en");
        assert_eq!(resolve_language(None, "auto", "テスト"), "ja");
        assert_eq!(resolve_language(None, "auto", "한국어 한국어 abc"), "ko");
        // 兜底对空文本的防御语义（实际经 postprocess → None 短路，不达此处）
        assert_eq!(resolve_language(None, "auto", ""), "auto");
    }

    #[test]
    fn load_error_when_dir_missing() {
        let err = SenseVoiceEngine::load(Path::new("Z:/no/such/dir"), None, "auto");
        assert!(matches!(err, Err(EngineError::Load(_))));
    }
}

/// D-30 验收探针（临时，不入常规测试面）：真实缓存模型 + test_wavs 转写，
/// 走 `transcribe` 完整路径断言语言字段——auto 下不再恒 "auto"（启发式兜底），
/// 显式设置下与设置自洽。
/// 运行：cargo test -p lt-asr probe_real_sensevoice -- --ignored --nocapture
#[cfg(test)]
mod probe_real_sensevoice_tmp {
    use super::*;
    use std::time::Instant;

    const SNAPSHOT: &str = "C:/Users/fjqz177/.config/livetranslate/models/modelscope/models/pengzhendong--sherpa-onnx-sense-voice-zh-en-ja-ko-yue/snapshots/master";
    const NANO_WAVS: &str = "C:/Users/fjqz177/.config/livetranslate/models/huggingface/hub/models--csukuangfj--sherpa-onnx-funasr-nano-int8-2025-12-30/snapshots/main/test_wavs";

    fn probe_one(eng: &mut SenseVoiceEngine, full: &str, expect: &str) {
        let p = Path::new(full);
        if !p.is_file() {
            println!("跳过（未下载）: {full}");
            return;
        }
        let Some(wave) = sherpa_onnx::Wave::read(full) else {
            panic!("wav 读取失败: {full}");
        };
        assert_eq!(wave.sample_rate(), SAMPLE_RATE as i32, "{full} 采样率");
        let t1 = Instant::now();
        let r = eng.transcribe(wave.samples(), false).expect("transcribe");
        println!(
            "{full}: {:?} text={:?} lang={}（期望 {expect}）",
            t1.elapsed(),
            r.text,
            r.language
        );
        assert!(!r.text.is_empty(), "{full} 转写为空");
        assert_eq!(r.language, expect, "{full} 语言判定");
    }

    #[test]
    #[ignore = "真实 SenseVoice 模型 + test_wavs；D-30 验收探针（一次性）"]
    fn probe_real_sensevoice_lang_decision() {
        let md = Path::new(SNAPSHOT);
        let mut eng = SenseVoiceEngine::load(md, Some(0.5), "auto").expect("sensevoice 加载失败");
        // auto：启发式兜底——修复前恒 "auto"（sherpa 无标签），修复后按文本判 ISO 码
        probe_one(&mut eng, &format!("{NANO_WAVS}/noise_en.wav"), "en");
        probe_one(&mut eng, &format!("{NANO_WAVS}/rag_physics.wav"), "zh");
        // 粤语启发式归 zh（D-30 已知取舍：启发式无 yue 档，与 nano 探针预期一致）
        probe_one(&mut eng, &format!("{NANO_WAVS}/dia_yue.wav"), "zh");
        // 显式设置优先：与语言过滤自洽（修复前显式 yue 时转写被整段过滤丢弃）
        for (lang, rel) in [
            ("zh", "rag_physics.wav"),
            ("en", "noise_en.wav"),
            ("yue", "dia_yue.wav"),
        ] {
            eng.set_language(lang).expect("set_language");
            probe_one(&mut eng, &format!("{NANO_WAVS}/{rel}"), lang);
        }
    }
}
