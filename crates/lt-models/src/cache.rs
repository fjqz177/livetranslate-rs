//! 缓存探测（原版 model_manager.py 探测语义的阶段二改造，DL-1 / docs/download-overhaul.md DEC-1）：
//! - 完整性 = 注册表 manifest 逐文件「存在 + len ≥ 下限」（files_min_bytes），
//!   取代原版「递归体积 ≥ 阈值」启发式——半截 .incomplete / 空文件 / 占位估计
//!   都不再可能被判「已缓存」（假阴性=多下一次可自愈，假阳性=死局）
//! - HF snapshot 取"字典序最后"且**含完整 manifest**（E-10 沿革）
//! - 双 hub 或语义（MS 或 HF 任一缓存即命中，避免重复下载）

use crate::paths::{hf_cache_root, ms_cache_root};
use crate::registry::{self, ModelEntry};
use std::path::{Path, PathBuf};

/// HF repo 缓存目录（`huggingface/hub/models--{org}--{name}`）——目录名拼装
/// 的单一来源：数据页扫描/缓存卡路径一律经此构造，手写 format 串会漂移
/// （AH-6/H10；qwen3 曾因漏改数据页扫描而应用内不可见）
pub fn hf_repo_dir(models_dir: &Path, repo: &str) -> PathBuf {
    let (org, name) = repo.split_once('/').unwrap_or((repo, ""));
    hf_cache_root(models_dir).join(format!("models--{org}--{name}"))
}

/// HF repo 缓存根下某 repo 的 snapshots 目录
fn hf_snapshots(models_dir: &Path, repo: &str) -> Option<PathBuf> {
    let p = hf_repo_dir(models_dir, repo).join("snapshots");
    p.is_dir().then_some(p)
}

/// 下载器写入布局（与探测同一约定）：
/// HF → `huggingface/hub/models--{org}--{name}/snapshots/{rev}`，
/// MS → `modelscope/models/{org}--{name}/snapshots/{rev}`
pub fn hf_style_snapshot(
    models_dir: &Path,
    hub: crate::download::Hub,
    repo: &str,
    rev: &str,
) -> PathBuf {
    match hub {
        crate::download::Hub::Hf => hf_repo_dir(models_dir, repo).join("snapshots").join(rev),
        crate::download::Hub::Ms => {
            let (org, name) = repo.split_once('/').unwrap_or((repo, ""));
            ms_cache_root(models_dir)
                .join("models")
                .join(format!("{org}--{name}"))
                .join("snapshots")
                .join(rev)
        }
    }
}

/// manifest 完整性：目录内注册表清单逐文件「存在 + len ≥ 下限」。
/// 只认清单文件——`.incomplete` 等旁支产物天然不参与（DL-1，灭 F1）。
pub fn dir_has_manifest(dir: &Path, files: &[&str], mins: &[u64]) -> bool {
    files.iter().zip(mins.iter()).all(|(f, min)| {
        dir.join(f)
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() >= *min)
    })
}

