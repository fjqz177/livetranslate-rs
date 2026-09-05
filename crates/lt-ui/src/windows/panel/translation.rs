//! 翻译页（对照原版 ui/panel/tabs/translation_tab.py + dialogs.py ModelEditDialog）：
//! 模型列表（Consolas 双行摘要 + 添加/编辑/复制/删除 + 双击行编辑）、
//! ModelEditDialog（Basic/Advanced 两段：proxy 三模式、行为复选、thinking_style
//! 下拉、价格、overrides checkbox+value 六行、extra_body JSON 校验）、
//! prompt 预设（4 预设 + 自定义）+ system_prompt 600ms 防抖、网络超时。
//!
//! 与原版的差异：
//! - `no_think` 复选由 `thinking_style` 下拉承载（settings 契约经 legacy 迁移，
//!   lt_translate::thinking::THINKING_STYLES 同款六值）；
//! - `json_schema_mode` 复选不做（lt-proto 契约无该键，不得私改）；
//! - 编辑确定 = 写回 models 行 + 防抖 ApplySettings（落盘）；
//!   编辑/选中活动模型另发即时命令 Cmd::SwitchTranslator（shell 已路由）；
//! - 删除模型时若被删行在活动模型之前，active_model 前移一位
//!   （原版仅做越界钳制，行前移会错位指向别的模型——有意修正）。

use super::{group_card, hint_line, mark_settings_dirty, Palette};
use crate::state::{AppState, ModelEditState, THINKING_STYLE_VALUES};
use egui::{RichText, Ui};
use lt_proto::ModelConfig;

/// prompt 预设下拉 i18n 键（daily/esports/anime/webid/custom；顺序同
/// lt_translate::PROMPT_PRESETS，末位 custom）
pub const PROMPT_PRESET_KEYS: [&str; 5] =
    ["prompt_daily", "prompt_esports", "prompt_anime", "prompt_webid", "prompt_custom"];

/// overrides 六行 i18n 键（原版 adv_layout addRow 顺序）
const OVERRIDE_LABEL_KEYS: [&str; 6] = [
    "label_temperature",
    "label_top_p",
    "label_max_tokens",
    "label_frequency_penalty",
    "label_presence_penalty",
    "label_seed",
];

/// 高级参数行数值范围/步进（原版 QDoubleSpinBox/QSpinBox 设置）：
/// (min, max, step)；整数行以 step=1.0 表达
const OVERRIDE_RANGES: [(f64, f64, f64); 6] = [
    (0.0, 2.0, 0.1),   // temperature
    (0.0, 1.0, 0.05),  // top_p
    (1.0, 32768.0, 1.0), // max_tokens
    (-2.0, 2.0, 0.1),  // frequency_penalty
    (-2.0, 2.0, 0.1),  // presence_penalty
    (0.0, 2_000_000_000.0, 1.0), // seed
];

/// 高级参数行是否整数语义（QSpinBox）
fn override_is_int(i: usize) -> bool {
    matches!(i, 2 | 5) // max_tokens / seed
}

// ── 纯逻辑（单测覆盖） ──

/// 模型列表行摘要（对照 refresh_model_list：活动行 ">>> " 前缀 + 加粗；
/// proxy 非 none 时追加 "  [proxy: url]"；第二行 "     api_base  |  model"）
pub fn model_row_text(index: usize, active: usize, m: &ModelConfig) -> String {
    let prefix = if index == active { ">>> " } else { "    " };
    let proxy_tag = if m.proxy != "none" { format!("  [proxy: {}]", m.proxy) } else { String::new() };
    format!("{prefix}{}{proxy_tag}\n     {}  |  {}", m.name, m.api_base, m.model)
}

/// system_prompt → 预设下拉索引（原版构造函数语义：精确匹配 4 预设 → 对应项；
/// 空/DEFAULT_PROMPT → daily(0)；其余 → custom(4)）
pub fn prompt_preset_index(text: &str) -> usize {
    let t = text.trim();
    if t.is_empty() || t == lt_translate::DEFAULT_PROMPT.trim() {
        return 0;
    }
    for (i, (_, body)) in lt_translate::PROMPT_PRESETS.iter().enumerate() {
        if t == body.trim() {
            return i;
        }
    }
    4
}

