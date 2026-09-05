//! 缓存探测（原版 model_manager.py 探测逻辑 1:1，经验 E-10）：
//! - HF snapshot 取"字典序最后"而非 mtime
//! - 完整性：所有文件 stat 可解 + 总量过阈值（funasr 50MB / whisper 半体积）
//! - 双 hub 或语义（MS 或 HF 任一缓存即命中，避免重复下载）

use crate::paths::{hf_cache_root, ms_cache_root};
use crate::registry::{self, ModelEntry};
use std::path::{Path, PathBuf};

/// funasr 缓存完整性下限（原版 min_bytes 默认 50MB）
const FUNASR_MIN_BYTES: u64 = 50_000_000;

/// HF repo 缓存根下某 repo 的 snapshots 目录
fn hf_snapshots(models_dir: &Path, repo: &str) -> Option<PathBuf> {
    let (org, name) = repo.split_once('/')?;
    let p = hf_cache_root(models_dir)
        .join(format!("models--{org}--{name}"))
        .join("snapshots");
    p.is_dir().then_some(p)
}

/// 下载器写入布局（与探测同一约定）：
/// HF → `huggingface/hub/models--{org}--{name}/snapshots/{rev}`，
/// MS → `modelscope/models/{org}--{name}/snapshots/{rev}`
pub fn hf_style_snapshot(models_dir: &Path, hub: crate::download::Hub, repo: &str, rev: &str) -> PathBuf {
    let (org, name) = repo.split_once('/').unwrap_or((repo, ""));
    match hub {
        crate::download::Hub::Hf => {
            hf_cache_root(models_dir).join(format!("models--{org}--{name}")).join("snapshots").join(rev)
        }
        crate::download::Hub::Ms => {
            ms_cache_root(models_dir).join("models").join(format!("{org}--{name}")).join("snapshots").join(rev)
        }
    }
}

/// HF repo 是否"存在且下载完成"（E-10：统计可解析文件总字节；
/// 忽略孤儿 .incomplete blob；损坏符号链接视为不完整）
pub fn hf_repo_complete(models_dir: &Path, repo: &str, min_bytes: u64) -> bool {
    let Some(snap_root) = hf_snapshots(models_dir, repo) else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(&snap_root) else {
        return false;
    };
    for snap in entries.flatten() {
        if !snap.path().is_dir() {
            continue;
        }
        let (mut total, mut broken) = (0u64, false);
        if let Ok(files) = walk_files(snap.path()) {
            for f in files {
                match f.metadata() {
                    Ok(m) => total += m.len(),
                    Err(_) => {
                        broken = true;
                        break;
                    }
                }
            }
        } else {
            broken = true;
        }
        if !broken && total >= min_bytes {
            return true;
        }
    }
    false
}

fn walk_files(root: PathBuf) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(dir)?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    Ok(out)
}

/// ModelScope 缓存路径：优先本版下载器布局（models/{org}--{name}/snapshots/<字典序最后>），
/// 兼容旧 funasr SDK 的散布布局（原版 _ms_model_path 1:1）。
pub fn ms_model_path(models_dir: &Path, repo: &str) -> PathBuf {
    let (org, name) = repo.split_once('/').unwrap_or((repo, ""));
    let ms_root = ms_cache_root(models_dir);
    for sub in [
        ms_root.join(org).join(name),
        ms_root.join("models").join(org).join(name),
        ms_root.join("models").join(org).join(name.replace('.', "___")),
        ms_root.join("hub").join("models").join(org).join(name),
        ms_root.join("hub").join(org).join(name),
    ] {
        if sub.exists() {
            return sub;
        }
    }
    // ≥1.38 SDK / 本版下载器：snapshots 取字典序最后（E-10）
    let snap_root = ms_root.join("models").join(format!("{org}--{name}")).join("snapshots");
    if let Ok(entries) = std::fs::read_dir(&snap_root) {
        let mut snaps: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        snaps.sort();
        if let Some(last) = snaps.pop() {
            return last;
        }
    }
    ms_root.join(org).join(name)
}

/// 双 hub 或：MS 路径存在 或 HF 缓存完整（funasr 系；原版 is_asr_cached funasr 分支）
pub fn is_funasr_cached(models_dir: &Path, entry: &ModelEntry) -> bool {
    let ms_ok = entry
        .ms
        .map(|repo| ms_model_path(models_dir, repo).exists())
        .unwrap_or(false);
    let hf_ok = entry
        .hf
        .map(|repo| hf_repo_complete(models_dir, repo, FUNASR_MIN_BYTES))
        .unwrap_or(false);
    ms_ok || hf_ok
}