/// HF repo 下含完整 manifest 的字典序最后快照（None = 无完整快照）。
pub fn hf_manifest_snapshot(
    models_dir: &Path,
    repo: &str,
    files: &[&str],
    mins: &[u64],
) -> Option<PathBuf> {
    let snap_root = hf_snapshots(models_dir, repo)?;
    let mut snaps: Vec<PathBuf> = std::fs::read_dir(snap_root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    snaps.sort();
    snaps
        .into_iter()
        .rev()
        .find(|s| dir_has_manifest(s, files, mins))
}

/// HF repo 是否"存在且下载完成"（manifest 语义；MS/HF 双 hub 或的上 HF 侧）。
pub fn hf_repo_cached(models_dir: &Path, repo: &str, entry: &ModelEntry) -> bool {
    hf_manifest_snapshot(models_dir, repo, entry.files, entry.files_min_bytes).is_some()
}

/// ModelScope 缓存路径：优先本版下载器布局（models/{org}--{name}/snapshots/<字典序最后>），
/// 兼容旧 funasr SDK 的散布布局（原版 _ms_model_path 1:1）。
pub fn ms_model_path(models_dir: &Path, repo: &str) -> PathBuf {
    let (org, name) = repo.split_once('/').unwrap_or((repo, ""));
    let ms_root = ms_cache_root(models_dir);
    for sub in [
        ms_root.join(org).join(name),
        ms_root.join("models").join(org).join(name),
        ms_root
            .join("models")
            .join(org)
            .join(name.replace('.', "___")),
        ms_root.join("hub").join("models").join(org).join(name),
        ms_root.join("hub").join(org).join(name),
    ] {
        if sub.exists() {
            return sub;
        }
    }
    // ≥1.38 SDK / 本版下载器：snapshots 取字典序最后（E-10）
    let snap_root = ms_root
        .join("models")
        .join(format!("{org}--{name}"))
        .join("snapshots");
    if let Ok(entries) = std::fs::read_dir(&snap_root) {
        let mut snaps: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        snaps.sort();
        if let Some(last) = snaps.pop() {
            return last;
        }
    }
    ms_root.join(org).join(name)
}

/// 双 hub 或：MS 路径 manifest 齐全 或 HF 存在完整 manifest 快照
/// （funasr 系；DL-1 后两 hub 同一 manifest 标准，不再有阈值分裂）
pub fn is_funasr_cached(models_dir: &Path, entry: &ModelEntry) -> bool {
    let ms_ok = entry
        .ms
        .map(|repo| {
            dir_has_manifest(
                &ms_model_path(models_dir, repo),
                entry.files,
                entry.files_min_bytes,
            )
        })
        .unwrap_or(false);
    let hf_ok = entry
        .hf
        .map(|repo| hf_repo_cached(models_dir, repo, entry))
        .unwrap_or(false);
    ms_ok || hf_ok
}

/// whisper 档位 → 模型 .bin 绝对路径。
/// builtin 档：HF snapshot 取字典序最后（E-10）且含目标量化文件的目录；
/// 非 builtin 值视为本地 GGML 路径，存在即返回。
pub fn whisper_model_path(models_dir: &Path, size: &str) -> Option<PathBuf> {
    let Some(repo) = registry::whisper_repo(size) else {
        let p = PathBuf::from(size);
        return p.is_file().then_some(p);
    };
    let file = registry::whisper_ggml_file(size)?;
    let snap_root = hf_snapshots(models_dir, repo)?;
    let mut snaps: Vec<PathBuf> = std::fs::read_dir(snap_root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    snaps.sort();
    snaps
        .into_iter()
        .rev()
        .map(|snap| snap.join(file))
        .find(|f| f.is_file())
}

/// whisper 档位缓存：GGML 单文件，下限 = 注册表 files_min_bytes[0]（实测半体积，
/// 完整下载必过阈）；非 builtin 值视为本地路径，存在即缓存。
pub fn is_whisper_cached(models_dir: &Path, size: &str) -> bool {
    if registry::whisper_repo(size).is_none() {
        // 本地 GGML 路径
        return Path::new(size).is_file();
    }
    let Some(p) = whisper_model_path(models_dir, size) else {
        return false;
    };
    let entry = registry::whisper_entry_for(size).expect("注册表已核");
    let min = entry.files_min_bytes.first().copied().unwrap_or(1);
    p.metadata().is_ok_and(|m| m.len() >= min)
}

/// 统一入口（对齐原版 is_asr_cached(engine, model, hub)；
/// hub 仅影响下载顺序，探测双 hub 都查）。
/// 注（AH-6/H10）：生产路径各有专用判定（funasr `is_funasr_cached` / whisper
/// `is_whisper_cached`+`whisper_model_path` / 装配 `local_model_dir`），本函数
/// 现为测试与人工核验入口——引擎分派臂勿以它为唯一参照。
pub fn is_asr_cached(models_dir: &Path, engine: &str, model: &str) -> bool {
    match engine {
        "funasr" => registry::funasr_entry(model).is_some_and(|e| is_funasr_cached(models_dir, &e)),
        "whisper" => is_whisper_cached(models_dir, model),
        // WP-B：qwen3 单一模型，settings 无模型键（B-α），model 参数无意义
        "qwen3" => is_funasr_cached(models_dir, &registry::qwen3_entry()),
        _ => false,
    }
}

/// 缺失模型条目（原版 get_missing_models 的元素结构）。
/// Silero VAD 内嵌 exe（D-5），恒不缺失。
#[derive(Debug, Clone)]
pub struct MissingModel {
    /// 显示名（向导/下载对话框标题行用，如 "SenseVoice Small" / "Whisper tiny"）
    pub display: String,
    pub hub_hf: Option<&'static str>,
    pub hub_ms: Option<&'static str>,
    pub always_hf: bool,
    pub files: &'static [&'static str],
    /// 每个清单文件的字节数下限（与 files 等长；下载器跳过校验共用，DL-2）
    pub files_min_bytes: &'static [u64],
    /// 每个清单文件的 sha256（与 files 等长；空串=未登记跳过内容校验，AH-5）
    pub files_sha256: &'static [&'static str],
    /// 估计体积（注册表 calibrated；UI 进度条总量与「未缓存 ≈X」共用）
    pub estimated_bytes: u64,
}