/// 删除选中模型（原版 _remove_model：仅一行时不删；active 越界钳制；
/// Rust 增量：被删行在 active 之前时 active 前移，保持指向同一模型）。
/// 返回是否删除。
pub fn remove_model(models: &mut Vec<ModelConfig>, active: &mut usize, row: usize) -> bool {
    if models.len() <= 1 || row >= models.len() {
        return false;
    }
    models.remove(row);
    if *active >= models.len() {
        *active = models.len() - 1;
    } else if row < *active {
        *active -= 1;
    }
    true
}

/// 复制选中模型到表尾（原版 _dup_model：name + " (copy)"）。
/// 返回新行号（row 越界 → None）。
pub fn duplicate_model(models: &mut Vec<ModelConfig>, row: usize) -> Option<usize> {
    let mut dup = models.get(row)?.clone();
    dup.name.push_str(" (copy)");
    models.push(dup);
    Some(models.len() - 1)
}

// ── UI ──

/// 翻译页 UI 总入口
pub fn page(ui: &mut Ui, state: &mut AppState, pal: &Palette) {
    // ── 模型配置（原版 models_group）──
    group_card(ui, pal, &lt_i18n::t("group_model_configs"), |ui| {
        let active = state.settings.active_model;
        let count = state.settings.models.len();
        let mut select: Option<usize> = None;
        let mut edit_row: Option<usize> = None;
        for i in 0..count {
            let text = model_row_text(i, active, &state.settings.models[i]);
            let mut rich = RichText::new(&text).monospace().size(12.0);
            if i == active {
                rich = rich.strong();
            }
            let resp = ui
                .push_id(i, |ui| {
                    ui.add(
                        egui::Button::selectable(state.panel.model_selected == Some(i), rich)
                            .corner_radius(4.0)
                            .min_size(egui::vec2(ui.available_width(), 0.0)),
                    )
                })
                .inner;
            if resp.clicked() {
                select = Some(i);
            }
            // 双击行 = 编辑（原版 itemDoubleClicked → _on_model_double_clicked）
            if resp.double_clicked() {
                edit_row = Some(i);
            }
        }

        // 行选中 → active_model 跟随并即时切换翻译器（原版"当前模型"语义）
        if let Some(i) = select {
            state.panel.model_selected = Some(i);
            if i != state.settings.active_model && i < state.settings.models.len() {
                state.settings.active_model = i;
                if let Some(cfg) = super::active_model_config(&state.settings) {
                    state.send_cmd(lt_proto::Cmd::SwitchTranslator(Box::new(cfg)));
                }
                mark_settings_dirty(state);
            }
        }

        ui.add_space(4.0);
        // 四按钮行（原版 btn_row：添加/编辑/复制/删除）
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(RichText::new(lt_i18n::t("btn_add")).size(12.5)).corner_radius(6.0))
                .clicked()
            {
                state.panel.model_editor = Some(ModelEditState::new_add());
            }
            let edit_target = edit_row.or(state.panel.model_selected);
            if ui
                .add_enabled(
                    edit_target.is_some(),
                    egui::Button::new(RichText::new(lt_i18n::t("btn_edit")).size(12.5)).corner_radius(6.0),
                )
                .clicked()
            {
                if let Some(i) = edit_target {
                    if i < state.settings.models.len() {
                        let cfg = state.settings.models[i].clone();
                        state.panel.model_editor = Some(ModelEditState::new_edit(i, &cfg));
                    }
                }
            }
            if ui
                .add_enabled(
                    state.panel.model_selected.is_some(),
                    egui::Button::new(RichText::new(lt_i18n::t("btn_duplicate")).size(12.5))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                if let Some(i) = state.panel.model_selected {
                    if duplicate_model(&mut state.settings.models, i).is_some() {
                        mark_settings_dirty(state);
                    }
                }
            }
            let can_remove = state.settings.models.len() > 1 && state.panel.model_selected.is_some();
            if ui
                .add_enabled(
                    can_remove,
                    egui::Button::new(RichText::new(lt_i18n::t("btn_remove")).size(12.5)).corner_radius(6.0),
                )
                .clicked()
            {
                if let Some(i) = state.panel.model_selected {
                    let mut active = state.settings.active_model;
                    if remove_model(&mut state.settings.models, &mut active, i) {
                        state.settings.active_model = active;
                        state.panel.model_selected = None;
                        // active 可能变化 → 重建翻译器（原版 _emit_models_list_changed 面）
                        if let Some(cfg) = super::active_model_config(&state.settings) {
                            state.send_cmd(lt_proto::Cmd::SwitchTranslator(Box::new(cfg)));
                        }
                        mark_settings_dirty(state);
                    }
                }
            }
        });
    });

    // ── 系统提示词（原版 prompt_group）──
    group_card(ui, pal, &lt_i18n::t("group_system_prompt"), |ui| {
        // 预设下拉（原版 _prompt_preset：4 预设精确匹配；DEFAULT_PROMPT → daily）
        let cur = prompt_preset_index(&state.settings.system_prompt);
        let mut next = cur;
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ", lt_i18n::t("label_prompt_preset"))).color(pal.text));
            egui::ComboBox::from_id_salt("panel_prompt_preset")
                .selected_text(lt_i18n::t(PROMPT_PRESET_KEYS[cur]))
                .width(220.0)
                .show_ui(ui, |ui| {
                    for (i, key) in PROMPT_PRESET_KEYS.iter().enumerate() {
                        if ui.selectable_label(cur == i, lt_i18n::t(key)).clicked() && cur != i {
                            next = i;
                        }
                    }
                });
        });
        if next != cur && next < 4 {
            // 原版 _on_prompt_preset_changed：写入预设文本并立即应用
            state.settings.system_prompt = lt_translate::PROMPT_PRESETS[next].1.to_string();
            state.schedule_prompt_apply();
            mark_settings_dirty(state);
        }
        ui.add_space(2.0);
        // 多行编辑（Consolas 等宽；变更 → 600ms 防抖 SwitchTranslator + 300ms 落盘）
        let resp = ui.add(
            egui::TextEdit::multiline(&mut state.settings.system_prompt)
                .hint_text(lt_translate::DEFAULT_PROMPT)
                .desired_width(f32::INFINITY)
                .min_size(egui::vec2(0.0, 88.0))
                .font(egui::TextStyle::Monospace),
        );
        if resp.changed() {
            state.schedule_prompt_apply();
            mark_settings_dirty(state);
        }
    });

    // ── 网络配置（原版 net_group：超时 1-60s；ApplySettings 热应用）──
    group_card(ui, pal, &lt_i18n::t("group_network"), |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ", lt_i18n::t("label_timeout"))).color(pal.text));
            let mut v = state.settings.timeout.clamp(1, 60) as f32;
            let resp = ui.add(
                egui::DragValue::new(&mut v)
                    .range(1.0..=60.0)
                    .speed(1.0)
                    .suffix(" s"),
            );
            if resp.changed() {
                state.settings.timeout = v.round() as u32;
                mark_settings_dirty(state);
            }
        });
        hint_line(ui, pal, &lt_i18n::t("context_turns_hint"));
    });

    ui.add_space(8.0);

    // ── ModelEditDialog（egui::Window 居中模态区，对照 dialogs.py）──
    render_model_editor(ui, state, pal);
}

