//! 字体系统（docs/font-system-plan.md W-3/W-6）：仓库自洽——所有字形由内嵌字体保证，
//! 系统字体仅作增强（锦上添花），任何开发者 clone 仓库即得一致渲染、零系统污染。
//!
//! - 内嵌思源黑体（Noto Sans CJK SC = Source Han Sans SC，OFL 1.1）恒在
//!   Proportional / Monospace / 命名族三条链的兜底位，任何系统字体缺失都不方块；
//! - 内嵌 Noto Sans Mono CJK SC（同发行源）作为等宽 chrome 的保证回退：
//!   系统有 Consolas 时保持原版 1:1（Consolas 为微软字体不可重分发/不内嵌）；
//! - UI 符号（✓ ✗ ● ▲ ▼ 等）由思源覆盖，不再依赖系统 seguisym；
//! - 行级字体键空串 = 跟随主设置（D-17 级联模型，见 [`resolve_family`]）；
//! - 系统字体经注册表枚举 + 懒加载字节（只读选中/注册的族；Windows 专属，
//!   其他平台为空列表——仓库内仍可完整渲染）。
//!
//! 渲染侧约定：`FontFamily::Name(族名)` 只出现在 [`FontsState::resolved`]（已注册者）
//! —— epaint 的 `FontsImpl::font` 对未绑定族名会 panic，绝不直接构造未注册的 Name。

use anyhow::{anyhow, Result};
use egui::{Context, FontData, FontDefinitions, FontFamily, FontId};
use lt_proto::Settings;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 内嵌思源黑体（W-1 入库，OFL 1.1）；`FontData::from_static` 零拷贝。
pub const EMBEDDED_BYTES: &[u8] = include_bytes!("../../../assets/fonts/NotoSansCJKsc-Regular.otf");
/// 内嵌等宽字体（W-6 入库，与思源同 noto-cjk 发布源/同设计；系统缺 Consolas 时保证
/// 等宽 chrome（含 CJK）全机器一致。Consolas 为微软字体不可重分发，故仅系统增强）。
pub const MONO_BYTES: &[u8] = include_bytes!("../../../assets/fonts/NotoSansMonoCJKsc-Regular.otf");
/// 内嵌符号字体（W-6 入库，OFL 1.1；补 ✗ 等思源未覆盖的符号——旧实现靠系统
/// seguisym.ttf（微软字体不可重分发），仓库自洽后符号渲染不再依赖系统）。
pub const SYMBOLS_BYTES: &[u8] = include_bytes!("../../../assets/fonts/NotoSansSymbols2-Regular.ttf");
/// 内嵌字体「展示族名」：设置键默认值 + 选择器首项显示。
pub const EMBEDDED_FAMILY: &str = "Noto Sans CJK SC";
/// 内嵌思源在 `FontDefinitions.font_data` 中的注册名（链尾兜底 + 命名族链尾部）。
pub const EMBEDDED_KEY: &str = "sans-cjk-sc";
/// 内嵌等宽字体的注册名（Monospace 链内，兜底位）。
pub const MONO_KEY: &str = "sans-cjk-mono";
/// 内嵌符号字体的注册名（各链尾部；✓ ✗ 等）。
pub const SYMBOLS_KEY: &str = "sans-symbols";
/// 中英韩样例：UI 预览卡与字形覆盖测试共用（防漂移）。
pub const SAMPLE_TEXT: &str = "天地玄黄 宇宙洪荒 The quick brown fox 한국어 어둠 0123456789";
/// 字形覆盖检查样例（选择器缺字提示与覆盖测试用）。
pub const CJK_SAMPLE: &str = "天地玄黄宇宙洪荒한국어日本語";

/// 系统字体条目：注册表值清洗后的展示族名 + 字体文件路径（字节懒加载）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFont {
    pub display: String,
    pub path: PathBuf,
}

/// 字体运行时态（挂在 AppState；启动扫描一次，选择器「刷新」重扫）。
#[derive(Default)]
pub struct FontsState {
    /// 系统字体扫描结果（注册表枚举，仅元数据不读字节）。
    pub system: Vec<SystemFont>,
    /// 已加载字节（Arc 复用：热应用重建 FontDefinitions 时不重复读文件/拷贝）。
    loaded: HashMap<PathBuf, Arc<FontData>>,
    consolas: Option<Arc<FontData>>,
    /// 族名 → 渲染可用 FontFamily（apply_fonts 时重建；渲染侧每帧只查表）。
    pub resolved: HashMap<String, FontFamily>,
}

