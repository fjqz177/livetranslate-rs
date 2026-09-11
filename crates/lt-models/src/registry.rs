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
    /// 探测语义（DL-1，docs/archive/download-overhaul.md DEC-1）：manifest 逐文件
    /// 「存在 + len ≥ 下限」，取代原版体积阈值启发式——下限刻意取远低于
    /// 实际值（假阴性 = 多下一次，可自愈；假阳性 = 判已缓存却加载失败）。
    pub files_min_bytes: &'static [u64],
    /// 每个清单文件的 sha256（十六进制小写，与 files 等长 zip；AH-5/H8）。
    /// 空串 = 未登记，下载器跳过内容校验（当前全表已零空串，见
    /// `files_sha256_parallel_and_wellformed` 的硬不变量断言）。
    /// 已登记项在下载 finalize 前流式校验，不匹配按 Checksum 快速失败并删除
    /// `.incomplete`。来源：本地真实缓存 sha256sum 实测（2026-09-08；sherpa
    /// 三包 + whisper tiny），qwen3 decoder 前缀与
    /// docs/archive/asr-engine-expansion.md §4.2 记录交叉核对一致；
    /// whisper 其余五档 2026-09-10 经 HF LFS oid 补齐（tree API 与 resolve
    /// HEAD 的 `X-Linked-Etag` 双源一致，且与 tiny 的本地实测互证）。
    pub files_sha256: &'static [&'static str],
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
    // MS 侧 pengzhendong 镜像与 HF 官方包逐字节一致（M2 核对），哈希共用
    files_sha256: &[
        "c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51",
        "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc",
    ],
};

/// funasr-nano（WP-A 实装）。【2026-09-08 核实】sherpa-onnx 官方 int8 包，
/// 仓/清单/字节数经 HF API 实测（docs/archive/asr-engine-expansion.md §2.2）。
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
    files_min_bytes: &[
        100_000_000,
        150_000_000,
        300_000_000,
        1_000_000,
        5_000_000,
        1_000_000,
    ],
    files_sha256: &[
        "95e61cd0c9c3b9543339a4cf973c95c116815e745ccc1e0285cbd81f76d18644",
        "f36dea2e30fbc33b5db1d7a7265cc976c5e5586c77b042d5adb1ad27c72db422",
        "dfbf9aa3be41bccc257587f151e15c63fbe1b549f2b517f5ccd5bdce3bf4322a",
        "8831e4f1a044471340f7c0a83d7bd71306a5b867e95fd870f74d0c5308a904d5",
        "aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4",
        "ca10d7e9fb3ed18575dd1e277a2579c16d108e32f27439684afa0e10b1440910",
    ],
};

