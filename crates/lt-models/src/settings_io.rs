//! settings.json 读写 —— 原子写（tmp + rename）+ 兼容加载。

use crate::paths::settings_file;
use lt_proto::Settings;

/// 加载设置；文件不存在返回 None（调用方走首启向导）。
/// 任何解析失败都不致命：记警告后按"文件不存在"处理（原版 _load_saved_settings
/// 读坏文件同样得到空 dict）。
pub fn load() -> anyhow::Result<Option<Settings>> {
    let path = settings_file()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)?;
    let v: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("settings.json 解析失败，视为无配置: {e}");
            return Ok(None);
        }
    };
    let s = Settings::from_value_compatible(v);
    Ok(Some(s))
}

/// 原子保存：先写 `settings.json.tmp` 再 rename 覆盖（防崩溃损坏，原版同款）。
pub fn save(s: &Settings) -> anyhow::Result<()> {
    let path = settings_file()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(s)?;
    std::fs::write(&tmp, body)?;
    // Windows 上 rename 目标存在时需要替换语义；std::fs::rename 在 Windows
    // 对已存在目标的行为是失败（ERR_FILE_EXISTS 部分场景），用 remove+rename 兜底。
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 独占一个临时配置目录跑读写往返（避免测试间环境变量竞争：
    /// 串行执行，用全局锁保护环境变量）
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn roundtrip_and_legacy_load() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("lt_settings_test_{}", std::process::id()));
        std::env::set_var("LIVETRANSLATE_CONFIG_DIR", &dir);
        let _cleanup = scopeguard(&dir);

        assert!(load().unwrap().is_none());

        // 保存 → 读回一致
        let mut s = Settings::default();
        s.target_language = "ja".into();
        s.vad_threshold = 0.3;
        save(&s).unwrap();
        let back = load().unwrap().unwrap();
        assert_eq!(back.target_language, "ja");
        assert_eq!(back.vad_threshold, 0.3);

        // 手工写入一份"原版风格"文件（含 legacy 键），走兼容加载
        let original_style = r#"{
            "asr_engine": "remote-whisper",
            "remote_asr_url": "http://127.0.0.1:8765",
            "asr_device": "cuda:0 (RTX 4090)",
            "models": [{"name":"x","api_base":"y","api_key":"z","model":"m","no_think": true}],
            "vad_mode": "silero", "vad_threshold": 0.3,
            "min_speech_duration": 1.0, "max_speech_duration": 8.0,
            "silence_mode": "auto", "silence_duration": 0.8,
            "asr_language": "auto", "target_language": "zh",
            "hub": "ms", "download_proxy": "system", "asr_engine_old": null
        }"#;
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(settings_file().unwrap(), original_style).unwrap();
        let s = load().unwrap().unwrap();
        assert_eq!(s.asr_engine, "funasr"); // remote-whisper 裁剪回退
        // no_think=true 迁移为 thinking_style="auto"（内存值；序列化时该值会被跳过）
        assert_eq!(s.models[0].thinking_style.as_deref(), Some("auto"));
        assert!(serde_json::to_value(&s).unwrap()["models"][0].get("thinking_style").is_none());
        // 未识别键（remote_asr_url 等）被 serde 默认忽略，不报错
    }

    #[test]
    fn corrupted_file_treated_as_missing() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("lt_settings_bad_{})", std::process::id()).replace(")", ""));
        std::env::set_var("LIVETRANSLATE_CONFIG_DIR", &dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(settings_file().unwrap(), "{ 这不是合法 json").unwrap();
        assert!(load().unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
        std::env::remove_var("LIVETRANSLATE_CONFIG_DIR");
    }

    // 简易 RAII 清理（避免引入 scopeguard 依赖）
    struct Guard<'a>(&'a std::path::Path);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0);
            std::env::remove_var("LIVETRANSLATE_CONFIG_DIR");
        }
    }
    fn scopeguard(p: &std::path::Path) -> Guard<'_> {
        Guard(p)
    }
}
