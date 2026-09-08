//! Qwen3-ASR 引擎（sherpa-onnx 离线识别；WP-B，docs/archive/asr-engine-expansion.md §4）。
//!
//! 与 nano 的差异（§4.3）：无 language/prompt 字段——纯 auto-LID，`set_language`
//! 对非 auto 诚实返回 `Unsupported`（auto 放行 no-op，免挂起命令噪音）；
//! tokenizer 目录为 `tokenizer/`；多 `max_total_len`（VAD 分段默认 ≤8s，远低于边界）。
//! 官方样例输出为纯文本（无 `<|...|>` 标签）——保留防御性标签清理；
//! `AsrResult.language` 用共享启发式 LID 兜底（`super::guess_language`）。

use super::{describe_model_files, guess_language, sherpa_create_failure_hint, strip_special_tags};
use crate::engine::AsrEngine;
use lt_proto::{AsrResult, EngineError};
use sherpa_onnx::{OfflineQwen3ASRModelConfig, OfflineRecognizer, OfflineRecognizerConfig};
use std::path::Path;

pub const SAMPLE_RATE: u32 = 16000;

/// 官方推荐参数（sherpa 文档页 qwen3-asr/pretrained.html CLI dump，2026-09-08 核对）。
/// 注意 binding `Default` 不可裸用：max_new_tokens 默认 128（官方 512）。
const MAX_TOTAL_LEN: i32 = 512;
const OFFICIAL_MAX_NEW_TOKENS: i32 = 512;
const OFFICIAL_TEMPERATURE: f32 = 1e-6;
const OFFICIAL_TOP_P: f32 = 0.8;
const OFFICIAL_SEED: i32 = 42;
/// S0 spike 本机实测择优（cantonese.wav：1/2/3 线程 → RTF 0.459/0.312/0.280，
/// 单调改善；官方 mic 示例亦用 3）
const NUM_THREADS: i32 = 3;

/// 三 onnx 主件 + tokenizer 目录（官方包根布局；下载清单 = 注册表 files）
const ONNX_FILES: [&str; 3] = [
    "conv_frontend.onnx",
    "encoder.int8.onnx",
    "decoder.int8.onnx",
];
const TOKENIZER_DIR: &str = "tokenizer";

pub struct Qwen3AsrEngine {
    recognizer: OfflineRecognizer,
}

impl Qwen3AsrEngine {
    /// 从模型目录加载：三 onnx + `tokenizer/` 目录（缺一即报错，
    /// 不让 sherpa 半路失败——缓存探测 manifest 与本清单同源）
    pub fn load(model_dir: &Path) -> Result<Self, EngineError> {
        for name in ONNX_FILES {
            let p = model_dir.join(name);
            if !p.is_file() {
                return Err(EngineError::Load(format!(
                    "qwen3 模型文件缺失: {}（于 {}）",
                    name,
                    model_dir.display()
                )));
            }
        }
        let tok = model_dir.join(TOKENIZER_DIR);
        if !tok.is_dir() {
            return Err(EngineError::Load(format!(
                "qwen3 tokenizer 目录缺失: {}",
                tok.display()
            )));
        }
        let recognizer = build_recognizer(model_dir, NUM_THREADS)?;
        tracing::info!("Qwen3-ASR-0.6B loaded: {}", model_dir.display());
        Ok(Self { recognizer })
    }

    /// 后处理：防御性清标签（官方输出预期纯文本）→ trim → 空文本 → None。
    /// qwen3 无 funasr 的 "sil" 语义，不做该判断。
    fn postprocess(raw: &str) -> Option<String> {
        let out = strip_special_tags(raw);
        let out = out.trim();
        if out.is_empty() {
            None
        } else {
            Some(out.to_string())
        }
    }
}

/// `set_language` 语义（纯 auto-LID，无 language 字段）：auto 族放行（no-op），
/// 显式语言拒绝。抽纯函数便于单测（引擎实例不可离线构造）。
fn language_auto(language: &str) -> bool {
    matches!(language, "" | "auto" | "Auto")
}

