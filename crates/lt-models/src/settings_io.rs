//! settings.json 读写 —— 原子写（tmp + .bak 链，R17）+ 兼容加载 + 坏档隔离（R17）。

use crate::paths::settings_file;
use lt_proto::Settings;

/// 加载设置；文件不存在返回 None（调用方走首启向导）。
/// 解析失败不致命（原版 _load_saved_settings 读坏文件同样得到空 dict），但坏档
/// 不再无痕留在原地等下次 save 覆盖（R17）：原地改名为
/// `settings.json.corrupt-<unix_secs>` 隔离留证；改名失败（如被占用）保留原文件
/// 仅记日志。两种情况都按"文件不存在"处理返回 None。
/// R17 补全（D-75）：settings.json 缺失但 `.bak` 在位时自动恢复——`save` 的
/// 原子链在「现档→bak」与「tmp→现档」之间崩溃会留下该中间态，自动恢复旧档
/// 免于静默回默认值；代价：手动删除 settings.json 的重置意图同样被 .bak 复活，
/// 崩溃自愈优先（取舍登记于 docs/architecture-v2.md §3.7 D-75）。
pub fn load() -> anyhow::Result<Option<Settings>> {
    let path = settings_file()?;
    if !path.exists() {
        // R17/D-75：崩溃中间态自愈。目录占位等异常状态不算 .bak（is_file 判据），
        // 不能恢复时按无配置处理并记日志
        let bak = path.with_extension("json.bak");
        if bak.is_file() {
            if let Err(e) = std::fs::rename(&bak, &path) {
                tracing::warn!(
                    "settings.json 缺失，从保存备份恢复失败（文件可能被占用），按无配置处理: {e}"
                );
                return Ok(None);
            }
            tracing::warn!("settings.json 缺失，已从保存备份 .bak 自动恢复旧配置");
        } else {
            return Ok(None);
        }
    }
    let raw = std::fs::read_to_string(&path)?;
    let v: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            // 坏档隔离（R17）：时间戳取 unix 秒（lt-models 无 chrono 依赖，SystemTime 足矣）
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let corrupt = path.with_extension(format!("json.corrupt-{ts}"));
            match std::fs::rename(&path, &corrupt) {
                Ok(()) => tracing::warn!(
                    "settings.json 解析失败，坏档已隔离为 {} 留证，按无配置处理: {e}",
                    corrupt.display()
                ),
                Err(re) => tracing::warn!(
                    "settings.json 解析失败，隔离改名 {} 失败（文件可能被占用），原文件保留: {re}；解析错误: {e}",
                    corrupt.display()
                ),
            }
            return Ok(None);
        }
    };
    let s = Settings::from_value_compatible(v);
    Ok(Some(s))
}