impl FontsState {
    /// 启动构造：注册表扫描一次（<10ms；失败仅记录，降级仅内嵌）。
    pub fn new() -> Self {
        let mut s = Self::default();
        s.rescan();
        s
    }

    /// 重扫系统字体（「刷新字体列表」按钮）。不影响已应用的选择。
    pub fn rescan(&mut self) {
        match scan_system_fonts() {
            Ok(list) => {
                tracing::info!("系统字体扫描完成: {} 个", list.len());
                self.system = list;
            }
            Err(e) => {
                self.system.clear();
                tracing::warn!("系统字体扫描失败（{e:#}），仅内嵌字体可用");
            }
        }
    }
}

/// 级联解析（D-17）：行级键空串 = 跟随主设置。
pub fn resolve_family<'a>(row: &'a str, master: &'a str) -> &'a str {
    if row.trim().is_empty() { master } else { row }
}

/// 渲染侧取 FontFamily：已解析注册则用命名族，否则回落全局链（Proportional）。
pub fn font_family_for(family: &str, fonts: &FontsState) -> FontFamily {
    fonts
        .resolved
        .get(family)
        .cloned()
        .unwrap_or(FontFamily::Proportional)
}

/// 一键应用：按当前 Settings 重建字体定义并 set_fonts。
/// 全窗口共享单 Context（MultiWindowApp.ctx），一次生效、下一帧起渲染。
pub fn apply_fonts(ctx: &Context, settings: &Settings, fonts: &mut FontsState) {
    let defs = build_definitions(settings, fonts);
    ctx.set_fonts(defs);
    ctx.request_repaint();
}

/// 字体族是否覆盖 CJK/韩文样例（选择器缺字提示）。
/// family 必须来自 [`FontsState::resolved`]（已注册）或全局链——未绑定族名会 panic。
pub fn family_covers_cjk(ctx: &Context, family: &FontFamily) -> bool {
    ctx.fonts_mut(|f| f.has_glyphs(&FontId::new(16.0, family.clone()), CJK_SAMPLE))
}

// ── 系统字体扫描 ──

/// 枚举 HKLM+HKCU 字体注册表，输出清洗后的系统字体列表。
/// 注册表不可读/为空 → Err（调用方降级仅内嵌）。非 Windows 平台恒空（仓库自洽不受影响）。
#[cfg(windows)]
pub fn scan_system_fonts() -> Result<Vec<SystemFont>> {
    use ::windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    let mut raw: Vec<(String, String)> = Vec::new();
    // HKCU 先枚举（用户自装字体优先于 HKLM 同名族）
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        unsafe {
            scan_hive(hive, &mut raw);
        }
    }
    if raw.is_empty() {
        return Err(anyhow!("字体注册表枚举结果为空（HKLM/HKCU 均未读到值）"));
    }
    Ok(normalize_registry_entries(&raw, &system_root()))
}

/// 非 Windows 平台：无系统字体列表（内嵌链保证完整渲染，选择器仅内嵌项）。
#[cfg(not(windows))]
pub fn scan_system_fonts() -> Result<Vec<SystemFont>> {
    Ok(Vec::new())
}

/// 系统根目录（%SystemRoot%，缺省 C:\Windows）。
fn system_root() -> PathBuf {
    std::env::var("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Windows"))
}

