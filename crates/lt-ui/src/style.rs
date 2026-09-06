//! 悬浮窗样式 14+1 预设全表（对照原版 subtitle_overlay.py DEFAULT_STYLE / STYLE_PRESETS）。
//!
//! settings.style 持全字段（serde default 补齐 = 原版 `{**DEFAULT_STYLE, **saved}` 合并）；
//! preset 键只作"当前预设名"标记，面板切换预设 = 整组字段覆写（M4.3）。

use egui::Color32;
use lt_proto::Style;

/// 预设覆盖表：预设名 → 覆写字段（未列出的字段 = BASE 默认值）。
/// 顺序即面板下拉顺序（default 之外的 13 个主题 + compact）。
pub const PRESET_OVERRIDES: &[(&str, fn(&mut Style))] = &[
    (
        "transparent",
        |s| {
            s.bg_opacity = 120;
            s.header_opacity = 120;
            s.window_opacity = 70;
        },
    ),
    (
        "compact",
        |s| {
            s.original_font_size = 9;
            s.translation_font_size = 11;
        },
    ),
    (
        "light",
        |s| {
            s.bg_color = "#e8e8f0".into();
            s.bg_opacity = 230;
            s.header_color = "#c8c8d8".into();
            s.header_opacity = 220;
            s.original_color = "#333333".into();
            s.translation_color = "#111111".into();
            s.timestamp_color = "#666688".into();
        },
    ),
    (
        "dracula",
        |s| {
            s.bg_color = "#282a36".into();
            s.bg_opacity = 235;
            s.header_color = "#44475a".into();
            s.header_opacity = 230;
            s.original_color = "#f8f8f2".into();
            s.translation_color = "#f8f8f2".into();
            s.timestamp_color = "#6272a4".into();
        },
    ),
    (
        "nord",
        |s| {
            s.bg_color = "#2e3440".into();
            s.bg_opacity = 235;
            s.header_color = "#3b4252".into();
            s.header_opacity = 230;
            s.original_color = "#d8dee9".into();
            s.translation_color = "#eceff4".into();
            s.timestamp_color = "#4c566a".into();
        },
    ),
    (
        "monokai",
        |s| {
            s.bg_color = "#272822".into();
            s.bg_opacity = 235;
            s.header_color = "#3e3d32".into();
            s.header_opacity = 230;
            s.original_color = "#f8f8f2".into();
            s.translation_color = "#f8f8f2".into();
            s.timestamp_color = "#75715e".into();
        },
    ),
    (
        "solarized",
        |s| {
            s.bg_color = "#002b36".into();
            s.bg_opacity = 235;
            s.header_color = "#073642".into();
            s.header_opacity = 230;
            s.original_color = "#839496".into();
            s.translation_color = "#eee8d5".into();
            s.timestamp_color = "#586e75".into();
        },
    ),
    (
        "gruvbox",
        |s| {
            s.bg_color = "#282828".into();
            s.bg_opacity = 235;
            s.header_color = "#3c3836".into();
            s.header_opacity = 230;
            s.original_color = "#ebdbb2".into();
            s.translation_color = "#fbf1c7".into();
            s.timestamp_color = "#928374".into();
        },
    ),
    (
        "tokyo_night",
        |s| {
            s.bg_color = "#1a1b26".into();
            s.bg_opacity = 235;
            s.header_color = "#24283b".into();
            s.header_opacity = 230;
            s.original_color = "#a9b1d6".into();
            s.translation_color = "#c0caf5".into();
            s.timestamp_color = "#565f89".into();
        },
    ),
    (
        "catppuccin",
        |s| {
            s.bg_color = "#1e1e2e".into();
            s.bg_opacity = 235;
            s.header_color = "#313244".into();
            s.header_opacity = 230;
            s.original_color = "#cdd6f4".into();
            s.translation_color = "#cdd6f4".into();
            s.timestamp_color = "#6c7086".into();
        },
    ),
    (
        "one_dark",
        |s| {
            s.bg_color = "#282c34".into();
            s.bg_opacity = 235;
            s.header_color = "#3e4452".into();
            s.header_opacity = 230;
            s.original_color = "#abb2bf".into();
            s.translation_color = "#e5c07b".into();
            s.timestamp_color = "#636d83".into();
        },
    ),
    (
        "everforest",
        |s| {
            s.bg_color = "#2d353b".into();
            s.bg_opacity = 235;
            s.header_color = "#343f44".into();
            s.header_opacity = 230;
            s.original_color = "#d3c6aa".into();
            s.translation_color = "#d3c6aa".into();
            s.timestamp_color = "#859289".into();
        },
    ),
    (
        "kanagawa",
        |s| {
            s.bg_color = "#1f1f28".into();
            s.bg_opacity = 235;
            s.header_color = "#2a2a37".into();
            s.header_opacity = 230;
            s.original_color = "#dcd7ba".into();
            s.translation_color = "#dcd7ba".into();
            s.timestamp_color = "#54546d".into();
        },
    ),
];

