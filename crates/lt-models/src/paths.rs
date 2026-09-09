//! 配置与数据目录解析（D-9：全部收拢到 `~/.config/livetranslate`）。
//!
//! 用户明确要求使用字面 `~/.config`（而非 Windows 惯例 %APPDATA%）。
//! 环境变量 `LIVETRANSLATE_CONFIG_DIR` 供测试与便携模式覆盖。

use std::path::PathBuf;

/// 配置根目录：`~/.config/livetranslate`（测试可用环境变量覆盖）
pub fn config_dir() -> anyhow::Result<PathBuf> {
    if let Ok(p) = std::env::var("LIVETRANSLATE_CONFIG_DIR") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("无法定位用户主目录"))?;
    Ok(home.join(".config").join("livetranslate"))
}

/// settings.json 路径
pub fn settings_file() -> anyhow::Result<PathBuf> {
    Ok(config_dir()?.join("settings.json"))
}

/// 模型缓存根目录：settings.models_dir 或默认 `<config>/models`
pub fn models_dir(custom: Option<&std::path::Path>) -> anyhow::Result<PathBuf> {
    match custom {
        Some(p) => Ok(p.to_path_buf()),
        None => Ok(config_dir()?.join("models")),
    }
}

/// HF 缓存根（布局兼容原版：huggingface/hub/models--org--name/...）
/// W6 起拼装单一事实源在 lt_proto::layout（下载器与探测共用）
pub use lt_proto::layout::hf_cache_root;

/// ModelScope 缓存根
pub use lt_proto::layout::ms_cache_root;

/// 转写输出目录
pub fn transcripts_dir() -> anyhow::Result<PathBuf> {
    Ok(config_dir()?.join("transcripts"))
}

/// 日志目录
pub fn logs_dir() -> anyhow::Result<PathBuf> {
    Ok(config_dir()?.join("logs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_respects_env_override() {
        let _g = crate::ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("lt_test_cfg");
        std::env::set_var("LIVETRANSLATE_CONFIG_DIR", &dir);
        assert_eq!(config_dir().unwrap(), dir);
        std::env::remove_var("LIVETRANSLATE_CONFIG_DIR");
    }

    #[test]
    fn cache_layout_compatible_with_original() {
        let md = PathBuf::from("/models");
        assert_eq!(hf_cache_root(&md), PathBuf::from("/models/huggingface/hub"));
        assert_eq!(ms_cache_root(&md), PathBuf::from("/models/modelscope"));
    }
}