/// 单注册表键枚举：把 (值名, 值数据) 追加到 out。键必然关闭。
#[cfg(windows)]
unsafe fn scan_hive(hive: ::windows::Win32::System::Registry::HKEY, out: &mut Vec<(String, String)>) {
    use ::windows::Win32::Foundation::ERROR_NO_MORE_ITEMS;
    use ::windows::Win32::System::Registry::{RegCloseKey, RegEnumValueW, RegOpenKeyExW, HKEY, KEY_READ};
    use ::windows::core::{PCWSTR, PWSTR};

    let subkey: Vec<u16> = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Fonts\0"
        .encode_utf16()
        .collect();
    let mut key = HKEY::default();
    let err = RegOpenKeyExW(hive, PCWSTR(subkey.as_ptr()), None, KEY_READ, &mut key);
    if !err.is_ok() {
        tracing::debug!("字体注册表键打开失败(hive): {err:?}");
        return;
    }
    let mut index: u32 = 0;
    loop {
        let mut name = vec![0u16; 1024];
        let mut name_len = name.len() as u32;
        let mut data = vec![0u8; 4096];
        let mut data_len = data.len() as u32;
        let err = RegEnumValueW(
            key,
            index,
            Some(PWSTR(name.as_mut_ptr())),
            &mut name_len,
            None,
            None,
            Some(data.as_mut_ptr()),
            Some(&mut data_len),
        );
        if err == ERROR_NO_MORE_ITEMS {
            break;
        }
        index += 1;
        if !err.is_ok() {
            // ERROR_MORE_DATA 等：条目过长/类型异常，跳过该值
            continue;
        }
        let name = String::from_utf16_lossy(&name[..name_len as usize]);
        // REG_SZ/REG_EXPAND_SZ 数据为 UTF-16 字节（含结尾 NUL）
        let units = data_len as usize / 2;
        let mut value = String::from_utf16_lossy(unsafe {
            std::slice::from_raw_parts(data.as_ptr() as *const u16, units)
        });
        if let Some(nul) = value.find('\0') {
            value.truncate(nul);
        }
        if !name.is_empty() && !value.is_empty() {
            out.push((name, value));
        }
    }
    let _ = RegCloseKey(key);
}

/// 纯函数：注册表 (值名, 值数据) 原始对 → 系统字体列表（单测覆盖）。规则：
/// - 值名去尾部 "(TrueType)"/"(OpenType)"/"(All res)" 等说明段（[`strip_registry_suffix`]）；
/// - 值数据为相对路径 → 拼 system_root；绝对路径原样；`%SystemRoot%`/`%WINDIR%` 前缀展开；
/// - 仅保留 .ttf/.ttc/.otf；族名大小写不敏感去重（先到先得，调用方保证 HKCU 在前）；
/// - 与内嵌思源同族名丢弃（内嵌恒为链首/选择器置顶）。
pub fn normalize_registry_entries(raw: &[(String, String)], system_root: &Path) -> Vec<SystemFont> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (name, data) in raw {
        let display = strip_registry_suffix(name);
        if display.is_empty() || display.eq_ignore_ascii_case(EMBEDDED_FAMILY) {
            continue;
        }
        let Some(path) = resolve_font_path(data, system_root) else {
            continue;
        };
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "ttc" | "otf") {
            continue;
        }
        if !seen.insert(display.to_lowercase()) {
            continue;
        }
        out.push(SystemFont { display, path });
    }
    out
}

/// 值名 "微软雅黑 (TrueType)" → "微软雅黑"；仅剥尾段、内容含字体说明关键字的 "(...)"。
pub fn strip_registry_suffix(name: &str) -> String {
    let s = name.trim();
    let Some(open) = s.rfind('(') else {
        return s.to_string();
    };
    if s.ends_with(')') {
        let inside = &s[open + 1..s.len() - 1];
        let l = inside.to_ascii_lowercase();
        if l.contains("truetype") || l.contains("opentype") || l.contains("all res") || l.contains("font")
        {
            return s[..open].trim_end().to_string();
        }
    }
    s.to_string()
}

/// 值数据 → 路径：`%SystemRoot%/%WINDIR%` 前缀展开为相对 system_root；绝对路径原样；
/// 相对路径拼 system_root。空/非法返回 None。
fn resolve_font_path(data: &str, system_root: &Path) -> Option<PathBuf> {
    let d = data.trim();
    let lowered = d.to_ascii_lowercase();
    // 展开首个 "%…%" 前缀段（仅 %SystemRoot% / %WINDIR%）
    let normalized: &str = if lowered.starts_with("%systemroot%") || lowered.starts_with("%windir%")
    {
        let close = d[1..].find('%')? + 1;
        let var = &lowered[..close + 1];
        if var == "%systemroot%" || var == "%windir%" {
            &d[close + 1..]
        } else {
            d
        }
    } else {
        d
    };
    let normalized = normalized.trim_start_matches(|c| c == '\\' || c == '/');
    if normalized.is_empty() {
        return None;
    }
    let p = PathBuf::from(normalized);
    Some(if p.is_absolute() { p } else { system_root.join(p) })
}

// ── 链构建 ──

/// 收集需求族（去重保序）；空串跳过（=跟随，不注册）。
fn wish_family<'a>(wanted: &mut Vec<&'a str>, seen: &mut HashSet<String>, fam: &'a str) {
    if fam.is_empty() || !seen.insert(fam.to_lowercase()) {
        return;
    }
    wanted.push(fam);
}