/// 全部预设名（含 "default"）
pub const PRESET_NAMES: &[&str] =
    &["default", "transparent", "compact", "light", "dracula", "nord", "monokai",
      "solarized", "gruvbox", "tokyo_night", "catppuccin", "one_dark", "everforest", "kanagawa"];

/// 按预设名从 BASE 生成完整 Style（面板切换预设用，M4.3 消费）
pub fn preset_style(name: &str) -> Style {
    let mut s = Style::default(); // = BASE(default)
    s.preset = name.to_string();
    if let Some((_, apply)) = PRESET_OVERRIDES.iter().find(|(n, _)| *n == name) {
        apply(&mut s);
    }
    s
}

// ── 全局控件三态稳定（WP-B，消 hover 文字微移） ──

/// egui 0.36 按钮内边距公式 = `button_padding − bg_stroke.width`
/// （egui widget_style.rs button_style）。dark/light 默认三态描边宽度不一致
/// （inactive 0 / hovered·active 1）→ hover 时内容区每边扩 1px、文字横移
/// 0.5px；fg_stroke 宽 1.0→1.5 还会让文字 hover 变粗。
///
/// 以 inactive 态为基准把 hovered/active 的描边与字重拉齐：静止外观不变，
/// hover 反馈只剩底色/字色变化（对齐原版 QSS hover 语义 = 只变底色字色，
/// 不动布局不变粗）。
pub fn stabilize_widget_strokes(v: &mut egui::Visuals) {
    let stroke_w = v.widgets.inactive.bg_stroke.width;
    v.widgets.hovered.bg_stroke.width = stroke_w;
    v.widgets.active.bg_stroke.width = stroke_w;
    let fg_w = v.widgets.inactive.fg_stroke.width;
    v.widgets.hovered.fg_stroke.width = fg_w;
    v.widgets.active.fg_stroke.width = fg_w;
    let radius = v.widgets.inactive.corner_radius;
    v.widgets.hovered.corner_radius = radius;
    v.widgets.active.corner_radius = radius;
}

// ── 原版字面强调色（不随预设变化） ──

/// 源语言标签（原版 #6cf）
pub const LANG_BLUE: Color32 = Color32::from_rgb(0x66, 0xcc, 0xff);
/// ASR 耗时（原版 #8b8）
pub const ASR_MS: Color32 = Color32::from_rgb(0x88, 0xbb, 0x88);
/// TL 耗时（原版 #db8）
pub const TL_MS: Color32 = Color32::from_rgb(0xdd, 0xbb, 0x88);
/// "翻译中"/同语言占位（原版 #999 / #aaa）
pub const PLACEHOLDER: Color32 = Color32::from_rgb(0x99, 0x99, 0x99);
pub const SAME_LANG: Color32 = Color32::from_rgb(0xaa, 0xaa, 0xaa);

/// #RRGGBB → Color32；解析失败回退 fallback（原版 _hex_to_rgba 的容错路径）
pub fn parse_color(s: &str, fallback: Color32) -> Color32 {
    Color32::from_hex(s).unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_count_and_names_match_original() {
        // 原版 STYLE_PRESETS：default + 13 个变体
        assert_eq!(PRESET_NAMES.len(), 14);
        assert_eq!(PRESET_OVERRIDES.len(), 13);
        assert_eq!(preset_style("default").preset, "default");
    }

    #[test]
    fn transparent_overrides_only_opacity_fields() {
        let s = preset_style("transparent");
        assert_eq!(s.bg_opacity, 120);
        assert_eq!(s.header_opacity, 120);
        assert_eq!(s.window_opacity, 70);
        // 其余保持 BASE
        assert_eq!(s.bg_color, "#000000");
        assert_eq!(s.original_font_size, 11);
    }

    #[test]
    fn light_theme_colors() {
        let s = preset_style("light");
        assert_eq!(s.bg_color, "#e8e8f0");
        assert_eq!(s.translation_color, "#111111");
        assert_eq!(s.window_opacity, 95);
    }

    #[test]
    fn compact_shrinks_fonts_only() {
        let s = preset_style("compact");
        assert_eq!(s.original_font_size, 9);
        assert_eq!(s.translation_font_size, 11);
        assert_eq!(s.bg_color, "#000000");
    }
}
