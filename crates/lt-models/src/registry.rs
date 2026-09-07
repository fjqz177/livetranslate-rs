//! 模型注册表（PLAN §3.4）：repo 映射、文件清单、体积估计。
//!
//! whisper 的 ggml 文件名映射【M5 首日核对仓内实际文件名】。

/// 单个模型条目
#[derive(Debug, Clone, PartialEq)]
pub struct ModelEntry {
    /// settings.funasr_model / whisper_model_size 中的键
    pub key: &'static str,
    pub display: &'static str,
    /// HuggingFace repo id（None = 无 HF 源）
    pub hf: Option<&'static str>,
    /// ModelScope repo id（None = 无 MS 源，always HF）
    pub ms: Option<&'static str>,
    /// 该模型必须从 HF 下载（MS 无对应仓库）
    pub always_hf: bool,
    /// 体积估计（进度条/「未缓存 ≈X」展示用；不参与完整性判定）
    pub estimated_bytes: u64,
    /// 完整性所需的核心文件（下载清单 = 缓存探测目标）
    pub files: &'static [&'static str],
    /// 每个清单文件的字节数下限（与 files 等长 zip；0 = 仅要求存在）。
    /// 探测语义（DL-1，docs/download-overhaul.md DEC-1）：manifest 逐文件
    /// 「存在 + len ≥ 下限」，取代原版体积阈值启发式——下限刻意取远低于
    /// 实际值（假阴性 = 多下一次，可自愈；假阳性 = 判已缓存却加载失败）。
    pub files_min_bytes: &'static [u64],
}

/// SenseVoice（sherpa-onnx 转换包：model.int8.onnx 239MB + tokens.txt）。
/// MS 侧 csukuangfj 未发布 → 用 pengzhendong 镜像仓（文件与 HF 官方包一致，M2 核对完毕）。
pub const SENSEVOICE_SMALL: ModelEntry = ModelEntry {
    key: "sensevoice-small",
    display: "SenseVoice Small",
    hf: Some("csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17"),
    ms: Some("pengzhendong/sherpa-onnx-sense-voice-zh-en-ja-ko-yue"),
    always_hf: false,
    estimated_bytes: 250_000_000,
    files: &["model.int8.onnx", "tokens.txt"],
    files_min_bytes: &[50_000_000, 1_024],
};

/// funasr-nano（WP-A 实装）。【2026-09-08 核实】sherpa-onnx 官方 int8 包，
/// 仓/清单/字节数经 HF API 实测（docs/asr-engine-expansion.md §2.2）。
/// D-24：MS 无官方单仓 → ms=None 诚实留空（禁止伪造 ms 字段），always_hf=true，
/// 国内经 hf-mirror 镜像下载（download::hf_endpoint_for）。
/// files_min_bytes 刻意远低于实测（假阴性=多下一次可自愈；假阳性=判已缓存却加载失败）。
pub const FUNASR_NANO: ModelEntry = ModelEntry {
    key: "funasr-nano-2512",
    display: "Fun-ASR-Nano",
    hf: Some("csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30"),
    ms: None,
    always_hf: true,
    estimated_bytes: 1_050_000_000,
    files: &[
        "embedding.int8.onnx",
        "encoder_adaptor.int8.onnx",
        "llm.int8.onnx",
        "Qwen3-0.6B/merges.txt",
        "Qwen3-0.6B/tokenizer.json",
        "Qwen3-0.6B/vocab.json",
    ],
    files_min_bytes: &[100_000_000, 150_000_000, 300_000_000, 1_000_000, 5_000_000, 1_000_000],
};

/// whisper 各档统一 HF repo。
/// 【M5 首日核实】PLAN 预估的 ggml-org/whisper-{size} 六仓已全部转为私有
/// （API 返回 401，resolve 端点同）；whisper.cpp 官方模型仓是单仓
/// `ggerganov/whisper.cpp`（六档文件齐全，HF/hf-mirror 均公开可下）。
pub fn whisper_repo(size: &str) -> Option<&'static str> {
    match size {
        "tiny" | "base" | "small" | "medium" | "large-v3" | "turbo" => {
            Some("ggerganov/whisper.cpp")
        }
        _ => None,
    }
}