/// 按 Settings 重建 FontDefinitions（纯构建；调用方 set_fonts）。链装配（仓库自洽）：
/// - Proportional：[界面字体] → [内嵌思源] → [内嵌符号] → [egui 默认族尾]
/// - Monospace：[Consolas(系统，锦上添花)] → [内嵌等宽 MonoCJK] → [界面字体] → [内嵌思源] → [内嵌符号] → [默认族尾]
/// - Name(族名)（行级/字幕字体）：[该字体] → [内嵌思源] → [内嵌符号]
/// 找不到的族名不注册（Name 不出现），渲染经 resolved 回落 Proportional（全局链，内嵌兜底）。
/// **不读取任何系统符号字体**：✓ ✗ ● ▲ ▼ 等由内嵌思源 + Noto Sans Symbols 2 覆盖
/// （覆盖率测试常驻，缺失任一字符即失败——仓库自洽成为硬保证）。
pub fn build_definitions(settings: &Settings, fonts: &mut FontsState) -> FontDefinitions {
    let mut defs = FontDefinitions::default();

    // 内嵌三字体恒注册：思源（链尾兜底）+ 等宽 MonoCJK（chrome 保证回退）+ 符号
    defs.font_data
        .insert(EMBEDDED_KEY.into(), FontData::from_static(EMBEDDED_BYTES).into());
    defs.font_data
        .insert(MONO_KEY.into(), FontData::from_static(MONO_BYTES).into());
    defs.font_data
        .insert(SYMBOLS_KEY.into(), FontData::from_static(SYMBOLS_BYTES).into());

    // 系统锦上添花：Consolas（等宽 chrome，原版 QFont Consolas；缺省回落内嵌等宽）
    let consolas_key = "consolas";
    let consolas_ok = load_window_font(&mut fonts.consolas, "consola.ttf")
        .map(|d| {
            defs.font_data
                .insert(consolas_key.into(), d.into());
        })
        .is_some();

    let ui = settings.ui_font_family.trim();
    let sub = settings.subtitle_font_family.trim();

    // 需求族去重（大小写不敏感，保序）：界面 → 字幕主 → 行级/样式键非空
    let mut wanted: Vec<&str> = Vec::new();
    let mut seen = HashSet::new();
    wish_family(&mut wanted, &mut seen, ui);
    wish_family(&mut wanted, &mut seen, sub);
    for line in &settings.subtitle_mode.lines {
        wish_family(&mut wanted, &mut seen, line.font_family.trim());
    }
    wish_family(
        &mut wanted,
        &mut seen,
        settings.style.original_font_family.trim(),
    );
    wish_family(
        &mut wanted,
        &mut seen,
        settings.style.translation_font_family.trim(),
    );

    // 逐族解析：注册命名族链 + 记录 resolved（失败族名不注册，回落全局链）
    fonts.resolved.clear();
    let mut ui_key: Option<String> = None;
    for family in &wanted {
        let Some((key, data)) = resolve_font_data(family, fonts) else {
            fonts
                .resolved
                .insert((*family).to_string(), FontFamily::Proportional);
            continue;
        };
        defs.font_data.insert(key.clone(), data);
        let mut chain = vec![key.clone()];
        push_unique(&mut chain, EMBEDDED_KEY.into());
        push_unique(&mut chain, SYMBOLS_KEY.into());
        defs.families
            .insert(FontFamily::Name(Arc::from(*family)), chain);
        fonts
            .resolved
            .insert((*family).to_string(), FontFamily::Name(Arc::from(*family)));
        if *family == ui {
            ui_key = Some(key);
        }
    }

    // 全局族（界面字体为 Proportional 链首；chrome 等宽：Consolas→内嵌等宽保证）
    let default_prop =
        std::mem::take(defs.families.get_mut(&FontFamily::Proportional).unwrap());
    let default_mono =
        std::mem::take(defs.families.get_mut(&FontFamily::Monospace).unwrap());

    let mut prop = Vec::new();
    if let Some(k) = &ui_key {
        push_unique(&mut prop, k.clone());
    }
    push_unique(&mut prop, EMBEDDED_KEY.into());
    push_unique(&mut prop, SYMBOLS_KEY.into());
    prop.extend(default_prop);
    defs.families.insert(FontFamily::Proportional, prop);

    let mut mono = Vec::new();
    if consolas_ok {
        push_unique(&mut mono, consolas_key.into());
    }
    push_unique(&mut mono, MONO_KEY.into());
    if let Some(k) = &ui_key {
        push_unique(&mut mono, k.clone());
    }
    push_unique(&mut mono, EMBEDDED_KEY.into());
    push_unique(&mut mono, SYMBOLS_KEY.into());
    mono.extend(default_mono);
    defs.families.insert(FontFamily::Monospace, mono);

    defs
}