/// 缺失模型清单（原版 get_missing_models；本地自定义 whisper 路径不触发下载）
pub fn missing_models(
    models_dir: &Path,
    engine: &str,
    funasr_model: &str,
    whisper_size: &str,
) -> Vec<MissingModel> {
    match engine {
        "funasr" => {
            // mlt/非法键回退 sensevoice-small（与 pipeline 装载一致，D-14）
            let entry = registry::funasr_entry(funasr_model)
                .or_else(|| registry::funasr_entry("sensevoice-small"))
                .expect("sensevoice-small 常量条目必存在");
            if is_funasr_cached(models_dir, &entry) {
                Vec::new()
            } else {
                vec![MissingModel {
                    display: entry.display.into(),
                    hub_hf: entry.hf,
                    hub_ms: entry.ms,
                    always_hf: entry.always_hf,
                    files: entry.files,
                    files_min_bytes: entry.files_min_bytes,
                    files_sha256: entry.files_sha256,
                    estimated_bytes: entry.estimated_bytes,
                }]
            }
        }
        "whisper" => {
            // 非 builtin 档位（本地 GGML 路径）不算缺失（原版同语义）
            if registry::whisper_repo(whisper_size).is_none()
                || is_whisper_cached(models_dir, whisper_size)
            {
                return Vec::new();
            }
            let entry = registry::whisper_entry_for(whisper_size).expect("builtin 档位已核");
            vec![MissingModel {
                display: format!("Whisper {}", entry.key),
                hub_hf: entry.hf,
                hub_ms: entry.ms,
                always_hf: entry.always_hf,
                files: entry.files,
                files_min_bytes: entry.files_min_bytes,
                files_sha256: entry.files_sha256,
                estimated_bytes: entry.estimated_bytes,
            }]
        }
        // WP-B：qwen3 单一模型（B-α），两个模型参数均无意义
        "qwen3" => {
            let entry = registry::qwen3_entry();
            if is_funasr_cached(models_dir, &entry) {
                Vec::new()
            } else {
                vec![MissingModel {
                    display: entry.display.into(),
                    hub_hf: entry.hf,
                    hub_ms: entry.ms,
                    always_hf: entry.always_hf,
                    files: entry.files,
                    files_min_bytes: entry.files_min_bytes,
                    files_sha256: entry.files_sha256,
                    estimated_bytes: entry.estimated_bytes,
                }]
            }
        }
        _ => Vec::new(),
    }
}