/// ModelEditDialog 模态区（打开中每帧渲染；确定/取消/关闭由 outcome 收敛）
fn render_model_editor(ui: &mut Ui, state: &mut AppState, pal: &Palette) {
    if state.panel.model_editor.is_none() {
        return;
    }
    let mut open = true;
    let mut cancel = false;
    // Some(cfg) = 确定；None + 窗关闭/取消 = 放弃编辑
    let mut accepted: Option<ModelConfig> = None;
    {
        let panel = &mut state.panel;
        let Some(ed) = panel.model_editor.as_mut() else { return };
        let title = if ed.is_new { lt_i18n::t("dialog_add_model") } else { lt_i18n::t("dialog_edit_model") };
        let extra_valid = ed.parse_extra_body().is_ok();
        egui::Window::new(RichText::new(title).strong())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(600.0)
            .show(ui.ctx(), |ui| {
                egui::ScrollArea::vertical()
                    .max_height(480.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        editor_fields(ui, ed, pal);
                    });
                ui.add_space(6.0);
                ui.separator();
                // 确定按钮：extra_body 非法时禁用 + 红字（原版 _on_accept 拒绝语义前移）
                ui.horizontal(|ui| {
                    if let Err(e) = ed.parse_extra_body() {
                        ui.label(
                            RichText::new(format!("{}: {e}", lt_i18n::t("extra_body_invalid")))
                                .size(11.0)
                                .color(pal.err),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                extra_valid,
                                egui::Button::new(RichText::new(lt_i18n::t("common_ok"))),
                            )
                            .clicked()
                        {
                            if let Ok(cfg) = ed.build() {
                                accepted = Some(cfg);
                            }
                        }
                        if ui.button(lt_i18n::t("subwin_cancel")).clicked() {
                            cancel = true;
                        }
                    });
                });
            });
    }
    if let Some(cfg) = accepted {
        let ed = state.panel.model_editor.take().expect("编辑器打开中");
        apply_editor_result(state, cfg, ed.index, ed.is_new);
    } else if !open || cancel {
        state.panel.model_editor = None;
    }
}