/// 族名 → 字体数据（注册键名 + Arc<FontData>）。命中内嵌或系统扫描表；
/// 系统字节懒加载缓存；找不到 → None（调用方回落全局链）。
fn resolve_font_data(family: &str, fonts: &mut FontsState) -> Option<(String, Arc<FontData>)> {
    if family.eq_ignore_ascii_case(EMBEDDED_FAMILY) {
        return Some((
            EMBEDDED_KEY.into(),
            Arc::new(FontData::from_static(EMBEDDED_BYTES)),
        ));
    }
    let sys = fonts
        .system
        .iter()
        .find(|f| f.display.eq_ignore_ascii_case(family))?;
    let key = format!("sys-{}", sys.display.to_lowercase());
    match fonts.loaded.entry(sys.path.clone()) {
        std::collections::hash_map::Entry::Occupied(e) => Some((key, e.get().clone())),
        std::collections::hash_map::Entry::Vacant(e) => match std::fs::read(&sys.path) {
            Ok(bytes) => {
                let data = Arc::new(FontData::from_owned(bytes));
                let out = data.clone();
                e.insert(data);
                Some((key, out))
            }
            Err(err) => {
                tracing::warn!(
                    "字体文件读取失败（{family}）: {}: {err}",
                    sys.path.display()
                );
                None
            }
        },
    }
}

/// 读 C:\Windows\Fonts\<file>（缓存于 FontsState，仅首次读磁盘；缺省回落内嵌字体）。
fn load_window_font(cache: &mut Option<Arc<FontData>>, file: &str) -> Option<Arc<FontData>> {
    if cache.is_none() {
        let path = system_root().join("Fonts").join(file);
        *cache = std::fs::read(&path).ok().map(|b| Arc::new(FontData::from_owned(b)));
        if cache.is_none() {
            tracing::warn!("系统字体未找到（{file}），相关字符回落内嵌字体");
        }
    }
    cache.clone()
}

