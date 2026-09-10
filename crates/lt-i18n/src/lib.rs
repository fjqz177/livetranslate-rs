//! lt-i18n: LiveTranslate 的国际化模块。
//!
//! 编译期内嵌 zh/en 两张扁平键值表（YAML），提供语言检测、切换与查询接口。
//! 语义对齐 Python 原版 LiveTranslate/i18n.py：缺 key 时 `t()` 返回 key 本身。

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use serde_yaml::from_str;

/// 内嵌的中文语言表（编译期从仓库 assets/i18n/zh.yaml 嵌入二进制）
const ZH_YAML: &str = include_str!("../../../assets/i18n/zh.yaml");
/// 内嵌的英文语言表（编译期从仓库 assets/i18n/en.yaml 嵌入二进制）
const EN_YAML: &str = include_str!("../../../assets/i18n/en.yaml");

/// 全局语言状态：当前语言码 + 已加载的字符串表
struct State {
    lang: String,
    map: HashMap<String, String>,
}

/// 全局状态，懒初始化：首次调用 t()/get_lang()/set_lang() 时按系统语言建立
static STATE: OnceLock<RwLock<State>> = OnceLock::new();

/// 检测系统语言：locale 以 zh 开头（如 zh-CN）返回 "zh"，否则（含取不到）返回 "en"
pub fn detect_system_lang() -> &'static str {
    match sys_locale::get_locale() {
        Some(locale) if locale.to_lowercase().starts_with("zh") => "zh",
        _ => "en",
    }
}

/// 按语言码取内嵌表内容；没有对应内嵌表的语言码回退英文表
/// （与原版"文件不存在回退 en.yaml"语义一致，语言码本身保留为所设值）
fn yaml_for_lang(lang: &str) -> &'static str {
    match lang {
        "zh" => ZH_YAML,
        _ => EN_YAML,
    }
}

/// 把 YAML 文本解析为扁平键值表（R14/D-62：解析失败返回值——内嵌资产
/// 损坏即全 UI 显示原始键名的静默空表，应作为 boot 硬错让用户可见）
fn parse_yaml(text: &str) -> Result<HashMap<String, String>, serde_yaml::Error> {
    from_str(text)
}

/// 取全局状态锁（不存在则先用系统语言懒初始化）
fn state() -> &'static RwLock<State> {
    STATE.get_or_init(|| {
        let lang = detect_system_lang().to_string();
        // 内嵌资产损坏（构建期后）是致命故障：此处无 Result 面（懒初始化
        // 取引用），直接 panic——boot 期 get 前会先经 set_lang 硬错呈现
        let map = parse_yaml(yaml_for_lang(&lang))
            .unwrap_or_else(|e| panic!("内嵌 i18n 资产损坏: {e}"));
        RwLock::new(State { lang, map })
    })
}

/// 切换界面语言并重新加载对应语言表（无对应内嵌表时加载英文表）。
/// R14：内嵌资产解析失败 → Err（boot 期硬错呈现；运行期恢复默认英文表）
pub fn set_lang(lang: &str) -> Result<(), String> {
    let mut guard = state().write().unwrap_or_else(|e| e.into_inner());
    let map = parse_yaml(yaml_for_lang(lang)).map_err(|e| format!("i18n 解析失败: {e}"))?;
    guard.lang = lang.to_string();
    guard.map = map;
    Ok(())
}

/// 获取当前语言码（如 "zh" / "en"）
pub fn get_lang() -> String {
    let guard = state().read().unwrap_or_else(|e| e.into_inner());
    guard.lang.clone()
}