fn build_recognizer(model_dir: &Path, num_threads: i32) -> Result<OfflineRecognizer, EngineError> {
    let join = |name: &str| model_dir.join(name).to_string_lossy().into_owned();
    let mut cfg = OfflineRecognizerConfig::default();
    cfg.model_config.qwen3_asr = OfflineQwen3ASRModelConfig {
        conv_frontend: Some(join("conv_frontend.onnx")),
        encoder: Some(join("encoder.int8.onnx")),
        decoder: Some(join("decoder.int8.onnx")),
        tokenizer: Some(join(TOKENIZER_DIR)),
        max_total_len: MAX_TOTAL_LEN,
        max_new_tokens: OFFICIAL_MAX_NEW_TOKENS,
        temperature: OFFICIAL_TEMPERATURE,
        top_p: OFFICIAL_TOP_P,
        seed: OFFICIAL_SEED,
        hotwords: None,
    };
    cfg.model_config.num_threads = num_threads;
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

impl AsrEngine for Qwen3AsrEngine {
    fn transcribe(
        &mut self,
        audio: &[f32],
        _word_timestamps: bool,
    ) -> Result<AsrResult, EngineError> {
        if audio.is_empty() {
            return Ok(AsrResult::default());
        }
        // qwen3 无 pad 桶（build_worker_config 对 qwen3 恒 None）
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE as i32, audio);
        self.recognizer.decode(&stream);
        let Some(result) = stream.get_result() else {
            return Ok(AsrResult::default());
        };

        match Self::postprocess(&result.text) {
            None => Ok(AsrResult::default()),
            Some(text) => {
                // 纯 auto-LID 引擎：language 字段用共享启发式兜底（§4.3）
                let language = guess_language(&text);
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
        // 无语言参数：auto 值放行（no-op），显式语言诚实拒绝
        // （manager 对 Failed 仅 warn 一次并照常提交挂起，无重复噪音）
        if language_auto(language) {
            Ok(())
        } else {
            Err(EngineError::Unsupported)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postprocess_strips_tags_and_trims() {
        // 官方输出预期纯文本：直通
        assert_eq!(
            Qwen3AsrEngine::postprocess("你好，世界。"),
            Some("你好，世界。".into())
        );
        assert_eq!(
            Qwen3AsrEngine::postprocess("  hello world  "),
            Some("hello world".into())
        );
        // 防御性：万一出现标签也清理（与 nano 共享 strip）
        assert_eq!(
            Qwen3AsrEngine::postprocess("<|zh|>你好"),
            Some("你好".into())
        );
        // 空/纯空白 → None（worker 层语义：无结果）
        assert_eq!(Qwen3AsrEngine::postprocess(""), None);
        assert_eq!(Qwen3AsrEngine::postprocess("   "), None);
        // 纯标签 → None
        assert_eq!(Qwen3AsrEngine::postprocess("<|BGM|><|Speech|>"), None);
    }

    #[test]
    fn set_language_auto_only() {
        // auto 语义值放行（no-op）；显式语言：无 language 字段 → 诚实 Unsupported
        assert!(language_auto("auto"));
        assert!(language_auto(""));
        assert!(language_auto("Auto"));
        assert!(!language_auto("zh"));
        assert!(!language_auto("en"));
    }

    #[test]
    fn load_error_when_files_missing() {
        let err = Qwen3AsrEngine::load(Path::new("Z:/no/such/dir"));
        assert!(matches!(err, Err(EngineError::Load(_))));
        // 有 onnx 缺 tokenizer 目录同样报错
        let dir = std::env::temp_dir().join(format!("lt_qwen3_load_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ONNX_FILES {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let err = Qwen3AsrEngine::load(&dir);
        assert!(
            matches!(err, Err(EngineError::Load(_))),
            "缺 tokenizer 目录必须报错"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// WP-B S0/引擎级验收探针（临时，不入常规测试面）：真实缓存加载 +
/// 仓内 test_wavs 对照 transcript.txt + 线程数择优。
/// 运行：cargo test -p lt-asr probe_real_qwen3 -- --ignored --nocapture
#[cfg(test)]
mod probe_real_qwen3_tmp {
    use super::*;
    use std::time::Instant;

    const SNAPSHOT: &str = "C:/Users/fjqz177/.config/livetranslate/models/huggingface/hub/models--csukuangfj2--sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25/snapshots/main";

    /// 线性插值重采样（验收探针专用；test_wavs 混有 44.1kHz，
    /// 生产管线采集侧恒 16k 不经此路径）
    fn resample_to_16k(samples: &[f32], from: i32) -> Vec<f32> {
        if from == SAMPLE_RATE as i32 {
            return samples.to_vec();
        }
        let ratio = from as f64 / SAMPLE_RATE as f64;
        let out_len = (samples.len() as f64 / ratio).round() as usize;
        (0..out_len)
            .map(|i| {
                let src = i as f64 * ratio;
                let i0 = src as usize;
                let i1 = (i0 + 1).min(samples.len() - 1);
                let frac = src - i0 as f64;
                samples[i0] * (1.0 - frac) as f32 + samples[i1] * frac as f32
            })
            .collect()
    }

    #[test]
    #[ignore = "真实模型加载（941MB 包）+ test_wavs 转写；WP-B S0/引擎级验收用"]
    fn probe_real_qwen3_transcribe() {
        let md = Path::new(SNAPSHOT);
        let t0 = Instant::now();
        let mut eng = Qwen3AsrEngine::load(md).expect("qwen3 加载失败");
        println!("加载耗时 {:?}", t0.elapsed());

        // S0-④：线程数择优（1/2/3 各跑 cantonese 一条）
        for threads in [1, 2, 3] {
            let rec = build_recognizer(md, threads).expect("build");
            let mut e = Qwen3AsrEngine { recognizer: rec };
            let p = md.join("test_wavs/cantonese.wav");
            let wave = sherpa_onnx::Wave::read(&p.to_string_lossy()).expect("wav");
            let audio = resample_to_16k(wave.samples(), wave.sample_rate());
            let t1 = Instant::now();
            let r = e.transcribe(&audio, false).expect("transcribe");
            let rtf = t1.elapsed().as_secs_f32() / audio.len() as f32 * SAMPLE_RATE as f32;
            println!(
                "threads={threads}: RTF {:.3} len={}",
                rtf,
                r.text.chars().count()
            );
        }

        // 验收：中/粤/日/语码切换 对照 transcript.txt（文本人工比对打印，
        // 断言非空 + 启发式 LID 语种；粤语文本以汉字为主 → 判 zh 属预期）
        let wavs = [
            ("test_wavs/cantonese.wav", "zh"),
            ("test_wavs/raokouling.wav", "zh"),
            ("test_wavs/fast1.wav", "zh"),
            ("test_wavs/ja1.wav", "ja"),
            ("test_wavs/codeswitch.wav", "en"), // 英法意西混说，无假名/谚文/汉字 → 启发式 en
        ];
        for (rel, expect_lang) in wavs {
            let p = md.join(rel);
            let Some(wave) = sherpa_onnx::Wave::read(&p.to_string_lossy()) else {
                println!("跳过（未下载）: {rel}");
                continue;
            };
            let audio = resample_to_16k(wave.samples(), wave.sample_rate());
            let t1 = Instant::now();
            let r = eng.transcribe(&audio, false).expect("transcribe");
            let rtf = t1.elapsed().as_secs_f32() / audio.len() as f32 * SAMPLE_RATE as f32;
            println!(
                "{rel}: lang={} RTF {:.3}\n  text={:?}",
                r.language, rtf, r.text
            );
            assert!(!r.text.is_empty(), "{rel} 转写为空");
            assert_eq!(r.language, expect_lang, "{rel} 语种判定");
        }
    }
}