/// Qwen3-ASR-0.6B（WP-B 实装）。【2026-09-08 核实】sherpa-onnx 官方 int8 包，
/// 仓/清单/字节数经 HF API 实测（docs/archive/asr-engine-expansion.md §4.2）；
/// 注意作者是 csukuangfj**2**（`csukuangfj/` 同名仓不存在）。
/// D-24：MS 无官方单仓（csukuangfj2/csukuangfj 双 404）→ ms=None 诚实留空，
/// always_hf=true，国内经 hf-mirror 镜像下载（download::hf_endpoint_for）。
/// files_min_bytes 刻意远低于实测（假阴性=多下一次可自愈；假阳性=判已缓存却加载失败）。
pub const QWEN3_ASR: ModelEntry = ModelEntry {
    key: "qwen3-asr-0.6b",
    display: "Qwen3-ASR-0.6B",
    hf: Some("csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25"),
    ms: None,
    always_hf: true,
    estimated_bytes: 1_020_000_000,
    files: &[
        "conv_frontend.onnx",
        "encoder.int8.onnx",
        "decoder.int8.onnx",
        "tokenizer/merges.txt",
        "tokenizer/tokenizer_config.json",
        "tokenizer/vocab.json",
    ],
    files_min_bytes: &[
        20_000_000,
        90_000_000,
        350_000_000,
        1_000_000,
        5_000,
        1_000_000,
    ],
    // decoder 前缀 4f6885be5959ae26 与 docs/archive/asr-engine-expansion.md §4.2 记录一致
    files_sha256: &[
        "d22dc4423e0940e49884e903d2ea2f7e5567c14fc1aed97e4e26d6b8f208ef9e",
        "60748d3e6744a57c9c91e1b17424a6c2990567e8adceb0783940c03ed98fa9d9",
        "4f6885be5959ae26af3089d38ee7972c5fafbeeb1cf8d5e76eab6d8b61ca5771",
        "8831e4f1a044471340f7c0a83d7bd71306a5b867e95fd870f74d0c5308a904d5",
        "4942d005604266809309cabc9f4e9cb89ce855d59b14681fdc0e1cc62ea26c4c",
        "ca10d7e9fb3ed18575dd1e277a2579c16d108e32f27439684afa0e10b1440910",
    ],
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
/// files_min_bytes = 实测半体积（完整下载必过阈）；
/// files_sha256 = 仓内 LFS oid（= 文件内容 sha256；2026-09-10 补齐，lfs.oid
/// 与 resolve HEAD `X-Linked-Etag` 两路一致，六个档位字节数亦逐档吻合）。
pub const WHISPER_ENTRIES: [ModelEntry; 6] = [
    ModelEntry {
        key: "tiny",
        display: "tiny",
        hf: Some("ggerganov/whisper.cpp"),
        ms: None,
        always_hf: true,
        estimated_bytes: 32_152_673,
        files: &["ggml-tiny-q5_1.bin"],
        files_min_bytes: &[16_076_336],
        files_sha256: &["818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7"],
    },
    ModelEntry {
        key: "base",
        display: "base",
        hf: Some("ggerganov/whisper.cpp"),
        ms: None,
        always_hf: true,
        estimated_bytes: 59_707_625,
        files: &["ggml-base-q5_1.bin"],
        files_min_bytes: &[29_853_812],
        files_sha256: &["422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898"],
    },
    ModelEntry {
        key: "small",
        display: "small",
        hf: Some("ggerganov/whisper.cpp"),
        ms: None,
        always_hf: true,
        estimated_bytes: 190_085_487,
        files: &["ggml-small-q5_1.bin"],
        files_min_bytes: &[95_042_743],
        files_sha256: &["ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb"],
    },
    ModelEntry {
        key: "medium",
        display: "medium",
        hf: Some("ggerganov/whisper.cpp"),
        ms: None,
        always_hf: true,
        estimated_bytes: 539_212_467,
        files: &["ggml-medium-q5_0.bin"],
        files_min_bytes: &[269_606_233],
        files_sha256: &["19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f"],
    },
    ModelEntry {
        key: "large-v3",
        display: "large-v3",
        hf: Some("ggerganov/whisper.cpp"),
        ms: None,
        always_hf: true,
        estimated_bytes: 1_081_140_203,
        files: &["ggml-large-v3-q5_0.bin"],
        files_min_bytes: &[540_570_101],
        files_sha256: &["d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1"],
    },
    ModelEntry {
        key: "turbo",
        display: "turbo",
        hf: Some("ggerganov/whisper.cpp"),
        ms: None,
        always_hf: true,
        estimated_bytes: 574_041_195,
        files: &["ggml-large-v3-turbo-q5_0.bin"],
        files_min_bytes: &[287_020_597],
        files_sha256: &["394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2"],
    },
];

/// 合法 funasr 模型键（与 lt_proto::FUNASR_MODELS 对齐）
pub const FUNASR_KEYS: [&str; 3] = [
    "sensevoice-small",
    "funasr-nano-2512",
    "funasr-mlt-nano-2512",
];

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

/// D-14 幽灵值判定（R25 归一单点）：键在合法值域（FUNASR_MODELS）内却无注册表
/// 条目——当前唯一在案幽灵值 = funasr-mlt-nano-2512（无上游 ONNX 转换，运行时
/// 统一回退 sensevoice-small）。值域成员/表项/幽灵三者的裁决收敛于此与
/// `funasr_entry`：`every_funasr_key_has_entry_or_is_the_documented_ghost`
/// 防线保证新键入值域必有表项（幽灵例外仅此一案）。
pub fn funasr_key_is_ghost(key: &str) -> bool {
    lt_proto::settings::FUNASR_MODELS.contains(&key) && funasr_entry(key).is_none()
}

/// whisper 条目（size 为合法档位时）
pub fn whisper_entry_for(size: &str) -> Option<ModelEntry> {
    WHISPER_ENTRIES.iter().find(|e| e.key == size).cloned()
}

/// Qwen3-ASR 条目（单一模型，settings 无独立模型键——B-α 拓扑：引擎值即模型）。
pub fn qwen3_entry() -> ModelEntry {
    QWEN3_ASR.clone()
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
        assert_eq!(
            whisper_ggml_file("large-v3"),
            Some("ggml-large-v3-q5_0.bin")
        );
        assert_eq!(
            whisper_ggml_file("turbo"),
            Some("ggml-large-v3-turbo-q5_0.bin")
        );
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

    /// R25 防线：合法值域 ↔ 注册表一致性。FUNASR_MODELS 每个合法值要么有
    /// 注册表条目，要么是唯一在案的幽灵值（D-14 mlt：无上游转换，运行时统一
    /// 回退 sensevoice-small）。今后给值域加键而忘登记表（幽灵值复发）在此爆掉；
    /// FUNASR_KEYS 镜像与 lt_proto::FUNASR_MODELS 漂移同样在此爆掉。
    #[test]
    fn every_funasr_key_has_entry_or_is_the_documented_ghost() {
        const GHOST: &str = "funasr-mlt-nano-2512"; // D-14 唯一在案幽灵值
        for key in lt_proto::settings::FUNASR_MODELS {
            let entry = funasr_entry(key);
            if key == GHOST {
                assert!(entry.is_none(), "D-14：mlt 仍无上游转换，不得偷偷登记表项");
            } else {
                assert!(entry.is_some(), "合法值 '{key}' 缺注册表条目（幽灵值复发）");
            }
        }
        assert_eq!(
            FUNASR_KEYS,
            lt_proto::settings::FUNASR_MODELS,
            "键表镜像漂移"
        );
        // R25 归一后的幽灵判定单点行为锁
        assert!(funasr_key_is_ghost(GHOST), "mlt 应判定为幽灵值");
        assert!(!funasr_key_is_ghost("sensevoice-small"));
        assert!(!funasr_key_is_ghost("bogus"), "值域外键不是幽灵，是非法键");
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
        assert!(FUNASR_NANO
            .files
            .iter()
            .any(|f| f.starts_with("Qwen3-0.6B/")));
        assert_eq!(FUNASR_NANO.estimated_bytes, 1_050_000_000);
    }

    #[test]
    fn qwen3_entry_hf_only() {
        // WP-B/D-24：真实仓（HF API 实测存在，作者 csukuangfj2）；无 MS 源 → ms=None 诚实留空
        assert_eq!(
            QWEN3_ASR.hf,
            Some("csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25")
        );
        assert!(QWEN3_ASR.ms.is_none());
        const {
            assert!(QWEN3_ASR.always_hf && QWEN3_ASR.ms.is_none()) // 编译期不变量：仅 HF 源
        }
        // 六件套（三 onnx + tokenizer 子目录）；实测合计 987,015,347B + 余量
        assert_eq!(QWEN3_ASR.files.len(), 6);
        assert_eq!(QWEN3_ASR.files_min_bytes.len(), 6);
        assert!(QWEN3_ASR.files.iter().any(|f| f.starts_with("tokenizer/")));
        assert_eq!(QWEN3_ASR.estimated_bytes, 1_020_000_000);
        assert_eq!(qwen3_entry().key, "qwen3-asr-0.6b");
    }

    #[test]
    fn manifest_min_bytes_parallel_to_files() {
        // DL-1⑥：files 与 files_min_bytes 必须等长（探测 zip 依赖）；
        // whisper 下限 = 实测半体积；funasr 下限远低于实际（防假阴性重下循环）
        for e in WHISPER_ENTRIES.iter() {
            assert_eq!(e.files.len(), e.files_min_bytes.len(), "{}", e.key);
            assert_eq!(e.files_min_bytes[0], e.estimated_bytes / 2, "{}", e.key);
        }
        for e in [
            SENSEVOICE_SMALL.clone(),
            FUNASR_NANO.clone(),
            QWEN3_ASR.clone(),
        ] {
            assert_eq!(e.files.len(), e.files_min_bytes.len(), "{}", e.key);
            assert!(
                e.files_min_bytes[0] >= 1 && e.files_min_bytes[1] >= 1,
                "{}",
                e.key
            );
            // 主文件下限必须远低于估计体积（下限是"远未完成"防线，不是完整性度量）
            assert!(e.files_min_bytes[0] * 2 < e.estimated_bytes, "{}", e.key);
        }
    }

    #[test]
    fn files_sha256_parallel_and_wellformed() {
        // AH-5/H8：sha256 清单与 files 等长；已登记项必须为 64 位十六进制。
        // 空串在机制上仍被允许（下载器跳过内容校验），但**当前全表已零空串**
        //（2026-09-10 whisper 五档补齐）——下方硬不变量把「渐进登记」钉成
        // 历史态：新增模型条目若不登记哈希，此测试即爆（要么补哈希，要么在
        // 此显式放宽并留痕，不得静默落空串）。
        let all = [
            SENSEVOICE_SMALL.clone(),
            FUNASR_NANO.clone(),
            QWEN3_ASR.clone(),
        ]
        .into_iter()
        .chain(WHISPER_ENTRIES.iter().cloned());
        for e in all {
            assert_eq!(e.files.len(), e.files_sha256.len(), "{}", e.key);
            for h in e.files_sha256 {
                assert!(
                    h.is_empty() || (h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())),
                    "{} 含非法 sha256: {h:?}",
                    e.key
                );
            }
        }
        // 硬不变量：全部条目（三个 sherpa 包 + whisper 六档）逐文件登记哈希
        for e in [
            SENSEVOICE_SMALL.clone(),
            FUNASR_NANO.clone(),
            QWEN3_ASR.clone(),
        ]
        .into_iter()
        .chain(WHISPER_ENTRIES.iter().cloned())
        {
            assert!(
                e.files_sha256.iter().all(|h| h.len() == 64),
                "{} 未登记 sha256（渐进登记期已结束：新条目必须带哈希或在此显式放宽）",
                e.key
            );
        }
    }
}
