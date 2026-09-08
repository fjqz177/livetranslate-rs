//! Fun-ASR-Nano 引擎（sherpa-onnx 离线识别；原版 asr_funasr_nano.py 的 Rust 等价）。
//!
//! 后处理 1:1：正则清除全部 `<|...|>` 标签 → trim → 空文本或 `"sil"` 视为无结果；
//! 语言判定：设值优先（sherpa 创建期参数，"auto"=不传），否则启发式 LID
//! （假名>0→ja；谚文>30%→ko；汉字>30%→zh；否则 en；空→auto）。
//! nano 无 padding 语义（原版 funasr_supports_padding: nano=false）——
//! `set_input_padding` 保持 trait 默认 `Unsupported`，manager 已按引擎名跳过下发。

use super::{
    describe_model_files, guess_language, normalize_language, sherpa_create_failure_hint,
    strip_special_tags,
};
use crate::engine::AsrEngine;
use lt_proto::{AsrResult, EngineError};
use sherpa_onnx::{OfflineFunASRNanoModelConfig, OfflineRecognizer, OfflineRecognizerConfig};
use std::path::{Path, PathBuf};

pub const SAMPLE_RATE: u32 = 16000;

/// 官方推荐参数（sherpa 文档页 CLI dump，2026-09-08 核对）。
/// 注意 binding `Default` 不可裸用（max_new_tokens=0/temperature=1.0/top_p=1.0）。
const OFFICIAL_MAX_NEW_TOKENS: i32 = 512;
const OFFICIAL_TEMPERATURE: f32 = 1e-6;
const OFFICIAL_TOP_P: f32 = 0.8;
const OFFICIAL_SEED: i32 = 42;
const OFFICIAL_SYSTEM_PROMPT: &str = "You are a helpful assistant.";
const OFFICIAL_USER_PROMPT: &str = "语音转写：";
/// 官方 RTF 基准线程数（0.144–0.191@2线程；AR 自回归解码受益于多线程，
/// 与 sensevoice 的 1 线程策略不同）
const NUM_THREADS: i32 = 2;

/// 三 onnx 主件 + tokenizer 目录（官方包根布局；下载清单 = 注册表 files）
const ONNX_FILES: [&str; 3] = [
    "embedding.int8.onnx",
    "encoder_adaptor.int8.onnx",
    "llm.int8.onnx",
];
const TOKENIZER_DIR: &str = "Qwen3-0.6B";

pub struct NanoEngine {
    recognizer: OfflineRecognizer,
    /// 模型目录（set_language 重建识别器用）
    model_dir: PathBuf,
    /// None = auto；变更时懒重建识别器（sherpa 语言是创建期参数）
    language: Option<String>,
}

impl NanoEngine {
    /// 从模型目录加载：三 onnx + `Qwen3-0.6B/` tokenizer 目录（缺一即报错，
    /// 不让 sherpa 半路失败——缓存探测 manifest 与本清单同源）
    pub fn load(model_dir: &Path, language: &str) -> Result<Self, EngineError> {
        for name in ONNX_FILES {
            let p = model_dir.join(name);
            if !p.is_file() {
                return Err(EngineError::Load(format!(
                    "nano 模型文件缺失: {}（于 {}）",
                    name,
                    model_dir.display()
                )));
            }
        }
        let tok = model_dir.join(TOKENIZER_DIR);
        if !tok.is_dir() {
            return Err(EngineError::Load(format!(
                "nano tokenizer 目录缺失: {}",
                tok.display()
            )));
        }
        let language = normalize_language(language);
        let recognizer = build_recognizer(model_dir, language.clone())?;
        tracing::info!(
            "Fun-ASR-Nano loaded: {} (language={})",
            model_dir.display(),
            language.as_deref().unwrap_or("auto")
        );
        Ok(Self {
            recognizer,
            model_dir: model_dir.to_path_buf(),
            language,
        })
    }

    /// 后处理（原版 1:1）：清全部 `<|...|>` 标签 → trim → 空或 "sil" → None
    fn postprocess(raw: &str) -> Option<String> {
        let out = strip_special_tags(raw);
        let out = out.trim();
        if out.is_empty() || out == "sil" {
            None
        } else {
            Some(out.to_string())
        }
    }
}

fn build_recognizer(
    model_dir: &Path,
    language: Option<String>,
) -> Result<OfflineRecognizer, EngineError> {
    let join = |name: &str| model_dir.join(name).to_string_lossy().into_owned();
    let mut cfg = OfflineRecognizerConfig::default();
    cfg.model_config.funasr_nano = OfflineFunASRNanoModelConfig {
        encoder_adaptor: Some(join("encoder_adaptor.int8.onnx")),
        llm: Some(join("llm.int8.onnx")),
        embedding: Some(join("embedding.int8.onnx")),
        tokenizer: Some(join(TOKENIZER_DIR)),
        system_prompt: Some(OFFICIAL_SYSTEM_PROMPT.into()),
        user_prompt: Some(OFFICIAL_USER_PROMPT.into()),
        max_new_tokens: OFFICIAL_MAX_NEW_TOKENS,
        temperature: OFFICIAL_TEMPERATURE,
        top_p: OFFICIAL_TOP_P,
        seed: OFFICIAL_SEED,
        language,
        itn: 0,
        hotwords: None,
    };
    cfg.model_config.num_threads = NUM_THREADS;
    OfflineRecognizer::create(&cfg).ok_or_else(|| {
        EngineError::Load(format!(
            "sherpa 创建识别器失败（model={}；文件 [{}]，tokenizer 目录 {}）——{}",
            model_dir.display(),
            describe_model_files(model_dir, &ONNX_FILES),
            if model_dir.join(TOKENIZER_DIR).is_dir() {
                "在"
            } else {
                "缺"
            },
            sherpa_create_failure_hint(),
        ))
    })
}