/// 原子保存（R17 .bak 链）：写 tmp 成功后 现档 → `settings.json.bak`，再 tmp → 现档，
/// 成功后删 .bak。消除旧实现 remove 现档与 rename tmp 之间的"无 settings.json 窗口"
/// （进程死在窗口期 = 配置全丢）：任意时刻崩溃，旧档要么原位、要么在 .bak，均可找回。
pub fn save(s: &Settings) -> anyhow::Result<()> {
    let path = settings_file()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let bak = path.with_extension("json.bak");
    let body = serde_json::to_string_pretty(s)?;
    std::fs::write(&tmp, body)?;
    // 第一步：现档挪进 .bak。覆盖旧 .bak 用先 remove 再 rename 的顺序（Windows 上
    // rename 对已存在目标的行为是失败，ERR_FILE_EXISTS 部分场景，历史注释同款）。
    // 目录占位等异常状态不算"现档"，不进 .bak 链，交给第二步 rename 自然报错。
    if path.is_file() {
        if let Err(e) = std::fs::remove_file(&bak) {
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(anyhow::anyhow!("旧备份 {} 无法覆盖: {e}", bak.display()));
            }
        }
        std::fs::rename(&path, &bak)?;
    }
    // 第二步：tmp 顶上现档。失败则尽力把 .bak 改回恢复旧档；恢复也失败就保留
    // .bak 并在错误信息中提示可手动恢复。
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp); // 尽力清 tmp，失败不影响错误上抛
        let restored = bak.exists() && std::fs::rename(&bak, &path).is_ok();
        return if restored {
            Err(anyhow::anyhow!(
                "settings.json 保存失败（{e}）；旧档已从 {} 恢复",
                bak.display()
            ))
        } else if bak.exists() {
            Err(anyhow::anyhow!(
                "settings.json 保存失败（{e}）；旧档保留在 {}，可手动改回 settings.json",
                bak.display()
            ))
        } else {
            Err(anyhow::anyhow!("settings.json 保存失败: {e}"))
        };
    }
    // 成功：新档就位，.bak 完成使命（NotFound = 本次未走 .bak 链，忽略）
    if let Err(e) = std::fs::remove_file(&bak) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("settings.json 已保存，但清理备份 {} 失败: {e}", bak.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_legacy_load() {
        let _g = crate::ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("lt_settings_test_{}", std::process::id()));
        std::env::set_var("LIVETRANSLATE_CONFIG_DIR", &dir);
        let _cleanup = scopeguard(&dir);

        assert!(load().unwrap().is_none());

        // 保存 → 读回一致
        let s = Settings {
            target_language: "ja".into(),
            vad_threshold: 0.3,
            ..Default::default()
        };
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
                                            // W1/方案 §2.3：no_think=true 迁移为总开关 disable_thinking=true
                                            //（该值即默认值，序列化时省略不写）
        assert!(s.models[0].disable_thinking);
        assert_eq!(s.models[0].thinking_style, None);
        assert!(serde_json::to_value(&s).unwrap()["models"][0]
            .get("disable_thinking")
            .is_none());
        // 未识别键（remote_asr_url 等）被 serde 默认忽略，不报错
    }

    #[test]
    fn corrupted_file_treated_as_missing() {
        let _g = crate::ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("lt_settings_bad_{}", std::process::id()));
        std::env::set_var("LIVETRANSLATE_CONFIG_DIR", &dir);
        let _cleanup = scopeguard(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bad = "{ 这不是合法 json";
        std::fs::write(settings_file().unwrap(), bad).unwrap();

        assert!(load().unwrap().is_none());

        // R17：坏档已原地改名隔离为 settings.json.corrupt-<unix_secs>，内容留证
        let path = settings_file().unwrap();
        assert!(!path.exists(), "坏档应已隔离，settings.json 不再在原地");
        let corrupt: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("settings.json.corrupt-"))
                    .unwrap_or(false)
            })
            .collect();
        assert_eq!(corrupt.len(), 1, "应恰好产生一个隔离文件: {corrupt:?}");
        assert_eq!(
            std::fs::read_to_string(&corrupt[0]).unwrap(),
            bad,
            "隔离文件内容应与原坏档一致"
        );
    }

    #[test]
    fn save_bak_chain_recovers_on_success() {
        let _g = crate::ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("lt_settings_bak_ok_{}", std::process::id()));
        std::env::set_var("LIVETRANSLATE_CONFIG_DIR", &dir);
        let _cleanup = scopeguard(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = settings_file().unwrap();
        let bak = path.with_extension("json.bak");
        std::fs::write(&path, r#"{ "target_language": "zh" }"#).unwrap();

        let s = Settings {
            target_language: "ja".into(),
            ..Default::default()
        };
        save(&s).unwrap();

        // 成功后 .bak 完成使命被删除、tmp 无残留，新内容生效
        assert!(!bak.exists(), "成功保存后不应残留 .bak");
        assert!(!path.with_extension("json.tmp").exists());
        let back = load().unwrap().unwrap();
        assert_eq!(back.target_language, "ja");
    }

    #[test]
    fn save_keeps_bak_when_target_rename_fails() {
        let _g = crate::ENV_LOCK.lock().unwrap();
        let dir =
            std::env::temp_dir().join(format!("lt_settings_bak_fail_{}", std::process::id()));
        std::env::set_var("LIVETRANSLATE_CONFIG_DIR", &dir);
        let _cleanup = scopeguard(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = settings_file().unwrap();
        let bak = path.with_extension("json.bak");
        // 构造第二步 rename 失败（目录占位法）：settings.json 位置被一个目录占住，
        // 「文件 rename 顶替目录」在 Windows/Linux 上都必然失败；上一轮崩溃遗留的
        // .bak 里正是旧档。
        let old = r#"{ "target_language": "zh" }"#;
        std::fs::write(&bak, old).unwrap();
        std::fs::create_dir_all(&path).unwrap();

        let s = Settings {
            target_language: "ja".into(),
            ..Default::default()
        };
        let err = save(&s).expect_err("目录占位应使保存失败");

        // .bak 保留且内容仍是旧档；错误信息提到 .bak 可手动恢复
        assert!(err.to_string().contains(".bak"), "错误信息应提到 .bak: {err}");
        assert!(bak.exists(), "恢复失败后 .bak 应保留");
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), old);
        assert!(!path.is_file(), "目录占位不应被新档顶替");
    }

    /// R17/D-75：保存中途崩溃的中间态（settings.json 缺失 + .bak 在位）→
    /// load 自动恢复旧档，不再静默回默认值；.bak 被改名消耗不复存在。
    #[test]
    fn load_auto_restores_from_bak_when_settings_missing() {
        let _g = crate::ENV_LOCK.lock().unwrap();
        let dir =
            std::env::temp_dir().join(format!("lt_settings_bak_restore_{}", std::process::id()));
        std::env::set_var("LIVETRANSLATE_CONFIG_DIR", &dir);
        let _cleanup = scopeguard(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = settings_file().unwrap();
        let bak = path.with_extension("json.bak");
        // 旧档留在 .bak，settings.json 不复存在（崩溃中间态）
        std::fs::write(&bak, r#"{ "target_language": "ja" }"#).unwrap();
        assert!(!path.exists());

        let back = load().unwrap().expect("应自动恢复 .bak 中的旧配置");
        assert_eq!(back.target_language, "ja");
        assert!(path.is_file(), "恢复后 settings.json 应就位");
        assert!(!bak.exists(), "恢复后 .bak 应被改名消耗");
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