/// whisper 档位缓存：GGML 单文件，阈值 = 估计体积一半（原版语义）；
/// 非 builtin 值视为本地路径，存在即缓存。
pub fn is_whisper_cached(models_dir: &Path, size: &str) -> bool {
    if registry::whisper_repo(size).is_none() {
        // 本地 GGML 路径（M5 细化扩展名过滤）
        return Path::new(size).is_file();
    }
    let Some(repo) = registry::whisper_repo(size) else { return false };
    let Some(file) = registry::whisper_ggml_file(size) else { return false };
    let entry = registry::whisper_entry_for(size).expect("注册表已核");
    let min = (entry.estimated_bytes / 2).max(1);
    let Some(snap_root) = hf_snapshots(models_dir, repo) else {
        return false;
    };
    // 任一 snapshot 内目标文件存在且过半体积
    let Ok(entries) = std::fs::read_dir(&snap_root) else {
        return false;
    };
    for snap in entries.flatten() {
        let f = snap.path().join(file);
        if let Ok(m) = f.metadata() {
            if m.len() >= min {
                return true;
            }
        }
    }
    false
}

/// 统一入口（对齐原版 is_asr_cached(engine, model, hub)；
/// hub 仅影响下载顺序，探测双 hub 都查）
pub fn is_asr_cached(models_dir: &Path, engine: &str, model: &str) -> bool {
    match engine {
        "funasr" => registry::funasr_entry(model).map_or(false, |e| is_funasr_cached(models_dir, &e)),
        "whisper" => is_whisper_cached(models_dir, model),
        _ => false,
    }
}

/// 本地模型目录（已缓存时返回 snapshot 路径；未缓存 None）。
/// 双 hub 优先级：MS（国内快）→ HF。
pub fn local_model_dir(models_dir: &Path, entry: &ModelEntry) -> Option<PathBuf> {
    if let Some(repo) = entry.ms {
        let p = ms_model_path(models_dir, repo);
        if p.exists() {
            return Some(p);
        }
    }
    if let Some(repo) = entry.hf {
        if let Some(snap_root) = hf_snapshots(models_dir, repo) {
            let mut snaps: Vec<PathBuf> = std::fs::read_dir(&snap_root)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            snaps.sort();
            if let Some(last) = snaps.pop() {
                return Some(last);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("lt_cache_test_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn write(p: &Path, n: u64) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![0u8; n as usize]).unwrap();
    }

    #[test]
    fn hf_complete_requires_threshold_and_resolvable_files() {
        let dir = tmpdir("hf");
        let repo = "csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17";
        let snap = hf_cache_root(&dir).join("models--csukuangfj--sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17").join("snapshots").join("rev1");
        // 小于 50MB → 不算完整
        write(&snap.join("model.int8.onnx"), 10_000_000);
        assert!(!hf_repo_complete(&dir, repo, FUNASR_MIN_BYTES));
        // 过 50MB → 完整
        write(&snap.join("model.int8.onnx"), 60_000_000);
        assert!(hf_repo_complete(&dir, repo, FUNASR_MIN_BYTES));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hf_broken_snapshot_falls_through() {
        let dir = tmpdir("hfbrk");
        let repo = "a/b";
        let snaps = hf_cache_root(&dir).join("models--a--b").join("snapshots");
        write(&snaps.join("rev_old").join("f.bin"), 60_000_000);
        // rev_new 含 .incomplete（截断文件小于阈值）
        write(&snaps.join("rev_new").join("f.bin.incomplete"), 1_000);
        assert!(hf_repo_complete(&dir, repo, FUNASR_MIN_BYTES));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ms_snapshot_takes_lexicographically_last() {
        let dir = tmpdir("ms");
        let snaps = ms_cache_root(&dir).join("models").join("iic--SenseVoiceSmall").join("snapshots");
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
        // MS 侧命中（pengzhendong 镜像仓，M2 核对）
        write(
            &ms_cache_root(&dir).join("pengzhendong").join("sherpa-onnx-sense-voice-zh-en-ja-ko-yue").join("model.int8.onnx"),
            10,
        );
        assert!(is_asr_cached(&dir, "funasr", "sensevoice-small"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn whisper_needs_half_of_estimate() {
        let dir = tmpdir("wh");
        assert!(!is_asr_cached(&dir, "whisper", "tiny"));
        let snap = hf_cache_root(&dir).join("models--ggml-org--whisper-tiny").join("snapshots").join("main");
        write(&snap.join("ggml-tiny-q5_0.bin"), 40_000_000); // < 78MB/2 = 39MB? 40>39 ✓ 但边界测试补一档
        assert!(is_asr_cached(&dir, "whisper", "tiny"));
        // 半体积阈值：78/2=39MB；30MB 不够
        let snap2 = hf_cache_root(&dir).join("models--ggml-org--whisper-tiny").join("snapshots").join("v2");
        write(&snap2.join("ggml-tiny-q5_0.bin"), 30_000_000);
        // 旧 snapshot（main）仍完整 → 或语义下仍 true；独立目录验证：
        let dir2 = tmpdir("wh2");
        let s2 = hf_cache_root(&dir2).join("models--ggml-org--whisper-tiny").join("snapshots").join("main");
        write(&s2.join("ggml-tiny-q5_0.bin"), 30_000_000);
        assert!(!is_asr_cached(&dir2, "whisper", "tiny"));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }
}