impl AsrEngine for NanoEngine {
    fn transcribe(
        &mut self,
        audio: &[f32],
        _word_timestamps: bool,
    ) -> Result<AsrResult, EngineError> {
        if audio.is_empty() {
            return Ok(AsrResult::default());
        }
        // nano 无 pad 桶（原版 nano 不做 padding；build_worker_config 对 nano 恒 None）
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE as i32, audio);
        self.recognizer.decode(&stream);
        let Some(result) = stream.get_result() else {
            return Ok(AsrResult::default());
        };

        match Self::postprocess(&result.text) {
            None => Ok(AsrResult::default()),
            Some(text) => {
                // 原版：设值优先，否则启发式 LID；language_name = language
                let language = self
                    .language
                    .clone()
                    .unwrap_or_else(|| guess_language(&text));
                Ok(AsrResult {
                    text,
                    language: language.clone(),
                    language_name: language,
                    words: None,
                })
            }
        }
    }

    fn set_language(&mut self, language: &str) -> Result<(), EngineError> {
        // 创建期参数 → 变更即重建识别器（原版每次 generate 传参，语义等价；
        // 切换很少见，重建成本可接受）
        let new_lang = normalize_language(language);
        if new_lang != self.language {
            self.recognizer = build_recognizer(&self.model_dir, new_lang.clone())?;
            self.language = new_lang;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postprocess_strips_tags_sil_and_empty() {
        assert_eq!(
            NanoEngine::postprocess("<|zh|>你好世界"),
            Some("你好世界".into())
        );
        assert_eq!(
            NanoEngine::postprocess("hello <|HAPPY|>world<|BGM|>"),
            Some("hello world".into())
        );
        assert_eq!(
            NanoEngine::postprocess("  你好，世界。 "),
            Some("你好，世界。".into())
        );
        // 纯标签 / 空 / "sil" → None（原版三无结果语义）
        assert_eq!(NanoEngine::postprocess("<|BGM|><|Speech|>"), None);
        assert_eq!(NanoEngine::postprocess("   "), None);
        assert_eq!(NanoEngine::postprocess("sil"), None);
        // 未闭合标签：保留原样（与 sensevoice 同策略）
        assert_eq!(NanoEngine::postprocess("abc<|def"), Some("abc<|def".into()));
    }

    // 启发式 LID 边界测试随 guess_language 提升至 engines/mod.rs（共享面）

    #[test]
    fn load_error_when_files_missing() {
        let err = NanoEngine::load(Path::new("Z:/no/such/dir"), "auto");
        assert!(matches!(err, Err(EngineError::Load(_))));
    }
}

/// WP-A 验收探针（临时，不入常规测试面）：真实缓存加载 + 仓内 test_wavs 转写。
/// 运行：cargo test -p lt-asr probe_real_nano -- --ignored --nocapture
#[cfg(test)]
mod probe_real_nano_tmp {
    use super::*;
    use crate::engines::probe_models_root;
    use std::time::Instant;

    #[test]
    #[ignore = "真实模型加载（963MB 包）+ test_wavs 转写；WP-A 引擎级验收用"]
    fn probe_real_nano_transcribe() {
        let md = probe_models_root().join(
            "huggingface/hub/models--csukuangfj--sherpa-onnx-funasr-nano-int8-2025-12-30/snapshots/main",
        );
        if !md.is_dir() {
            println!(
                "跳过（模型未缓存；设 LIVETRANSLATE_CONFIG_DIR 指向含 models 的配置目录）: {}",
                md.display()
            );
            return;
        }
        let t0 = Instant::now();
        let mut eng = NanoEngine::load(&md, "auto").expect("nano 加载失败");
        println!("加载耗时 {:?}", t0.elapsed());

        // test_wavs 由验收脚本预先 curl 到快照旁（不进 manifest 清单）
        let wavs = [
            ("test_wavs/rag_physics.wav", "zh"), // 普通话领域词
            ("test_wavs/noise_en.wav", "en"),    // 带噪英文
            ("test_wavs/dia_yue.wav", "zh"),     // 粤语口语，书面转写以汉字为主 → LID 判 zh 属预期
        ];
        for (rel, expect_lang) in wavs {
            let p = md.join(rel);
            if !p.is_file() {
                println!("跳过（未下载）: {rel}");
                continue;
            }
            let Some(wave) = sherpa_onnx::Wave::read(&p.to_string_lossy()) else {
                panic!("wav 读取失败: {rel}");
            };
            assert_eq!(wave.sample_rate(), SAMPLE_RATE as i32, "{rel} 采样率");
            let t1 = Instant::now();
            let r = eng.transcribe(wave.samples(), false).expect("transcribe");
            let rtf = t1.elapsed().as_secs_f32() / wave.samples().len() as f32 * SAMPLE_RATE as f32;
            println!(
                "{rel}: {:?} → text={:?} lang={} (RTF {:.3})",
                t1.elapsed(),
                r.text,
                r.language,
                rtf
            );
            assert!(!r.text.is_empty(), "{rel} 转写为空");
            assert_eq!(r.language, expect_lang, "{rel} 语种判定");
        }
    }
}