/// 按 key 查询翻译文本；缺 key 时返回 key 本身（与 Python 原版 t() 语义一致）
pub fn t(key: &str) -> String {
    let guard = state().read().unwrap_or_else(|e| e.into_inner());
    guard
        .map
        .get(key)
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

/// 按语言码直接查询翻译文本（不触碰全局状态——测试用；避免并行测试对
/// 全局 set_lang 的竞争导致的语义断言不稳定）
pub fn t_for_lang(lang: &str, key: &str) -> String {
    parse_yaml(yaml_for_lang(lang))
        .map(|m| m.get(key).cloned().unwrap_or_else(|| key.to_string()))
        .unwrap_or_else(|_| key.to_string())
}

/// (code, 原生显示名)。auto 项显示名由调用方用 t("asr_lang_auto") 处理，此处为 None。
/// 逐项转录自 Python 原版 LiveTranslate/i18n.py 第 42-73 行，共 30 项。
pub const LANGUAGES: &[(&str, Option<&str>)] = &[
    ("auto", None), // 显示名由调用方用 t("asr_lang_auto") 处理
    ("ja", Some("日本語")),
    ("en", Some("English")),
    ("zh", Some("中文")),
    ("ko", Some("한국어")),
    ("fr", Some("Français")),
    ("de", Some("Deutsch")),
    ("es", Some("Español")),
    ("ru", Some("Русский")),
    ("pt", Some("Português")),
    ("it", Some("Italiano")),
    ("nl", Some("Nederlands")),
    ("pl", Some("Polski")),
    ("tr", Some("Türkçe")),
    ("ar", Some("العربية")),
    ("th", Some("ไทย")),
    ("vi", Some("Tiếng Việt")),
    ("id", Some("Bahasa Indonesia")),
    ("ms", Some("Bahasa Melayu")),
    ("hi", Some("हिन्दी")),
    ("uk", Some("Українська")),
    ("cs", Some("Čeština")),
    ("ro", Some("Română")),
    ("el", Some("Ελληνικά")),
    ("hu", Some("Magyar")),
    ("sv", Some("Svenska")),
    ("da", Some("Dansk")),
    ("fi", Some("Suomi")),
    ("no", Some("Norsk")),
    ("he", Some("עברית")),
];

/// 托盘菜单直接显示的常用语言（其余进"更多语言"子菜单）
pub const COMMON_LANG_CODES: &[&str] = &["auto", "ja", "en", "zh", "ko", "fr", "de", "es", "ru"];

#[cfg(test)]
mod tests {
    use super::*;

    /// 缺 key 时 t() 应回退返回 key 本身（任意语言表下均成立）
    #[test]
    fn t_missing_key_falls_back_to_key() {
        assert_eq!(t("__no_such_key__"), "__no_such_key__");
    }

    /// zh/en 两表键集必须完全一致（R14 防线）：任何一侧增删键而不同步另一侧，
    /// 此测试即刻爆掉——杜绝"一侧显示原文键名"的漂移
    #[test]
    fn zh_en_key_sets_identical() {
        let zh_map = parse_yaml(ZH_YAML).expect("zh.yaml 解析");
        let en_map = parse_yaml(EN_YAML).expect("en.yaml 解析");
        assert!(!zh_map.is_empty(), "zh.yaml 不应为空");
        let mut missing_in_en: Vec<&String> =
            zh_map.keys().filter(|k| !en_map.contains_key(*k)).collect();
        let mut missing_in_zh: Vec<&String> =
            en_map.keys().filter(|k| !zh_map.contains_key(*k)).collect();
        missing_in_en.sort();
        missing_in_zh.sort();
        assert!(missing_in_en.is_empty(), "en.yaml 缺键: {missing_in_en:?}");
        assert!(missing_in_zh.is_empty(), "zh.yaml 缺键: {missing_in_zh:?}");
        assert_eq!(zh_map.len(), en_map.len(), "键数应一致");
    }

    /// 两表内**不得有重复键**（2026-09-11 评审修复）：`HashMap` 反序列化对重复键
    /// 静默 last-wins——厂商预设的 custom 项曾与样式页 `preset_custom` 撞名，
    /// 把样式页标签覆盖成"自定义（不填充）"且全流程无告警。`serde_yaml::Value`
    /// 走 `Mapping` 路径会检出错（`duplicate entry`），故用它做守卫。
    #[test]
    fn yaml_has_no_duplicate_keys() {
        for (name, text) in [("zh.yaml", ZH_YAML), ("en.yaml", EN_YAML)] {
            serde_yaml::from_str::<serde_yaml::Value>(text)
                .unwrap_or_else(|e| panic!("{name} 含重复键或非法结构: {e}"));
        }
    }

    /// 切换语言后 t() 应查到对应语言的译文
    #[test]
    fn set_lang_switches_table() {
        // 前置确认：内嵌的 zh/en 两表都确有 quit 键，且两表译文不同
        let zh_map = parse_yaml(ZH_YAML).expect("zh.yaml 解析");
        let en_map = parse_yaml(EN_YAML).expect("en.yaml 解析");
        assert!(zh_map.contains_key("quit"), "zh.yaml 应包含 quit 键");
        assert!(en_map.contains_key("quit"), "en.yaml 应包含 quit 键");
        assert_ne!(zh_map["quit"], en_map["quit"]);

        // 切到 zh：语言码与译文都应生效
        set_lang("zh").expect("zh 表解析");
        assert_eq!(get_lang(), "zh");
        assert_eq!(t("quit"), zh_map["quit"]);

        // 切到 en：同key 译文必须不同，证明表确实被重新加载
        set_lang("en").expect("en 表解析");
        assert_eq!(get_lang(), "en");
        assert_eq!(t("quit"), en_map["quit"]);
        assert_ne!(zh_map["quit"], en_map["quit"]);
    }

    /// LANGUAGES 共 30 项，且首项为 ("auto", None)
    #[test]
    fn languages_has_30_entries_and_starts_with_auto() {
        assert_eq!(LANGUAGES.len(), 30);
        assert_eq!(LANGUAGES[0], ("auto", None));
    }

    /// COMMON_LANG_CODES 的每一项都必须出现在 LANGUAGES 中
    #[test]
    fn common_codes_all_in_languages() {
        for code in COMMON_LANG_CODES {
            assert!(
                LANGUAGES.iter().any(|(c, _)| c == code),
                "常用语言 {code} 不在 LANGUAGES 中"
            );
        }
    }
}