/// 对话框确定后的写回（原版 _add_model/_edit_model：name+model 非空才收；
/// 编辑活动模型 → 即时 SwitchTranslator；统一防抖落盘）
fn apply_editor_result(state: &mut AppState, cfg: ModelConfig, index: usize, is_new: bool) {
    if cfg.name.is_empty() || cfg.model.is_empty() {
        return; // 原版 get_data 后的 if data["name"] and data["model"] 守卫
    }
    if is_new {
        state.settings.models.push(cfg);
        state.panel.model_selected = Some(state.settings.models.len() - 1);
    } else {
        let idx = index.min(state.settings.models.len() - 1);
        let was_active = idx == state.settings.active_model;
        state.settings.models[idx] = cfg.clone();
        state.panel.model_selected = Some(idx);
        if was_active {
            state.send_cmd(lt_proto::Cmd::SwitchTranslator(Box::new(cfg)));
        }
    }
    mark_settings_dirty(state);
}

/// 对话框字段全集（Basic + Advanced，原版 QFormLayout 逐行）
fn editor_fields(ui: &mut Ui, ed: &mut ModelEditState, pal: &Palette) {
    // ── Basic ──
    egui::Grid::new("model_edit_basic")
        .num_columns(2)
        .spacing([8.0, 5.0])
        .min_col_width(120.0)
        .show(ui, |ui| {
            text_row(ui, "label_display_name", &mut ed.name, false);
            text_row(ui, "label_api_base", &mut ed.api_base, false);
            text_row(ui, "label_api_key", &mut ed.api_key, true);
            text_row(ui, "label_model", &mut ed.model, false);
            ui.end_row();

            // 代理三模式 + URL（原版 _proxy_mode / _proxy_url）
            ui.label(lt_i18n::t("label_proxy"));
            let modes = [lt_i18n::t("proxy_none"), lt_i18n::t("proxy_system"), lt_i18n::t("proxy_custom")];
            egui::ComboBox::from_id_salt("model_edit_proxy_mode")
                .selected_text(modes[ed.proxy_index].clone())
                .width(240.0)
                .show_ui(ui, |ui| {
                    for (i, label) in modes.iter().enumerate() {
                        if ui.selectable_label(ed.proxy_index == i, label.clone()).clicked()
                            && ed.proxy_index != i
                        {
                            ed.proxy_index = i; // 原版 _on_proxy_mode_changed：URL 仅 custom 可编辑
                        }
                    }
                });
            ui.end_row();
            ui.label(lt_i18n::t("label_proxy_url"));
            ui.add_enabled(
                ed.proxy_index == 2,
                egui::TextEdit::singleline(&mut ed.proxy_url)
                    .hint_text("http://127.0.0.1:7890")
                    .desired_width(240.0),
            );
            ui.end_row();

            // thinking_style 下拉（原版 no_think 复选的后继形态）
            ui.label(lt_i18n::t("label_thinking_style"));
            let styles = THINKING_STYLE_VALUES
                .iter()
                .map(|v| lt_i18n::t(&format!("thinking_style_{v}")))
                .collect::<Vec<_>>();
            egui::ComboBox::from_id_salt("model_edit_thinking")
                .selected_text(styles[ed.thinking_index].clone())
                .width(240.0)
                .show_ui(ui, |ui| {
                    for (i, label) in styles.iter().enumerate() {
                        if ui.selectable_label(ed.thinking_index == i, label.clone()).clicked()
                            && ed.thinking_index != i
                        {
                            ed.thinking_index = i;
                        }
                    }
                });
            ui.end_row();

            // 价格（原版 price_row：输入/输出 $/1M，0 显示 "—"）
            ui.label(lt_i18n::t("label_pricing"));
            ui.horizontal(|ui| {
                ui.label(lt_i18n::t("label_input_price"));
                price_drag(ui, "model_edit_ip", &mut ed.input_price);
                ui.label(lt_i18n::t("label_output_price"));
                price_drag(ui, "model_edit_op", &mut ed.output_price);
            });
            ui.end_row();

            // 上下文数（原版 _context_turns 0-20）
            ui.label(lt_i18n::t("label_context_turns"));
            let mut turns = ed.context_turns;
            if ui.add(egui::DragValue::new(&mut turns).range(0..=20)).changed() {
                ed.context_turns = turns;
            }
            ui.end_row();
        });

    // 行为复选（原版 addRow 顺序：streaming / json_response / no_system_role）
    ui.add_space(2.0);
    checkbox_row(ui, "model_edit_streaming", &mut ed.streaming, "streaming", "streaming_hint");
    checkbox_row(ui, "model_edit_json", &mut ed.json_response, "json_response", "json_response_hint");
    checkbox_row(
        ui,
        "model_edit_nsr",
        &mut ed.no_system_role,
        "no_system_role",
        "no_system_role_hint",
    );

    // ── Advanced（原版 adv_group：override_hint tooltip 语义移到组头提示行）──
    ui.add_space(6.0);
    ui.label(RichText::new(lt_i18n::t("label_advanced_params")).strong());
    hint_line(ui, pal, &lt_i18n::t("override_hint"));
    egui::Grid::new("model_edit_adv")
        .num_columns(2)
        .spacing([8.0, 5.0])
        .min_col_width(120.0)
        .show(ui, |ui| {
            for (i, (label_key, row)) in OVERRIDE_LABEL_KEYS.iter().zip(ed.overrides.iter_mut()).enumerate() {
                ui.label(lt_i18n::t(label_key));
                ui.horizontal(|ui| {
                    let mut current = *row;
                    if ui.checkbox(&mut current.enabled, lt_i18n::t("override_enable")).changed() {
                        *row = current;
                    }
                    ui.add_enabled(current.enabled, override_drag(i, &mut row.value));
                });
                ui.end_row();
            }
            ui.label(lt_i18n::t("label_extra_body"));
            ui.add(
                egui::TextEdit::multiline(&mut ed.extra_body_text)
                    .hint_text(r#"{"thinking": {"type": "disabled"}}"#)
                    .desired_width(300.0)
                    .desired_rows(2)
                    .font(egui::TextStyle::Monospace),
            )
            .on_hover_text(lt_i18n::t("extra_body_hint"));
            ui.end_row();
        });
}

/// 单行文本框行（label key + 输入；password=true 时掩码显示）
fn text_row(ui: &mut Ui, label_key: &str, value: &mut String, password: bool) {
    ui.label(lt_i18n::t(label_key));
    ui.add(
        egui::TextEdit::singleline(value)
            .password(password)
            .desired_width(240.0),
    );
    ui.end_row();
}

/// 复选行（label = 复选文本；hover 提示 = hint 键）
fn checkbox_row(ui: &mut Ui, id: &str, value: &mut bool, label_key: &str, hint_key: &str) {
    ui.push_id(id, |ui| {
        ui.add(egui::Checkbox::new(value, lt_i18n::t(label_key)))
            .on_hover_text(lt_i18n::t(hint_key))
    });
}

/// 价格输入（0..=999 两位小数；0 显示 "—"，原版 setSpecialValueText）
fn price_drag(ui: &mut Ui, id: &str, value: &mut f64) {
    ui.push_id(id, |ui| {
        let mut v = *value;
        let resp = ui
            .add(
                egui::DragValue::new(&mut v)
                    .range(0.0..=999.0)
                    .speed(0.1)
                    .fixed_decimals(2)
                    .custom_formatter(|v, _| if v <= 0.0 { "—".into() } else { format!("{v:.2}") }),
            )
            .changed();
        if resp {
            *value = v;
        }
    });
}

/// 高级参数行 DragValue（依行序取范围/步进/整数语义）
fn override_drag<'a>(i: usize, value: &'a mut f64) -> egui::DragValue<'a> {
    let (min, max, step) = OVERRIDE_RANGES[i];
    let mut dv = egui::DragValue::new(value).range(min..=max).speed(step);
    if override_is_int(i) {
        dv = dv.fixed_decimals(0);
    } else {
        dv = dv.fixed_decimals(2);
    }
    dv
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::OverrideRow;
    use std::collections::BTreeMap;

    /// 序列化快照：完整 ModelConfig → serde_json 键集合与原版 get_data 形状一致
    /// （额外键不出现；false/0/auto 缺省不出现）
    #[test]
    fn model_config_serialization_snapshot_matches_original_get_data() {
        let mut ed = ModelEditState::new_add();
        ed.name = "deepseek".into();
        ed.api_base = "https://api.deepseek.com/v1".into();
        ed.api_key = "sk-test".into();
        ed.model = "deepseek-chat".into();
        ed.proxy_index = 2;
        ed.proxy_url = "http://127.0.0.1:7890".into();
        ed.thinking_index = 1; // deepseek
        ed.no_system_role = true;
        ed.streaming = false;
        ed.json_response = true;
        ed.context_turns = 4;
        ed.input_price = 0.27;
        ed.output_price = 1.1;
        ed.overrides[0] = OverrideRow { enabled: true, value: 0.7 }; // temperature
        ed.overrides[2] = OverrideRow { enabled: true, value: 512.0 }; // max_tokens
        ed.overrides[1] = OverrideRow { enabled: false, value: 0.9 }; // 未勾选不写
        ed.extra_body_text = r#"{"thinking": {"type": "disabled"}}"#.into();
        let cfg = ed.build().expect("extra_body 合法");

        let obj = serde_json::to_value(&cfg).unwrap().as_object().unwrap().clone();
        // serde_json::Map 默认按字典序存键 → 键集合按排序比较（契约只锁键集合）
        let mut keys: Vec<_> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "api_base",
                "api_key",
                "context_turns",
                "extra_body",
                "input_price",
                "json_response",
                "model",
                "name",
                "no_system_role",
                "output_price",
                "overrides",
                "proxy",
                "streaming",
                "thinking_style"
            ],
            "键集合应与原版 get_data 条件序列化形状一致（无额外键）"
        );
        // 契约冻结：额外键不出现
        assert!(obj.get("json_schema_mode").is_none(), "json_schema_mode 不在契约内");
        assert!(obj.get("no_think").is_none(), "no_think 已迁移为 thinking_style");
        // overrides 只含勾选行（BTreeMap 键序）
        assert_eq!(
            obj["overrides"],
            serde_json::json!({"max_tokens": 512, "temperature": 0.7})
        );
        // 反序列化往返（形状即契约）
        let round: ModelConfig = serde_json::from_value(serde_json::to_value(&cfg).unwrap()).unwrap();
        assert_eq!(round, cfg);
    }

    /// 默认模型序列化：可选键全部不出现（false/0/auto 缺省不写）
    #[test]
    fn default_model_config_omits_all_optional_keys() {
        let obj = serde_json::to_value(ModelConfig::default()).unwrap();
        let obj = obj.as_object().unwrap();
        for key in [
            "no_system_role",
            "thinking_style",
            "streaming",
            "json_response",
            "context_turns",
            "input_price",
            "output_price",
            "overrides",
            "extra_body",
        ] {
            assert!(!obj.contains_key(key), "默认模型不应写出 {key}");
        }
        // thinking_style=auto 同样不写（skip_thinking_auto）
        let auto = ModelConfig { thinking_style: Some("auto".into()), ..Default::default() };
        assert!(!serde_json::to_value(auto).unwrap().as_object().unwrap().contains_key("thinking_style"));
    }

    /// 模型列表摘要行（refresh_model_list 形状：前缀/proxy 标签/第二行）
    #[test]
    fn model_row_text_formats_like_original() {
        let plain = ModelConfig { name: "A".into(), api_base: "http://b".into(), model: "m1".into(), ..Default::default() };
        assert_eq!(model_row_text(0, 0, &plain), ">>> A\n     http://b  |  m1");
        assert_eq!(model_row_text(1, 0, &plain), "    A\n     http://b  |  m1");
        let proxied = ModelConfig { proxy: "http://p:7890".into(), name: "B".into(), api_base: "u".into(), model: "m2".into(), ..Default::default() };
        let text = model_row_text(0, 0, &proxied);
        assert!(text.contains(">>> B  [proxy: http://p:7890]"), "{text}");
        assert!(text.contains("\n     u  |  m2"));
        // system 代理同样带标签；none 不带
        assert!(model_row_text(0, 0, &ModelConfig { proxy: "system".into(), ..plain.clone() }).contains("[proxy: system]"));
        assert!(!model_row_text(0, 0, &plain).contains("[proxy"));
    }

    /// prompt 预设索引映射（空/DEFAULT → daily；精确匹配；其余 custom）
    #[test]
    fn prompt_preset_index_mapping() {
        assert_eq!(prompt_preset_index(""), 0, "空 = DEFAULT_PROMPT → daily");
        assert_eq!(prompt_preset_index(lt_translate::DEFAULT_PROMPT), 0);
        for (i, (_, body)) in lt_translate::PROMPT_PRESETS.iter().enumerate() {
            assert_eq!(prompt_preset_index(body), i, "预设 {i} 应精确匹配");
            // 原版按 trim 比较
            assert_eq!(prompt_preset_index(&format!("  {body}\n")), i);
        }
        assert_eq!(prompt_preset_index("You are a pirate."), 4, "其余 → custom");
        assert_eq!(PROMPT_PRESET_KEYS.len(), 5);
        assert_eq!(lt_translate::PROMPT_PRESETS.len(), 4);
    }

    /// 删除模型：单行守卫 + active 钳制 + 行前移修正
    #[test]
    fn remove_model_guards_and_active_adjustment() {
        let mk = |n: &str| ModelConfig { name: n.into(), ..Default::default() };
        let mut models = vec![mk("a"), mk("b"), mk("c")];
        let mut active = 2;
        assert!(remove_model(&mut models, &mut active, 0));
        assert_eq!((models.len(), active), (2, 1), "删行在 active 前 → active 前移");
        assert_eq!(models[active].name, "c", "active 仍指向 c");
        // active 越界钳制（原版语义）
        let mut models = vec![mk("a"), mk("b")];
        let mut active = 1;
        assert!(remove_model(&mut models, &mut active, 1));
        assert_eq!(active, 0);
        // 单行不可删；越界行不可删
        let mut models = vec![mk("only")];
        let mut active = 0;
        assert!(!remove_model(&mut models, &mut active, 0));
        let mut models = vec![mk("a"), mk("b")];
        assert!(!remove_model(&mut models, &mut active, 9));
    }

    /// 复制模型：表尾追加 + " (copy)" 后缀 + 越界 None
    #[test]
    fn duplicate_model_appends_copy() {
        let mk = |n: &str| ModelConfig { name: n.into(), model: "m".into(), ..Default::default() };
        let mut models = vec![mk("a"), mk("b")];
        assert_eq!(duplicate_model(&mut models, 0), Some(2));
        assert_eq!(models.len(), 3);
        assert_eq!(models[2].name, "a (copy)");
        assert_eq!(models[2].model, "m");
        assert_eq!(duplicate_model(&mut models, 9), None);
    }

    /// 编辑活动模型 → build 后原样回写（契约形状不漂移的回归锁）
    #[test]
    fn editor_edit_roundtrip_via_build() {
        let cfg = ModelConfig {
            name: "glm".into(),
            api_base: "https://open.bigmodel.cn/api/paas/v4".into(),
            api_key: "k".into(),
            model: "glm-4".into(),
            proxy: "system".into(),
            thinking_style: Some("deepseek".into()),
            overrides: Some(BTreeMap::from([("top_p".to_string(), serde_json::json!(0.9))])),
            ..Default::default()
        };
        let ed = ModelEditState::new_edit(0, &cfg);
        assert_eq!(ed.build().unwrap(), cfg);
    }

    /// 模态区无头渲染冒烟：ModelEditDialog 打开态整页面跑两帧不 panic
    #[test]
    fn model_editor_modal_smoke_renders_headless() {
        let ctx = egui::Context::default();
        let mut st = AppState::new(lt_proto::Settings::default());
        st.panel.page = crate::state::PanelPage::Translation;
        let mut ed = ModelEditState::new_edit(0, &st.settings.models[0]);
        ed.extra_body_text = "{\"a\": 1}".into();
        st.panel.model_editor = Some(ed);
        for _ in 0..2 {
            let mut out =
                ctx.run_ui(egui::RawInput::default(), |ui| crate::windows::panel::panel_ui(ui, &mut st));
            assert!(!out.shapes.is_empty(), "编辑器打开态应产出图元");
            out.textures_delta.clear();
        }
        // 确定按钮不在帧内点击；编辑器保持打开（状态未被意外消费）
        assert!(st.panel.model_editor.is_some());
    }
}