/// 本地模型目录（已缓存时返回 manifest 完整的 snapshot 路径；未缓存 None）。
/// 双 hub 优先级：MS（国内快）→ HF；两 hub 同一 manifest 标准（DL-1 灭 F13：
/// 原实现 MS 侧只查 exists、HF 侧取字典序最后不验内容，均可能拿到半截目录）。
pub fn local_model_dir(models_dir: &Path, entry: &ModelEntry) -> Option<PathBuf> {
    if let Some(repo) = entry.ms {
        let p = ms_model_path(models_dir, repo);
        if dir_has_manifest(&p, entry.files, entry.files_min_bytes) {
            return Some(p);
        }
    }
    if let Some(repo) = entry.hf {
        return hf_manifest_snapshot(models_dir, repo, entry.files, entry.files_min_bytes);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(name: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("lt_cache_test_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn write(p: &Path, n: u64) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![0u8; n as usize]).unwrap();
    }

    #[test]
    fn incomplete_only_snapshot_is_not_cached() {
        // DL-1/F1①：仅 .incomplete（哪怕 150MB）→ 未缓存（原实现计入体积误判已缓存）
        let dir = tmpdir("f1a");
        let entry = registry::funasr_entry("sensevoice-small").unwrap();
        let snap = hf_cache_root(&dir)
            .join("models--csukuangfj--sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17")
            .join("snapshots")
            .join("main");
        write(&snap.join("model.int8.onnx.incomplete"), 150_000_000);
        assert!(!is_funasr_cached(&dir, &entry));
        assert!(local_model_dir(&dir, &entry).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn incomplete_plus_partial_manifest_not_cached() {
        // DL-1/F1②（死局复现用例）：.incomplete 150MB + 完整 tokens.txt 同快照
        // → 未缓存。原实现 sum ≥ 50MB 判「已缓存」，重试即假成功。
        let dir = tmpdir("f1b");
        let entry = registry::funasr_entry("sensevoice-small").unwrap();
        let snap = hf_cache_root(&dir)
            .join("models--csukuangfj--sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17")
            .join("snapshots")
            .join("main");
        write(&snap.join("model.int8.onnx.incomplete"), 150_000_000);
        write(&snap.join("tokens.txt"), 4_096);
        assert!(!is_funasr_cached(&dir, &entry));
        assert!(!is_asr_cached(&dir, "funasr", "sensevoice-small"));
        assert!(
            missing_models(&dir, "funasr", "sensevoice-small", "").len() == 1,
            "必须仍报缺失（可重试真下载），不得假成功"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_complete_both_hubs_cached() {
        // DL-1/F1③：双 hub 各自 manifest 齐全即命中（或语义），且阈值不再随 hub 分裂
        let entry = registry::funasr_entry("sensevoice-small").unwrap();
        // MS 侧（本版下载器布局）
        let dir = tmpdir("f1c_ms");
        let ms_snap = ms_cache_root(&dir)
            .join("models")
            .join("pengzhendong--sherpa-onnx-sense-voice-zh-en-ja-ko-yue")
            .join("snapshots")
            .join("master");
        write(&ms_snap.join("model.int8.onnx"), 60_000_000);
        write(&ms_snap.join("tokens.txt"), 4_096);
        assert!(is_funasr_cached(&dir, &entry));
        assert_eq!(local_model_dir(&dir, &entry).unwrap(), ms_snap);
        let _ = fs::remove_dir_all(&dir);
        // HF 侧
        let dir = tmpdir("f1c_hf");
        let hf_snap = hf_cache_root(&dir)
            .join("models--csukuangfj--sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17")
            .join("snapshots")
            .join("main");
        write(&hf_snap.join("model.int8.onnx"), 60_000_000);
        write(&hf_snap.join("tokens.txt"), 4_096);
        assert!(is_funasr_cached(&dir, &entry));
        assert_eq!(local_model_dir(&dir, &entry).unwrap(), hf_snap);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_missing_or_undersized_file_not_cached() {
        // DL-1/F1④：缺一文件 / 文件低于下限（tokens.txt ≥ 1KB）都判未缓存
        let dir = tmpdir("f1d");
        let entry = registry::funasr_entry("sensevoice-small").unwrap();
        let snap = hf_cache_root(&dir)
            .join("models--csukuangfj--sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17")
            .join("snapshots")
            .join("main");
        write(&snap.join("model.int8.onnx"), 60_000_000);
        assert!(!is_funasr_cached(&dir, &entry), "缺 tokens.txt");
        write(&snap.join("tokens.txt"), 512);
        assert!(!is_funasr_cached(&dir, &entry), "tokens.txt 低于 1KB 下限");
        write(&snap.join("tokens.txt"), 4_096);
        assert!(is_funasr_cached(&dir, &entry));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_model_dir_skips_incomplete_newer_snapshot() {
        // DL-1/F1⑤：较新快照含 .incomplete 不完整 → 回退较旧完整快照（MS/HF 两 hub 各验）
        let dir = tmpdir("f1e");
        let entry = registry::funasr_entry("sensevoice-small").unwrap();
        let snaps = hf_cache_root(&dir)
            .join("models--csukuangfj--sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17")
            .join("snapshots");
        write(&snaps.join("rev_old").join("model.int8.onnx"), 60_000_000);
        write(&snaps.join("rev_old").join("tokens.txt"), 4_096);
        write(
            &snaps.join("zzz_new").join("model.int8.onnx.incomplete"),
            150_000_000,
        );
        write(&snaps.join("zzz_new").join("tokens.txt"), 4_096);
        let got = local_model_dir(&dir, &entry).expect("应回退旧完整快照");
        assert!(got.ends_with("rev_old"), "got={got:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hf_broken_snapshot_falls_through() {
        // 沿革保留：快照含 .incomplete/坏文件不影响其他快照判定（manifest 语义天然成立）
        let dir = tmpdir("hfbrk");
        let repo = "a/b";
        let snaps = hf_cache_root(&dir).join("models--a--b").join("snapshots");
        write(&snaps.join("rev_old").join("f.bin"), 60_000_000);
        write(&snaps.join("rev_new").join("f.bin.incomplete"), 1_000);
        let entry_files = ["f.bin"];
        let entry_mins = [1];
        assert!(hf_manifest_snapshot(&dir, repo, &entry_files, &entry_mins)
            .expect("旧快照应命中")
            .ends_with("rev_old"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ms_snapshot_takes_lexicographically_last() {
        let dir = tmpdir("ms");
        let snaps = ms_cache_root(&dir)
            .join("models")
            .join("iic--SenseVoiceSmall")
            .join("snapshots");
        write(&snaps.join("aaa").join("tokens.txt"), 10);
        write(&snaps.join("zzz").join("tokens.txt"), 10);
        let got = ms_model_path(&dir, "iic/SenseVoiceSmall");
        assert!(got.ends_with("zzz"), "got={got:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_asr_cached_dual_hub_or() {
        let dir = tmpdir("dual");
        assert!(!is_asr_cached(&dir, "funasr", "sensevoice-small"));
        // MS 侧命中（pengzhendong 镜像仓，M2 核对）——manifest 需两文件齐全且过下限
        //（DL-1 前：体积 ≥ 50MB 单文件即判已缓存，半截仓会误判）
        let ms_dir = ms_cache_root(&dir)
            .join("pengzhendong")
            .join("sherpa-onnx-sense-voice-zh-en-ja-ko-yue");
        write(&ms_dir.join("model.int8.onnx"), 130_000_000);
        write(&ms_dir.join("tokens.txt"), 4_096);
        assert!(is_asr_cached(&dir, "funasr", "sensevoice-small"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn qwen3_manifest_probe_semantics() {
        // WP-B：qwen3 探测复用 funasr manifest 机制（model 参数无意义，B-α 单一模型）
        let dir = tmpdir("qwen3");
        assert!(!is_asr_cached(&dir, "qwen3", ""));
        let miss = missing_models(&dir, "qwen3", "", "");
        assert_eq!(miss.len(), 1);
        assert_eq!(miss[0].display, "Qwen3-ASR-0.6B");
        assert!(miss[0].always_hf, "D-24：qwen3 仅 HF 源");
        assert!(miss[0].hub_ms.is_none());
        assert_eq!(
            miss[0].hub_hf,
            Some("csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25")
        );
        assert_eq!(miss[0].files_min_bytes.len(), miss[0].files.len());

        // HF 侧 manifest 齐全（六件套逐文件过下限，tokenizer/ 为子目录）→ 已缓存
        let snap = hf_cache_root(&dir)
            .join("models--csukuangfj2--sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25")
            .join("snapshots")
            .join("main");
        write(&snap.join("conv_frontend.onnx"), 44_148_281);
        write(&snap.join("encoder.int8.onnx"), 182_491_662);
        write(&snap.join("decoder.int8.onnx"), 755_914_231);
        write(&snap.join("tokenizer/merges.txt"), 1_671_853);
        write(&snap.join("tokenizer/tokenizer_config.json"), 12_487);
        write(&snap.join("tokenizer/vocab.json"), 2_776_833);
        assert!(is_asr_cached(&dir, "qwen3", ""));
        assert!(missing_models(&dir, "qwen3", "", "").is_empty());
        // local_model_dir 命中 snapshot（装配层 model_dir 来源）
        let entry = registry::qwen3_entry();
        assert_eq!(local_model_dir(&dir, &entry).unwrap(), snap);
        // 缺 tokenizer 子目录任一文件即判未缓存
        let dir2 = tmpdir("qwen3b");
        let snap2 = hf_cache_root(&dir2)
            .join("models--csukuangfj2--sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25")
            .join("snapshots")
            .join("main");
        write(&snap2.join("conv_frontend.onnx"), 44_148_281);
        write(&snap2.join("encoder.int8.onnx"), 182_491_662);
        write(&snap2.join("decoder.int8.onnx"), 755_914_231);
        write(&snap2.join("tokenizer/merges.txt"), 1_671_853);
        write(&snap2.join("tokenizer/vocab.json"), 2_776_833);
        assert!(
            !is_asr_cached(&dir2, "qwen3", ""),
            "缺 tokenizer_config.json"
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }

    #[test]
    fn missing_models_semantics() {
        let dir = tmpdir("missing");
        // 空目录：funasr 缺 sensevoice-small
        let miss = missing_models(&dir, "funasr", "sensevoice-small", "");
        assert_eq!(miss.len(), 1);
        assert_eq!(miss[0].display, "SenseVoice Small");
        // 缺失条目带估计体积（进度条总量源）+ manifest 下限（下载器跳过校验源）
        assert!(miss[0].estimated_bytes > 0, "estimated_bytes 应填充");
        assert_eq!(
            miss[0].files_min_bytes.len(),
            miss[0].files.len(),
            "下限须与清单等长"
        );
        // mlt 键回退后同样报 sensevoice-small 缺失（D-14）
        let miss = missing_models(&dir, "funasr", "funasr-mlt-nano-2512", "");
        assert_eq!(miss[0].display, "SenseVoice Small");
        // whisper builtin 档缺；本地路径不触发下载
        let miss = missing_models(&dir, "whisper", "", "tiny");
        assert_eq!(miss.len(), 1);
        assert_eq!(miss[0].display, "Whisper tiny");
        assert!(missing_models(&dir, "whisper", "", "D:/my/model.bin").is_empty());
        // MS 侧 manifest 齐全（≥50MB + tokens）→ funasr 不再缺失
        let ms_dir = ms_cache_root(&dir)
            .join("pengzhendong")
            .join("sherpa-onnx-sense-voice-zh-en-ja-ko-yue");
        write(&ms_dir.join("model.int8.onnx"), 130_000_000);
        write(&ms_dir.join("tokens.txt"), 4_096);
        assert!(missing_models(&dir, "funasr", "sensevoice-small", "").is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn whisper_needs_half_of_estimate() {
        let dir = tmpdir("wh");
        assert!(!is_asr_cached(&dir, "whisper", "tiny"));
        // tiny q5_1 实际 32_152_673B，半体积阈值 ≈16MB：20MB 过、10MB 不过
        let snap = hf_cache_root(&dir)
            .join("models--ggerganov--whisper.cpp")
            .join("snapshots")
            .join("main");
        write(&snap.join("ggml-tiny-q5_1.bin"), 20_000_000);
        assert!(is_asr_cached(&dir, "whisper", "tiny"));
        // 独立目录验证半体积下界
        let dir2 = tmpdir("wh2");
        let s2 = hf_cache_root(&dir2)
            .join("models--ggerganov--whisper.cpp")
            .join("snapshots")
            .join("main");
        write(&s2.join("ggml-tiny-q5_1.bin"), 10_000_000);
        assert!(!is_asr_cached(&dir2, "whisper", "tiny"));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }

    #[test]
    fn whisper_model_path_picks_lexicographically_last_snapshot() {
        let dir = tmpdir("whpath");
        let snaps = hf_cache_root(&dir)
            .join("models--ggerganov--whisper.cpp")
            .join("snapshots");
        // 新旧 snapshot 均含文件 → 取字典序最后（E-10）
        write(&snaps.join("aaa").join("ggml-tiny-q5_1.bin"), 10);
        write(&snaps.join("zzz").join("ggml-tiny-q5_1.bin"), 10);
        let got = whisper_model_path(&dir, "tiny").expect("应解析到文件");
        assert!(got
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with("zzz/ggml-tiny-q5_1.bin"));
        // 新 snapshot 缺文件 → 回退旧 snapshot
        let dir2 = tmpdir("whpath2");
        let snaps2 = hf_cache_root(&dir2)
            .join("models--ggerganov--whisper.cpp")
            .join("snapshots");
        write(&snaps2.join("aaa").join("ggml-tiny-q5_1.bin"), 10);
        write(&snaps2.join("zzz").join("ggml-base-q5_1.bin"), 10);
        let got2 = whisper_model_path(&dir2, "tiny").expect("旧 snapshot 回退");
        assert!(got2
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with("aaa/ggml-tiny-q5_1.bin"));
        // 完全缺失 → None；本地路径直通
        assert!(whisper_model_path(&tmpdir("whpath3"), "small").is_none());
        let local = whisper_model_path(&dir, "D:/models/ggml-tiny.bin");
        assert!(local.is_none(), "不存在的本地路径不应命中");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }
}

#[cfg(test)]
mod probe_tmp {
    #[test]
    fn probe_real_cache() {
        let md = std::path::Path::new("C:/Users/fjqz177/.config/livetranslate/models");
        println!(
            "whisper_model_path tiny = {:?}",
            crate::cache::whisper_model_path(md, "tiny")
        );
        println!(
            "is_whisper_cached tiny = {}",
            crate::cache::is_whisper_cached(md, "tiny")
        );
        println!("hf_cache_root = {:?}", crate::paths::hf_cache_root(md));
    }
}

#[cfg(test)]
mod probe_settings_tmp {
    #[test]
    fn probe_load_from_smoke_dir() {
        std::env::set_var(
            "LIVETRANSLATE_CONFIG_DIR",
            "C:/Users/fjqz177/.zcode/tmp/lt_smoke",
        );
        let s = crate::settings_io::load();
        println!("load = {s:?}");
        if let Ok(Some(s)) = s {
            println!("models_dir = {:?}", s.models_dir);
            println!(
                "engine = {:?} size = {:?}",
                s.asr_engine, s.whisper_model_size
            );
        }
    }
}