/// whisper ggml 文件名（量化默认档）。
/// 【M5 首日核实】ggerganov/whisper.cpp 仓内实际：tiny/base/small 只有
/// q5_1 量化（无 q5_0，会 404）；medium/large-v3/turbo 有 q5_0。
/// 与 whisper.cpp 官方 download-ggml-model 脚本的量化档位一致。
pub fn whisper_ggml_file(size: &str) -> Option<&'static str> {
    match size {
        "tiny" => Some("ggml-tiny-q5_1.bin"),
        "base" => Some("ggml-base-q5_1.bin"),
        "small" => Some("ggml-small-q5_1.bin"),
        "medium" => Some("ggml-medium-q5_0.bin"),
        "large-v3" => Some("ggml-large-v3-q5_0.bin"),
        "turbo" => Some("ggml-large-v3-turbo-q5_0.bin"),
        _ => None,
    }
}

/// whisper 静态表（always HF；量化默认档）。
/// estimated_bytes = 仓内实际文件字节数（HF API tree/main 实测，2026-09-06）；
/// files_min_bytes = 实测半体积（完整下载必过阈）。
pub const WHISPER_ENTRIES: [ModelEntry; 6] = [
    ModelEntry { key: "tiny", display: "tiny", hf: Some("ggerganov/whisper.cpp"), ms: None, always_hf: true, estimated_bytes: 32_152_673, files: &["ggml-tiny-q5_1.bin"], files_min_bytes: &[16_076_336] },
    ModelEntry { key: "base", display: "base", hf: Some("ggerganov/whisper.cpp"), ms: None, always_hf: true, estimated_bytes: 59_707_625, files: &["ggml-base-q5_1.bin"], files_min_bytes: &[29_853_812] },
    ModelEntry { key: "small", display: "small", hf: Some("ggerganov/whisper.cpp"), ms: None, always_hf: true, estimated_bytes: 190_085_487, files: &["ggml-small-q5_1.bin"], files_min_bytes: &[95_042_743] },
    ModelEntry { key: "medium", display: "medium", hf: Some("ggerganov/whisper.cpp"), ms: None, always_hf: true, estimated_bytes: 539_212_467, files: &["ggml-medium-q5_0.bin"], files_min_bytes: &[269_606_233] },
    ModelEntry { key: "large-v3", display: "large-v3", hf: Some("ggerganov/whisper.cpp"), ms: None, always_hf: true, estimated_bytes: 1_081_140_203, files: &["ggml-large-v3-q5_0.bin"], files_min_bytes: &[540_570_101] },
    ModelEntry { key: "turbo", display: "turbo", hf: Some("ggerganov/whisper.cpp"), ms: None, always_hf: true, estimated_bytes: 574_041_195, files: &["ggml-large-v3-turbo-q5_0.bin"], files_min_bytes: &[287_020_597] },
];

/// 合法 funasr 模型键（与 lt_proto::FUNASR_MODELS 对齐）
pub const FUNASR_KEYS: [&str; 3] = ["sensevoice-small", "funasr-nano-2512", "funasr-mlt-nano-2512"];

/// 按键取 funasr 条目；None = 该键当前不可用
pub fn funasr_entry(key: &str) -> Option<ModelEntry> {
    match key {
        "sensevoice-small" => Some(SENSEVOICE_SMALL.clone()),
        "funasr-nano-2512" => Some(FUNASR_NANO.clone()),
        // mlt 是独立模型，无上游 ONNX 转换，待上游产出（D-14）；不得用 nano 冒充
        "funasr-mlt-nano-2512" => None,
        _ => None,
    }
}