/// 去重追加（保持首次出现位置，同字符后续跳过）。
fn push_unique(list: &mut Vec<String>, key: String) {
    if !list.iter().any(|k| k == &key) {
        list.push(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_suffix_true_type_and_others() {
        assert_eq!(strip_registry_suffix("微软雅黑 (TrueType)"), "微软雅黑");
        assert_eq!(strip_registry_suffix("Arial Bold (TrueType)"), "Arial Bold");
        assert_eq!(strip_registry_suffix("Noto Sans (OpenType)"), "Noto Sans");
        assert_eq!(strip_registry_suffix("MS Gothic (All res)"), "MS Gothic");
        assert_eq!(strip_registry_suffix("Consolas"), "Consolas");
        assert_eq!(strip_registry_suffix("Foo (Bar) (TrueType)"), "Foo (Bar)");
        // 非说明性尾段不剥
        assert_eq!(strip_registry_suffix("Foo (Bar)"), "Foo (Bar)");
        assert_eq!(strip_registry_suffix(""), "");
    }

    #[test]
    fn normalize_keeps_valid_and_filters_rest() {
        let raw = vec![
            ("微软雅黑 (TrueType)".into(), "msyh.ttc".into()),
            ("Consolas (TrueType)".into(), "consola.ttf".into()),
            ("Segoe UI (TrueType)".into(), "segoeui.ttf".into()),
            // 非法扩展名与 .fon 过滤
            ("Bad Ext (TrueType)".into(), "bad.exe".into()),
            ("Old System (All res)".into(), "old.fon".into()),
            // 与内嵌思源同名丢弃
            (EMBEDDED_FAMILY.into(), "szh.otf".into()),
            // 相对路径 → 拼系统根
            ("Custom (TrueType)".into(), "fonts\\custom.ttf".into()),
            // %SystemRoot% 前缀展开
            ("Native (OpenType)".into(), "%SystemRoot%\\Fonts\\native.otf".into()),
            // 绝对路径原样
            ("Abs (TrueType)".into(), r"D:\userfonts\abs.ttf".into()),
        ];
        let out = normalize_registry_entries(&raw, Path::new(r"C:\Windows"));
        let names: Vec<&str> = out.iter().map(|f| f.display.as_str()).collect();
        assert_eq!(
            names,
            vec!["微软雅黑", "Consolas", "Segoe UI", "Custom", "Native", "Abs"]
        );
        assert_eq!(out[0].path, PathBuf::from(r"C:\Windows\msyh.ttc"));
        assert_eq!(
            out[3].path,
            PathBuf::from(r"C:\Windows\fonts\custom.ttf")
        );
        assert_eq!(out[4].path, PathBuf::from(r"C:\Windows\Fonts\native.otf"));
        assert_eq!(out[5].path, PathBuf::from(r"D:\userfonts\abs.ttf"));
    }

    #[test]
    fn normalize_dedup_case_insensitive_first_wins() {
        // 先到先得（调用方保证 HKCU 在前）：同族名不同大小写只留首个
        let raw = vec![
            ("Comic Sans MS (TrueType)".into(), "comic.ttf".into()),
            ("comic sans ms (TrueType)".into(), "comic2.ttf".into()),
            ("Comic Sans MS (TrueType)".into(), "comic3.ttf".into()),
        ];
        let out = normalize_registry_entries(&raw, Path::new(r"C:\Windows"));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].path, PathBuf::from(r"C:\Windows\comic.ttf"));
    }

    #[test]
    fn resolve_family_cascade() {
        assert_eq!(resolve_family("", "Noto Sans CJK SC"), "Noto Sans CJK SC");
        assert_eq!(resolve_family("   ", "Master"), "Master");
        assert_eq!(resolve_family("Microsoft YaHei", "Master"), "Microsoft YaHei");
    }

    /// 内嵌链字形覆盖测试（机器无关：不读任何系统字体文件）。
    /// 替换旧 hangul 系统字体测试（原读 C:\Windows\Fonts\malgun.ttf，缺失即跳过）。
    #[test]
    fn embedded_chain_covers_cjk_hangul_latin() {
        let ctx = egui::Context::default();
        let settings = Settings::default();
        let mut fonts = FontsState::default();
        apply_fonts(&ctx, &settings, &mut fonts);
        ctx.begin_pass(egui::RawInput::default());
        let ok = ctx.fonts_mut(|f| f.has_glyphs(&FontId::proportional(16.0), SAMPLE_TEXT));
        assert!(ok, "内嵌思源链应覆盖中/英/韩/数字样例: {SAMPLE_TEXT}");
    }

    /// 内部：解析级字形缺失清单（skrifa cmap；不吃 epaint has_glyph 对
    /// replacement-face 的启发式假阴性——见 epaint font.rs has_glyph TODO）。
    fn font_missing(bytes: &[u8], sample: &str) -> String {
        use skrifa::MetadataProvider as _;
        let font = skrifa::FontRef::from_index(bytes, 0).expect("内嵌字体应可解析");
        let cmap = font.charmap();
        sample
            .chars()
            .filter(|c| cmap.map(*c as u32).is_none())
            .collect()
    }

    /// 仓库自洽字形覆盖（机器无关，解析级）：三内嵌字体联合覆盖全部 UI
    /// 字符集；且链装配必须包含三字体——任何系统字体缺失都不影响渲染。
    #[test]
    fn embedded_fonts_cover_all_ui_glyphs() {
        const ALL: &str = "天地玄黄宇宙洪荒한국어日本語 ABZdef0123456789✓✗●▲▼◆→←◎▪LiveTranslate%,:.";
        // 联合覆盖（= 渲染链覆盖）：任一字符不被三字体之一包含即失败
        let mut missing: Vec<char> = ALL
            .chars()
            .filter(|c| {
                ![EMBEDDED_BYTES, MONO_BYTES, SYMBOLS_BYTES]
                    .iter()
                    .any(|b| font_missing(b, &c.to_string()).is_empty())
            })
            .collect();
        assert!(
            missing.is_empty(),
            "三内嵌字体联合覆盖缺失: {}",
            missing.drain(..).collect::<String>()
        );
        // 语义职责：思源=中/英/韩；等宽=chrome 拉丁/数字/中文；符号=✗（思源所缺）
        assert!(
            font_missing(EMBEDDED_BYTES, "天地玄黄宇宙洪荒한국어日本語 ABZdef0123456789").is_empty(),
            "思源应覆盖中英韩"
        );
        assert!(
            font_missing(MONO_BYTES, "LiveTranslate 0123456789 。").is_empty(),
            "等宽应覆盖 chrome"
        );
        assert!(
            font_missing(SYMBOLS_BYTES, "✓✗●").is_empty(),
            "符号应覆盖 ✓✗●（旧实现依赖系统 seguisym.ttf 的理由）"
        );
        // 链装配：三条链必须各自包含对应内嵌字体（结构保证）
        let settings = Settings::default();
        let mut fonts = FontsState::default();
        let defs = build_definitions(&settings, &mut fonts);
        let prop = &defs.families[&FontFamily::Proportional];
        assert!(prop.iter().any(|k| k == EMBEDDED_KEY));
        assert!(prop.iter().any(|k| k == SYMBOLS_KEY));
        let mono = &defs.families[&FontFamily::Monospace];
        assert!(mono.iter().any(|k| k == MONO_KEY));
        assert!(mono.iter().any(|k| k == EMBEDDED_KEY));
    }

    /// 等宽 chrome 覆盖测试（机器无关）：无 Consolas 的系统也须由内嵌
    /// MonoCJK 提供拉丁/数字/中文等宽渲染（仓库自洽第二条保证）。
    #[test]
    fn embedded_mono_covers_chrome() {
        const MONO_SAMPLE: &str = "LiveTranslate ASR 0123456789 时秒MB CPU%";
        let ctx = egui::Context::default();
        let settings = Settings::default();
        let mut fonts = FontsState::default();
        apply_fonts(&ctx, &settings, &mut fonts);
        ctx.begin_pass(egui::RawInput::default());
        let ok = ctx.fonts_mut(|f| f.has_glyphs(&FontId::monospace(16.0), MONO_SAMPLE));
        assert!(ok, "内嵌等宽链应覆盖 chrome 样例: {MONO_SAMPLE}");
        // 默认链装配：等宽链第三位之前应含内嵌等宽（在无 Consolas 时仍兜底）
        let defs = build_definitions(&settings, &mut FontsState::default());
        let mono = &defs.families[&FontFamily::Monospace];
        assert!(mono.iter().any(|k| k == MONO_KEY));
    }

    /// 默认设置下链装配：内嵌为 Proportional 链首；命名族已注册；resolved 语义正确。
    #[test]
    fn build_definitions_default_chain_layout() {
        let settings = Settings::default();
        let mut fonts = FontsState::default();
        let defs = build_definitions(&settings, &mut fonts);

        let prop = &defs.families[&FontFamily::Proportional];
        assert_eq!(prop[0], EMBEDDED_KEY, "默认界面字体=内嵌思源，链首要命中它");
        assert!(prop.iter().any(|k| k == EMBEDDED_KEY));

        let name = FontFamily::Name(Arc::from(EMBEDDED_FAMILY));
        assert!(
            defs.families.contains_key(&name),
            "默认字幕主字体=内嵌，应注册其命名族供预览/字幕渲染"
        );

        // resolved：行级/主键指向命名族；未知名回落 Proportional
        assert_eq!(
            fonts.resolved[EMBEDDED_FAMILY],
            FontFamily::Name(Arc::from(EMBEDDED_FAMILY))
        );
        assert_eq!(font_family_for("不存在的族", &fonts), FontFamily::Proportional);
    }

    /// 行级键解析 + Name 注册/失败回落：设置一个系统不存在的行级字体。
    #[test]
    fn unknown_line_family_falls_back_to_global() {
        let mut settings = Settings::default();
        settings.style.original_font_family = "Bogus-Sans-Zzz".into();
        let mut fonts = FontsState::default();
        let defs = build_definitions(&settings, &mut fonts);
        let name = FontFamily::Name(Arc::from("Bogus-Sans-Zzz"));
        assert!(!defs.families.contains_key(&name), "未注册不落 Name 链");
        assert_eq!(
            font_family_for("Bogus-Sans-Zzz", &fonts),
            FontFamily::Proportional
        );
        // 行级空串跟随时解析到字幕主字体（默认内嵌）
        assert_eq!(
            font_family_for(resolve_family("", &settings.subtitle_font_family), &fonts),
            FontFamily::Name(Arc::from(EMBEDDED_FAMILY))
        );
    }
}
