//! 模型缓存磁盘布局（架构 2.0 W6 自 lt-models 上移，lt-download 分家后归属本 crate）：
//! 下载器写入、缓存探测、数据页扫描三方言明的路径拼装**单一事实源**——
//! 手写 format 串会漂移（AH-6/H10：qwen3 曾因漏改数据页扫描而应用内不可见）。
//!
//! 布局约定（原版 hf-hub/modelscope SDK 自控替代，M-01）：
//! HF → `models_dir/huggingface/hub/models--{org}--{name}/snapshots/{rev}`
//! MS → `models_dir/modelscope/models/{org}--{name}/snapshots/{rev}`

use std::path::{Path, PathBuf};

/// 下载目标 hub（原 lt-models::download::Hub；W2 后失败分类已入契约，
/// W6 分家时 hub/布局拼装随之归契约层——两 crate 共用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hub {
    Ms,
    Hf,
}

impl Hub {
    /// settings 字符串 → 值域枚举（E2/D-79 唯一转换点；未知值回退 Ms 并
    /// 告警——消灭 orchestrator `if hub_s == "hf" {Hf} else {Ms}` 的静默暗默认）
    pub fn from_settings_str(v: &str) -> Self {
        match v {
            "hf" => Self::Hf,
            "ms" => Self::Ms,
            other => {
                tracing::warn!("未知 hub 值 {other:?}，回退 ms（Hub 单点转换）");
                Self::Ms
            }
        }
    }

    /// → settings 持久层字符串（写回 `Settings.hub` 用）
    pub fn as_settings_str(self) -> &'static str {
        match self {
            Self::Ms => "ms",
            Self::Hf => "hf",
        }
    }
}

/// HF 缓存根（布局兼容原版：huggingface/hub/models--org--name/...）
pub fn hf_cache_root(models_dir: &Path) -> PathBuf {
    models_dir.join("huggingface").join("hub")
}

/// ModelScope 缓存根
pub fn ms_cache_root(models_dir: &Path) -> PathBuf {
    models_dir.join("modelscope")
}

/// HF repo 缓存目录（`models--{org}--{name}`）——目录名拼装的单一来源：
/// 数据页扫描/缓存卡路径一律经此构造。
pub fn hf_repo_dir(models_dir: &Path, repo: &str) -> PathBuf {
    let (org, name) = repo.split_once('/').unwrap_or((repo, ""));
    hf_cache_root(models_dir).join(format!("models--{org}--{name}"))
}

/// 下载器写入布局（与缓存探测同一约定）：HF → snapshots/{rev}，
/// MS → models/{org}--{name}/snapshots/{rev}
pub fn hf_style_snapshot(models_dir: &Path, hub: Hub, repo: &str, rev: &str) -> PathBuf {
    match hub {
        Hub::Hf => hf_repo_dir(models_dir, repo).join("snapshots").join(rev),
        Hub::Ms => {
            let (org, name) = repo.split_once('/').unwrap_or((repo, ""));
            ms_cache_root(models_dir)
                .join("models")
                .join(format!("{org}--{name}"))
                .join("snapshots")
                .join(rev)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 布局与旧下载器明文约定一致（MS master / HF main 快照位）
    #[test]
    fn snapshot_layouts_match_documented_paths() {
        let dir = Path::new("/models");
        assert_eq!(
            hf_repo_dir(dir, "ggml-org/whisper-tiny"),
            Path::new("/models/huggingface/hub/models--ggml-org--whisper-tiny")
        );
        assert_eq!(
            hf_style_snapshot(dir, Hub::Hf, "ggml-org/whisper-tiny", "main"),
            Path::new(
                "/models/huggingface/hub/models--ggml-org--whisper-tiny/snapshots/main"
            )
        );
        assert_eq!(
            hf_style_snapshot(dir, Hub::Ms, "iic/SenseVoiceSmall", "master"),
            Path::new("/models/modelscope/models/iic--SenseVoiceSmall/snapshots/master")
        );
        // 无斜杠 repo：org=repo 自身（下载清单项同语义）
        assert_eq!(
            hf_repo_dir(dir, "single"),
            Path::new("/models/huggingface/hub/models--single--")
        );
    }

    /// E2/D-79：Hub 单点转换——合法值域恒等；未知值回退 Ms（消灭
    /// orchestrator 静默 else 后的告警回退语义钉死）
    #[test]
    fn hub_mapping_and_fallback() {
        assert_eq!(Hub::from_settings_str("ms"), Hub::Ms);
        assert_eq!(Hub::from_settings_str("hf"), Hub::Hf);
        assert_eq!(Hub::from_settings_str("MS"), Hub::Ms, "大小写敏感，未知即回退");
        assert_eq!(Hub::from_settings_str(""), Hub::Ms);
        for h in [Hub::Ms, Hub::Hf] {
            assert_eq!(Hub::from_settings_str(h.as_settings_str()), h);
        }
    }
}
