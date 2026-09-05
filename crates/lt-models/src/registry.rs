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
    /// 体积估计（进度条/半体积阈值用）
    pub estimated_bytes: u64,
    /// 完整性所需的核心文件（缓存探测用）
    pub files: &'static [&'static str],
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
};

/// funasr-nano（M5 实装；注册表占位）
pub const FUNASR_NANO: ModelEntry = ModelEntry {
    key: "funasr-nano-2512",
    display: "Fun-ASR-Nano",
    hf: Some("csukuangfj/sherpa-onnx-funasr-nano-2512-zh-cantonese-en-ja-ko"),
    ms: Some("csukuangfj/sherpa-onnx-funasr-nano-2512-zh-cantonese-en-ja-ko"),
    always_hf: false,
    estimated_bytes: 1_100_000_000,
    files: &["model.int8.onnx", "tokens.txt"],
};

/// whisper 各档：repo = ggml-org/whisper-{size}（turbo 独立 repo）
pub fn whisper_repo(size: &str) -> Option<&'static str> {    match size {
        "tiny" => Some("ggml-org/whisper-tiny"),
        "base" => Some("ggml-org/whisper-base"),
        "small" => Some("ggml-org/whisper-small"),
        "medium" => Some("ggml-org/whisper-medium"),
        "large-v3" => Some("ggml-org/whisper-large-v3"),
        "turbo" => Some("ggml-org/whisper-large-v3-turbo"),
        _ => None,
    }
}

/// whisper ggml 文件名（q5_0 默认档）【M5 首日核对】
pub fn whisper_ggml_file(size: &str) -> Option<&'static str> {
    match size {
        "tiny" => Some("ggml-tiny-q5_0.bin"),
        "base" => Some("ggml-base-q5_0.bin"),
        "small" => Some("ggml-small-q5_0.bin"),
        "medium" => Some("ggml-medium-q5_0.bin"),
        "large-v3" => Some("ggml-large-v3-q5_0.bin"),
        "turbo" => Some("ggml-large-v3-turbo-q5_0.bin"),
        _ => None,
    }
}

/// whisper 静态表（always HF；q5_0 默认档）
pub const WHISPER_ENTRIES: [ModelEntry; 6] = [
    ModelEntry { key: "tiny", display: "tiny", hf: Some("ggml-org/whisper-tiny"), ms: None, always_hf: true, estimated_bytes: 78_000_000, files: &["ggml-tiny-q5_0.bin"] },
    ModelEntry { key: "base", display: "base", hf: Some("ggml-org/whisper-base"), ms: None, always_hf: true, estimated_bytes: 148_000_000, files: &["ggml-base-q5_0.bin"] },
    ModelEntry { key: "small", display: "small", hf: Some("ggml-org/whisper-small"), ms: None, always_hf: true, estimated_bytes: 488_000_000, files: &["ggml-small-q5_0.bin"] },
    ModelEntry { key: "medium", display: "medium", hf: Some("ggml-org/whisper-medium"), ms: None, always_hf: true, estimated_bytes: 1_530_000_000, files: &["ggml-medium-q5_0.bin"] },
    ModelEntry { key: "large-v3", display: "large-v3", hf: Some("ggml-org/whisper-large-v3"), ms: None, always_hf: true, estimated_bytes: 3_100_000_000, files: &["ggml-large-v3-q5_0.bin"] },
    ModelEntry { key: "turbo", display: "turbo (快档)", hf: Some("ggml-org/whisper-large-v3-turbo"), ms: None, always_hf: true, estimated_bytes: 809_000_000, files: &["ggml-large-v3-turbo-q5_0.bin"] },
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
        assert!(!SENSEVOICE_SMALL.always_hf);
        assert!(SENSEVOICE_SMALL.files.contains(&"model.int8.onnx"));
    }

    #[test]
    fn whisper_always_hf_with_files() {
        for size in ["tiny", "base", "small", "medium", "large-v3", "turbo"] {
            let e = whisper_entry_for(size).expect(size);
            assert!(e.always_hf);
            assert!(e.ms.is_none());
            assert_eq!(e.files.len(), 1);
            assert!(e.files[0].contains("q5_0"));
        }
        assert!(whisper_entry_for("custom-path").is_none());
    }

    #[test]
    fn funasr_entry_keys() {
        assert!(funasr_entry("sensevoice-small").is_some());
        assert!(funasr_entry("funasr-nano-2512").is_some());
        // mlt 无上游 ONNX 转换（D-14）→ None，运行时回退 sensevoice-small
        assert!(funasr_entry("funasr-mlt-nano-2512").is_none());
        assert!(funasr_entry("bogus").is_none());
    }
}