/// whisper 条目（size 为合法档位时）
pub fn whisper_entry_for(size: &str) -> Option<ModelEntry> {
    WHISPER_ENTRIES.iter().find(|e| e.key == size).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensevoice_dual_hub() {
        assert_eq!(
            SENSEVOICE_SMALL.hf,
            Some("csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17")
        );
        // MS 侧无 csukuangfj sherpa 包 → pengzhendong 镜像（M2 实测核对）
        assert_eq!(
            SENSEVOICE_SMALL.ms,
            Some("pengzhendong/sherpa-onnx-sense-voice-zh-en-ja-ko-yue")
        );
        const { assert!(!SENSEVOICE_SMALL.always_hf) } // 编译期注册表不变量
        assert!(SENSEVOICE_SMALL.files.contains(&"model.int8.onnx"));
    }

    #[test]
    fn whisper_always_hf_with_files() {
        for size in ["tiny", "base", "small", "medium", "large-v3", "turbo"] {
            let e = whisper_entry_for(size).expect(size);
            assert!(e.always_hf);
            assert!(e.ms.is_none());
            // 【M5 核实】ggml-org/whisper-* 六仓已私有 → 统一官方单仓
            assert_eq!(e.hf, Some("ggerganov/whisper.cpp"));
            assert_eq!(e.files.len(), 1);
            assert!(e.files[0].contains("q5_"));
        }
        assert!(whisper_entry_for("custom-path").is_none());
    }

    #[test]
    fn whisper_ggml_file_mapping_matches_repo_reality() {
        // 【M5 首日核实】ggerganov/whisper.cpp 仓内实际文件名：
        // tiny/base/small 只有 q5_1（q5_0 会 404）；medium/large-v3/turbo 有 q5_0
        assert_eq!(whisper_ggml_file("tiny"), Some("ggml-tiny-q5_1.bin"));
        assert_eq!(whisper_ggml_file("base"), Some("ggml-base-q5_1.bin"));
        assert_eq!(whisper_ggml_file("small"), Some("ggml-small-q5_1.bin"));
        assert_eq!(whisper_ggml_file("medium"), Some("ggml-medium-q5_0.bin"));
        assert_eq!(whisper_ggml_file("large-v3"), Some("ggml-large-v3-q5_0.bin"));
        assert_eq!(whisper_ggml_file("turbo"), Some("ggml-large-v3-turbo-q5_0.bin"));
        assert_eq!(whisper_ggml_file("nope"), None);
        // files 与映射函数必须一致（下载清单=探测目标）
        for size in ["tiny", "base", "small", "medium", "large-v3", "turbo"] {
            let e = whisper_entry_for(size).expect(size);
            assert_eq!(e.files, &[whisper_ggml_file(size).expect(size)]);
            // estimated_bytes 取仓内实际字节数（半体积阈值依赖）：
            // 必须超过 funasr 通用的 50MB 下限感知不到的小文件，且足够大以区分截断
            assert!(e.estimated_bytes >= 30_000_000, "{size}");
        }
        assert_eq!(whisper_repo("tiny"), whisper_repo("turbo"));
    }

    #[test]
    fn funasr_entry_keys() {
        assert!(funasr_entry("sensevoice-small").is_some());
        assert!(funasr_entry("funasr-nano-2512").is_some());
        // mlt 无上游 ONNX 转换（D-14）→ None，运行时回退 sensevoice-small
        assert!(funasr_entry("funasr-mlt-nano-2512").is_none());
        assert!(funasr_entry("bogus").is_none());
    }

    #[test]
    fn funasr_nano_entry_hf_only() {
        // WP-A/D-24：真实仓（HF API 实测存在）；无 MS 源 → ms=None 诚实留空
        assert_eq!(
            FUNASR_NANO.hf,
            Some("csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30")
        );
        assert!(FUNASR_NANO.ms.is_none());
        const { assert!(FUNASR_NANO.always_hf) } // 编译期不变量：仅 HF 源
        // 六件套（三 onnx + Qwen3-0.6B tokenizer 子目录）；实测合计 963MB + 余量
        assert_eq!(FUNASR_NANO.files.len(), 6);
        assert_eq!(FUNASR_NANO.files_min_bytes.len(), 6);
        assert!(FUNASR_NANO.files.iter().any(|f| f.starts_with("Qwen3-0.6B/")));
        assert_eq!(FUNASR_NANO.estimated_bytes, 1_050_000_000);
    }

    #[test]
    fn manifest_min_bytes_parallel_to_files() {
        // DL-1⑥：files 与 files_min_bytes 必须等长（探测 zip 依赖）；
        // whisper 下限 = 实测半体积；funasr 下限远低于实际（防假阴性重下循环）
        for e in WHISPER_ENTRIES.iter() {
            assert_eq!(e.files.len(), e.files_min_bytes.len(), "{}", e.key);
            assert_eq!(e.files_min_bytes[0], e.estimated_bytes / 2, "{}", e.key);
        }
        for e in [SENSEVOICE_SMALL.clone(), FUNASR_NANO.clone()] {
            assert_eq!(e.files.len(), e.files_min_bytes.len(), "{}", e.key);
            assert!(e.files_min_bytes[0] >= 1 && e.files_min_bytes[1] >= 1, "{}", e.key);
            // 主文件下限必须远低于估计体积（下限是"远未完成"防线，不是完整性度量）
            assert!(e.files_min_bytes[0] * 2 < e.estimated_bytes, "{}", e.key);
        }
    }
}
