//! 翻译页（对照原版 ui/panel/tabs/translation_tab.py + dialogs.py ModelEditDialog）：
//! 模型列表（Consolas 双行摘要 + 添加/编辑/复制/删除 + 双击行编辑）、
//! ModelEditDialog（Basic/Advanced 两段：proxy 三模式、行为复选、thinking_style
//! 下拉、价格、overrides checkbox+value 六行、extra_body JSON 校验）、
//! prompt 预设（4 预设 + 自定义）+ system_prompt 600ms 防抖、网络超时。
//!
//! 与原版的差异：
//! - `no_think` 复选由 `thinking_style` 下拉承载（settings 契约经 legacy 迁移，
//!   lt_proto::THINKING_STYLES 同款六值（E3 单一事实源））；
//! - `json_schema_mode` 复选不做（lt-proto 契约无该键，不得私改）；
//! - 编辑确定 = 写回 models 行 + 防抖 ApplySettings（落盘）；
//!   编辑/选中活动模型另发即时命令 Cmd::SwitchTranslator（shell 已路由）；
//! - 删除模型时若被删行在活动模型之前，active_model 前移一位
//!   （原版仅做越界钳制，行前移会错位指向别的模型——有意修正）。

use super::{group_card, hint_line, mark_settings_dirty, schedule_prompt_apply, Palette};
use crate::state::{ModalUi, ModelEditState, PanelUi, SessionView, Settings, TickKind, WinId};
use egui::{RichText, Ui};
use lt_proto::ModelConfig;
use std::time::{Duration, Instant};

/// 连接测试走秒/看门狗节拍间隔（D-85 §4.8：宿主节拍驱动，非 egui 自动重绘）
const PROBE_TICK_MS: u64 = 100;

/// prompt 预设下拉 i18n 键（daily/esports/anime/webid/custom；顺序同
/// lt_proto::PROMPT_PRESETS，末位 custom）
pub const PROMPT_PRESET_KEYS: [&str; 5] = [
    "prompt_daily",
    "prompt_esports",
    "prompt_anime",
    "prompt_webid",
    "prompt_custom",
];

/// 高级区仍渲染的覆写行（W1/方案 §2.2）：top_p / frequency_penalty /
/// presence_penalty——主流通用且翻译偶尔需要。temperature 已升为一等字段、
/// max_tokens 不再由应用发送、seed 冷门：三者不再渲染，但既有值经编辑器状态
/// （`ModelEditState::overrides`）原样保留，不丢用户数据。
const ADV_OVERRIDE_ROWS: [usize; 3] = [1, 3, 4];

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
    (0.0, 2.0, 0.1),             // temperature
    (0.0, 1.0, 0.05),            // top_p
    (1.0, 32768.0, 1.0),         // max_tokens
    (-2.0, 2.0, 0.1),            // frequency_penalty
    (-2.0, 2.0, 0.1),            // presence_penalty
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
    let proxy_tag = if m.proxy != "none" {
        format!("  [proxy: {}]", m.proxy)
    } else {
        String::new()
    };
    format!(
        "{prefix}{}{proxy_tag}\n     {}  |  {}",
        m.name, m.api_base, m.model
    )
}

/// system_prompt → 预设下拉索引（原版构造函数语义：精确匹配 4 预设 → 对应项；
/// 空/DEFAULT_PROMPT → daily(0)；其余 → custom(4)）
pub fn prompt_preset_index(text: &str) -> usize {
    let t = text.trim();
    if t.is_empty() || t == lt_proto::DEFAULT_PROMPT.trim() {
        return 0;
    }
    for (i, (_, body)) in lt_proto::PROMPT_PRESETS.iter().enumerate() {
        if t == body.trim() {
            return i;
        }
    }
    4
}

/// 模型配置就地校验提示（方案 §4.7 未落地项；2026-09-10 补齐）。
/// 语义边界：**只提示不阻断保存**、**绝不改写用户填的地址**——判断用
/// `trim` + 去尾斜杠后的副本，写回仍是用户原文。返回本地化提示文案
/// （空 = 无告警）。
fn config_warnings(ed: &ModelEditState) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // ── API 地址 ──
    let raw = ed.api_base.trim();
    if raw.is_empty() {
        out.push(lt_i18n::t("cfg_warn_api_base_empty"));
    } else {
        // 尾斜杠提示（原文判断；判断副本去尾斜杠以免与 /v1 提示叠加）
        if raw.ends_with('/') {
            out.push(lt_i18n::t("cfg_warn_api_base_slash"));
        }
        let base = raw.trim_end_matches('/');
        let lower = base.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            out.push(lt_i18n::t("cfg_warn_api_base_scheme"));
        }
        // D-85/J：地址与任一厂商预设**完全一致**时跳过"缺 /v1"提示——
        // 那是人家官方的形态（DeepSeek 就是 https://api.deepseek.com），
        // 预设填完立刻弹黄条会被当成 bug
        let is_official_preset = lt_proto::PROVIDER_PRESETS
            .iter()
            .any(|p| !p.api_base.is_empty() && p.api_base == base);
        if !has_version_segment(base) && !is_official_preset {
            out.push(lt_i18n::t("cfg_warn_api_base_v1"));
        }
    }
    // ── 模型名 ──
    if ed.model.trim().is_empty() {
        out.push(lt_i18n::t("cfg_warn_model_empty"));
    }
    // ── extra_body（类型化形态判定，不解析错误字符串）──
    match ed.extra_body_shape() {
        crate::state::ExtraBodyShape::Empty | crate::state::ExtraBodyShape::Object => {}
        crate::state::ExtraBodyShape::NonObject => {
            out.push(lt_i18n::t("cfg_warn_extra_body_nonobject"))
        }
        crate::state::ExtraBodyShape::Invalid => {
            out.push(lt_i18n::t("cfg_warn_extra_body_invalid"))
        }
    }
    out
}

/// 地址末段是否已是版本段（`/v1`、`/v4`、`/v1beta`…）。避免对厂商自有路径
/// （如 GLM `/api/paas/v4`）误报"缺 /v1"；不匹配时给软提示（不阻断）。
fn has_version_segment(base: &str) -> bool {
    let Some(seg) = base.rsplit('/').next() else {
        return false;
    };
    let mut chars = seg.chars();
    match chars.next() {
        Some('v' | 'V') => chars.next().is_some_and(|c| c.is_ascii_digit()),
        _ => false,
    }
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

// ── 页内数据模型（原版 dialog model 的 Rust 表达）──

/// 翻译页恢复默认：models 回默认单行（LM Studio 本地端点，含上下文数归 0）、
/// 清 prompt、timeout 回 10s；恢复后重发 SwitchTranslator（N3 生效管道）。
/// 破坏性（抹掉 API Key）——调用前必须已过确认框。
pub(crate) fn restore_translation_page(
    panel: &mut PanelUi,
    session: &mut SessionView,
    settings: &mut Settings,
) {
    let def = lt_proto::Settings::default();
    settings.models = def.models.clone();
    settings.active_model = 0;
    settings.system_prompt = def.system_prompt.clone();
    settings.timeout = def.timeout;
    panel.state.model_selected = None;
    panel.state.model_editor = None;
    if let Some(cfg) = super::active_model_config(settings) {
        session.send_cmd(lt_proto::Cmd::SwitchTranslator(Box::new(cfg)));
    }
    mark_settings_dirty(session);
}

// ── UI ──

/// 应用厂商预设（D-85/J 填充语义）：
/// - **覆盖**：`api_base` / `model` / 关闭姿态（`thinking_index` + `disable_thinking`）/ 币种
/// - **仅当显示名为空**才填显示名（用户改过名就保留）
/// - **绝不触碰**：`api_key` / 代理 / 温度 / 覆写 / 额外参数 / 价格 / 上下文数 / 传输开关
/// - `custom` 项：一个字段都不填
fn apply_preset(ed: &mut ModelEditState, p: &lt_proto::ProviderPreset) {
    ed.preset_key = Some(p.key);
    if p.is_custom() {
        return;
    }
    ed.api_base = p.api_base.to_string();
    ed.model = p.model.to_string();
    ed.disable_thinking = p.disable_thinking;
    // 关闭姿态：**对话框可选值域**（`thinking_methods()`，5 项）里取下标——
    // 2026-09-11 评审修复：旧实现按完整 `THINKING_STYLES`（6 项）取下标，
    // `"off"` 得到 5，保存时被钳到 4（= "openai"，会发 `reasoning_effort:"none"`）。
    // "off" 的语义是"不发送"（= 总开关 false，见 presets.rs 注），不在可选值域内
    // 时回落 auto=0；对 OpenAI 端点 auto 路由同样是"不发"。
    ed.thinking_index = crate::state::thinking_methods()
        .iter()
        .position(|s| *s == p.thinking_style)
        .unwrap_or(0);
    ed.currency = p.currency.map(str::to_string);
    if let Some(name) = p.name_for(&ed.name) {
        ed.name = name.to_string();
    }
}

/// 「厂商预设」下拉的显示标签（D-85 评审修复：custom 项与样式页 `preset_custom`
/// 撞键——同名键在 yaml 里重复定义、后写覆盖，样式页标签会被污染成"（不填充）"；
/// 此处改用专属键 `preset_provider_custom`）
fn preset_label(p: &lt_proto::ProviderPreset) -> String {
    if p.is_custom() {
        lt_i18n::t("preset_provider_custom")
    } else {
        lt_i18n::t(&format!("preset_{}", p.key))
    }
}

/// 连接测试结果行（D-85 四态：成功/失败/已中断/未定论）。
/// 颜色沿用页面既有的 ok/err/weak/warn，不新增主题色。
fn probe_result_line(ui: &mut egui::Ui, res: &crate::state::ProbeResult, pal: &Palette) {
    use lt_proto::ProbeOutcome as O;
    ui.add_space(2.0);
    match &res.outcome {
        O::Ok => {
            let mut line = format!("\u{2713} {} · {} ms", lt_i18n::t("probe_ok"), res.ms);
            if let Some(note) = &res.step_note {
                line.push_str(" · ");
                line.push_str(note);
            }
            if let Some(pv) = &res.preview {
                line.push_str(" · ");
                line.push_str(&lt_i18n::t("probe_preview").replace("{text}", pv));
            }
            ui.label(RichText::new(line).size(11.0).color(pal.ok));
        }
        O::Failed { kind, detail } => {
            ui.label(
                RichText::new(format!(
                    "\u{2716} {} · {} ms · {}",
                    lt_i18n::t("probe_failed"),
                    res.ms,
                    lt_i18n::t(kind.i18n_key())
                ))
                .size(11.0)
                .color(pal.err),
            )
            .on_hover_text(detail);
            ui.label(RichText::new(detail).size(10.5).color(pal.weak));
        }
        O::Cancelled => {
            ui.label(
                RichText::new(format!(
                    "\u{23F1} {} · {} ms",
                    lt_i18n::t("probe_cancelled"),
                    res.ms
                ))
                .size(11.0)
                .color(pal.weak),
            );
        }
        O::Inconclusive { attempted } => {
            ui.label(
                RichText::new(format!(
                    "? {} · {} ms · {}",
                    lt_i18n::t("probe_inconclusive"),
                    res.ms,
                    lt_i18n::t("probe_attempted").replace("{n}", &attempted.to_string())
                ))
                .size(11.0)
                .color(pal.warn),
            );
            ui.label(
                RichText::new(lt_i18n::t("probe_inconclusive_hint"))
                    .size(10.5)
                    .color(pal.weak),
            );
        }
    }
}

/// 翻译页 UI 总入口
pub fn page(
    ui: &mut Ui,
    panel: &mut PanelUi,
    session: &mut SessionView,
    settings: &mut Settings,
    modal: &mut ModalUi,
    pal: &Palette,
) {
    // N3/N4：翻译页偏离默认提示 + 恢复本页（恢复会清掉 API 配置 → 确认框）
    let diffs = crate::panel_diff::diff_paths(settings);
    let page_diffs: Vec<&str> = diffs
        .iter()
        .filter(|p| {
            ["models", "active_model", "system_prompt", "timeout"]
                .iter()
                .any(|pre| p.as_str() == *pre || p.as_str().starts_with(pre))
        })
        .map(|p: &String| p.as_str())
        .collect();
    if !page_diffs.is_empty() {
        super::reset_toolbar(ui, pal, page_diffs.len(), &page_diffs.join("、"), |_| {
            // D-33/H-5：确认改 egui 模态（原位 rfd 同步框阻塞事件循环线程）
            modal.request_confirm(
                crate::state::ConfirmKind::ResetTranslation,
                false,
                session
                    .visible
                    .get(&crate::state::WinId::Panel)
                    .copied()
                    .unwrap_or(true),
                lt_i18n::t("reset_confirm_title"),
                lt_i18n::t("reset_confirm_translation"),
            );
        });
    }

    // ── 模型配置（原版 models_group）──
    group_card(ui, pal, &lt_i18n::t("group_model_configs"), |ui| {
        // 翻译装置不可用（配置无效）：状态行红字 + 指引（P0-2 修复——
        // 不再是静默关闭整条翻译）
        if let Some(reason) = &panel.translator_error {
            ui.label(
                RichText::new(format!(
                    "{} {reason}",
                    lt_i18n::t("translator_error_banner")
                ))
                .size(11.0)
                .color(pal.err),
            );
            ui.add_space(2.0);
        }
        // D-85/F2：只读「当前使用」状态行 + 使用说明（设置页不提供切换控件——
        // 切换入口唯一在悬浮窗「模型」下拉，见 docs §4.8）
        {
            let idx = settings
                .active_model
                .min(settings.models.len().saturating_sub(1));
            let name = settings
                .models
                .get(idx)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| "?".into());
            let mut line = lt_i18n::t("status_active_model").replace("{name}", &name);
            if let Some((note_name, at)) = &panel.state.active_model_note {
                // 2 秒内且名字一致才显示"已切换生效"；过期即清（下一次重绘时）
                if note_name == &name && at.elapsed().as_secs() < 2 {
                    line.push_str(" · ");
                    line.push_str(&lt_i18n::t("status_switched"));
                } else if at.elapsed().as_secs() >= 2 {
                    panel.state.active_model_note = None;
                }
            }
            ui.label(RichText::new(line).size(12.0).color(pal.text));
            hint_line(ui, pal, &lt_i18n::t("models_group_hint"));
            ui.add_space(2.0);
        }

        let active = settings.active_model;
        let count = settings.models.len();
        let mut select: Option<usize> = None;
        let mut edit_row: Option<usize> = None;
        // D-85：本页每行右侧的「测试 / 中断」按钮
        let mut probe_click: Option<usize> = None;
        let mut cancel_click = false;
        let now = Instant::now();
        // 在途目标行是否仍可寻址（行号 + 配置身份双校验命中过至少一行）
        let mut my_row_rendered = false;
        for i in 0..count {
            let text = model_row_text(i, active, &settings.models[i]);
            let mut rich = RichText::new(&text).monospace().size(12.0);
            if i == active {
                rich = rich.strong();
            }
            let this_key = crate::state::cfg_key(&settings.models[i]);
            // 归属判据 = 行号 + 配置身份双校验（删行会让行号漂移）
            let is_my_row = panel
                .probe
                .running
                .as_ref()
                .is_some_and(|r| r.row == i && r.cfg_key == this_key);
            ui.horizontal(|ui| {
                let probe_w = 72.0;
                let spacing = ui.spacing().item_spacing.x;
                let row_w = (ui.available_width() - probe_w - spacing).max(120.0);
                let resp = ui
                    .push_id(i, |ui| {
                        ui.add(
                            egui::Button::selectable(panel.state.model_selected == Some(i), rich)
                                .corner_radius(4.0)
                                .min_size(egui::vec2(row_w, 0.0)),
                        )
                    })
                    .inner;
                if resp.clicked() {
                    select = Some(i);
                }
                // 双击行 = 直接进入编辑（原版 itemDoubleClicked →
                // _on_model_double_clicked 打开对话框）。2026-09-11 评审修复：
                // 旧实现只记 `edit_row`（"武装"底部编辑按钮），双击本身不开
                // 对话框——与注释/原版语义不符
                if resp.double_clicked() {
                    edit_row = Some(i);
                    if i < settings.models.len() {
                        let cfg = settings.models[i].clone();
                        panel.state.model_editor = Some(ModelEditState::new_edit(i, &cfg));
                    }
                }
                if is_my_row {
                    my_row_rendered = true;
                    if ui
                        .add(
                            egui::Button::new(RichText::new(lt_i18n::t("probe_cancel")).size(12.0))
                                .corner_radius(6.0),
                        )
                        .clicked()
                    {
                        cancel_click = true;
                    }
                } else if ui
                    .add_enabled(
                        panel.probe.running.is_none(),
                        egui::Button::new(RichText::new(lt_i18n::t("probe_btn")).size(12.0))
                            .corner_radius(6.0),
                    )
                    .clicked()
                {
                    probe_click = Some(i);
                }
            });
            // 行下附加行：在途走秒 / 结果（D-85 四态）
            if is_my_row {
                if let Some(run) = &panel.probe.running {
                    let secs = now.saturating_duration_since(run.started).as_secs_f32();
                    ui.label(
                        RichText::new(format!("{} {secs:.1}s", lt_i18n::t("probe_running")))
                            .size(11.0)
                            .color(pal.weak),
                    );
                }
            } else if let Some(res) = &panel.probe.result {
                if res.row == i && res.cfg_key == this_key {
                    probe_result_line(ui, res, pal);
                }
            }
        }

        // 在途目标行被删除/编辑（双校验全行失配）时，「中断」入口不能消失——
        // 给一个与行解耦的兜底按钮；否则所有行按钮因"有在途"禁用、又无中断可点，
        // 用户只能干等到回执（2026-09-11 评审修复）
        if panel.probe.running.is_some() && !my_row_rendered {
            if let Some(run) = &panel.probe.running {
                let secs = now.saturating_duration_since(run.started).as_secs_f32();
                ui.label(
                    RichText::new(format!("{} {secs:.1}s", lt_i18n::t("probe_running")))
                        .size(11.0)
                        .color(pal.weak),
                );
            }
            if ui
                .add(
                    egui::Button::new(RichText::new(lt_i18n::t("probe_cancel")).size(12.0))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                cancel_click = true;
            }
        }

        // 行选中只改"选中"（D-85 裁决 E：设置页只管配置，不切换运行中的模型）
        if let Some(i) = select {
            panel.state.model_selected = Some(i);
        }

        // 「测试」：发号 + 置在途 + 排节拍 + 发命令（目标就是这一行，无静默回落）
        if let Some(i) = probe_click {
            if let Some(cfg) = settings.models.get(i).cloned() {
                let id = panel.probe.begin(i, crate::state::cfg_key(&cfg), now);
                // D-85 评审修复：走秒与看门狗都挂在 `TickKind::ProbeTick` 上，
                // 而该节拍**只有自续拍**——首次排班必须由这里发出，否则
                // on_probe_tick 永不被调用（走秒冻结、看门狗永不触发）
                session.schedule_tick(
                    WinId::Panel,
                    TickKind::ProbeTick,
                    now + Duration::from_millis(PROBE_TICK_MS),
                );
                session.send_cmd(lt_proto::Cmd::TestTranslator {
                    config: Box::new(cfg),
                    probe_id: id,
                });
            }
        }
        // 「中断」：本地立即落"已中断"（不等后台回执），并请求后台收手
        if cancel_click {
            if let Some(run) = panel.probe.running.take() {
                session.send_cmd(lt_proto::Cmd::CancelTranslatorTest { probe_id: run.id });
                let ms = now.saturating_duration_since(run.started).as_millis() as u64;
                panel.probe.result = Some(crate::state::ProbeResult {
                    id: run.id,
                    row: run.row,
                    cfg_key: run.cfg_key,
                    name: String::new(),
                    outcome: lt_proto::ProbeOutcome::Cancelled,
                    ms,
                    step_note: None,
                    preview: None,
                });
            }
        }

        ui.add_space(4.0);
        // 四按钮行（原版 btn_row：添加/编辑/复制/删除）
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new(lt_i18n::t("btn_add")).size(12.5))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                panel.state.model_editor = Some(ModelEditState::new_add());
            }
            let edit_target = edit_row.or(panel.state.model_selected);
            if ui
                .add_enabled(
                    edit_target.is_some(),
                    egui::Button::new(RichText::new(lt_i18n::t("btn_edit")).size(12.5))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                if let Some(i) = edit_target {
                    if i < settings.models.len() {
                        let cfg = settings.models[i].clone();
                        panel.state.model_editor = Some(ModelEditState::new_edit(i, &cfg));
                    }
                }
            }
            if ui
                .add_enabled(
                    panel.state.model_selected.is_some(),
                    egui::Button::new(RichText::new(lt_i18n::t("btn_duplicate")).size(12.5))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                if let Some(i) = panel.state.model_selected {
                    if duplicate_model(&mut settings.models, i).is_some() {
                        mark_settings_dirty(session);
                    }
                }
            }
            let can_remove = settings.models.len() > 1 && panel.state.model_selected.is_some();
            if ui
                .add_enabled(
                    can_remove,
                    egui::Button::new(RichText::new(lt_i18n::t("btn_remove")).size(12.5))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                if let Some(i) = panel.state.model_selected {
                    let mut active = settings.active_model;
                    if remove_model(&mut settings.models, &mut active, i) {
                        settings.active_model = active;
                        panel.state.model_selected = None;
                        // active 可能变化 → 重建翻译器（原版 _emit_models_list_changed 面）
                        if let Some(cfg) = super::active_model_config(settings) {
                            session.send_cmd(lt_proto::Cmd::SwitchTranslator(Box::new(cfg)));
                        }
                        mark_settings_dirty(session);
                    }
                }
            }
        });

        // ── 当前模型「上下文数」（面板直达）──
        // 此前该设置只在模型编辑对话框的高级区里，配套说明却错挂在「网络配置」
        // 组下（本页看得见说明、找不到控件）。此项改的是**当前活跃模型**的
        // context_turns：600ms 防抖重建翻译器 + 300ms 落盘，与编辑对话框确定
        // 同路（原版 main.py:493 取 model_config.context_turns 后
        // set_context_turns 的语义；面板行是 Rust 版直达入口）
        if !settings.models.is_empty() {
            ui.add_space(6.0);
            let idx = settings.active_model.min(settings.models.len() - 1);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{} ", lt_i18n::t("label_context_turns")))
                        .color(pal.text),
                );
                let mut turns = settings.models[idx].context_turns as i32;
                let resp = ui
                    .add(egui::DragValue::new(&mut turns).range(0..=20).speed(1.0))
                    .on_hover_text(lt_i18n::t("context_turns_hint"));
                if resp.changed() {
                    settings.models[idx].context_turns = turns.clamp(0, 20) as u32;
                    schedule_prompt_apply(panel, session, std::time::Instant::now());
                    mark_settings_dirty(session);
                }
            });
            hint_line(ui, pal, &lt_i18n::t("context_turns_hint"));
        }
    });

    // ── 系统提示词（原版 prompt_group）──
    group_card(ui, pal, &lt_i18n::t("group_system_prompt"), |ui| {
        // 预设下拉（原版 _prompt_preset：4 预设精确匹配；DEFAULT_PROMPT → daily）
        let cur = prompt_preset_index(&settings.system_prompt);
        let mut next = cur;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} ", lt_i18n::t("label_prompt_preset"))).color(pal.text),
            );
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
            settings.system_prompt = lt_proto::PROMPT_PRESETS[next].1.to_string();
            schedule_prompt_apply(panel, session, std::time::Instant::now());
            mark_settings_dirty(session);
        }
        ui.add_space(2.0);
        // 多行编辑（Consolas 等宽；变更 → 600ms 防抖 SwitchTranslator + 300ms 落盘）
        let resp = ui.add(
            egui::TextEdit::multiline(&mut settings.system_prompt)
                .hint_text(lt_proto::DEFAULT_PROMPT)
                .desired_width(f32::INFINITY)
                .min_size(egui::vec2(0.0, 88.0))
                .font(egui::TextStyle::Monospace),
        );
        if resp.changed() {
            schedule_prompt_apply(panel, session, std::time::Instant::now());
            mark_settings_dirty(session);
        }
    });

    // ── 网络配置（原版 net_group：超时 1-60s；ApplySettings 热应用）──
    group_card(ui, pal, &lt_i18n::t("group_network"), |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ", lt_i18n::t("label_timeout"))).color(pal.text));
            let mut v = settings.timeout.clamp(1, 60) as f32;
            let resp = ui.add(
                egui::DragValue::new(&mut v)
                    .range(1.0..=60.0)
                    .speed(1.0)
                    .suffix(" s"),
            );
            if resp.changed() {
                settings.timeout = v.round() as u32;
                mark_settings_dirty(session);
            }
        });
    });

    ui.add_space(8.0);

    // ── ModelEditDialog（egui::Window 居中模态区，对照 dialogs.py）──
    render_model_editor(ui, panel, session, settings, pal);
}

/// ModelEditDialog 模态区（打开中每帧渲染；确定/取消/关闭由 outcome 收敛）
fn render_model_editor(
    ui: &mut Ui,
    panel: &mut PanelUi,
    session: &mut SessionView,
    settings: &mut Settings,
    pal: &Palette,
) {
    if panel.state.model_editor.is_none() {
        return;
    }
    let mut open = true;
    let mut cancel = false;
    // Some(cfg) = 确定；None + 窗关闭/取消 = 放弃编辑
    let mut accepted: Option<ModelConfig> = None;
    {
        let Some(ed) = panel.state.model_editor.as_mut() else {
            return;
        };
        let title = if ed.is_new {
            lt_i18n::t("dialog_add_model")
        } else {
            lt_i18n::t("dialog_edit_model")
        };
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
        let ed = panel.state.model_editor.take().expect("编辑器打开中");
        apply_editor_result(panel, session, settings, cfg, ed.index, ed.is_new);
    } else if !open || cancel {
        panel.state.model_editor = None;
    }
}

/// 对话框确定后的写回（原版 _add_model/_edit_model：name+model 非空才收；
/// 编辑活动模型 → 即时 SwitchTranslator；统一防抖落盘）
fn apply_editor_result(
    panel: &mut PanelUi,
    session: &mut SessionView,
    settings: &mut Settings,
    cfg: ModelConfig,
    index: usize,
    is_new: bool,
) {
    if cfg.name.is_empty() || cfg.model.is_empty() {
        return; // 原版 get_data 后的 if data["name"] and data["model"] 守卫
    }
    if is_new {
        settings.models.push(cfg);
        panel.state.model_selected = Some(settings.models.len() - 1);
    } else {
        let idx = index.min(settings.models.len() - 1);
        let was_active = idx == settings.active_model;
        settings.models[idx] = cfg.clone();
        panel.state.model_selected = Some(idx);
        if was_active {
            session.send_cmd(lt_proto::Cmd::SwitchTranslator(Box::new(cfg)));
        }
    }
    mark_settings_dirty(session);
}

/// 对话框字段全集（Basic + Advanced，原版 QFormLayout 逐行）
fn editor_fields(ui: &mut Ui, ed: &mut ModelEditState, pal: &Palette) {
    // ── 厂商预设（D-85/J）：选中即填 5 个字段；不覆盖用户已填/已选的内容 ──
    ui.horizontal(|ui| {
        ui.label(RichText::new(lt_i18n::t("preset_label")).color(pal.text));
        let mut picked: Option<&'static lt_proto::ProviderPreset> = None;
        // 选中态反映"最近一次选择/反查命中"的预设（未命中 → 自定义项）
        let selected = ed
            .preset_key
            .and_then(lt_proto::preset_by_key)
            .map(preset_label)
            .unwrap_or_else(|| lt_i18n::t("preset_provider_custom"));
        egui::ComboBox::from_id_salt("model_edit_preset")
            .selected_text(selected)
            .width(220.0)
            .show_ui(ui, |ui| {
                for p in lt_proto::PROVIDER_PRESETS.iter() {
                    if ui
                        .selectable_label(ed.preset_key == Some(p.key), preset_label(p))
                        .clicked()
                    {
                        picked = Some(p);
                    }
                }
            });
        if let Some(p) = picked {
            apply_preset(ed, p);
        }
        hint_line(ui, pal, &lt_i18n::t("preset_hint"));
    });
    ui.add_space(4.0);

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
            let modes = [
                lt_i18n::t("proxy_none"),
                lt_i18n::t("proxy_system"),
                lt_i18n::t("proxy_custom"),
            ];
            egui::ComboBox::from_id_salt("model_edit_proxy_mode")
                .selected_text(modes[ed.proxy_index].clone())
                .width(240.0)
                .show_ui(ui, |ui| {
                    for (i, label) in modes.iter().enumerate() {
                        if ui
                            .selectable_label(ed.proxy_index == i, label.clone())
                            .clicked()
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

            // W1（方案 §2.3）：关闭模型思考总开关——默认勾选，勾选即发送关闭字段。
            // item 5（2026-09-10）：已确认"关不掉"的模型显示为**未勾选** + 提示；
            // 用户手动重新勾选 = 明确要求再试一次 → 清除持久化标记（正常走后端阶梯）
            ui.label(lt_i18n::t("label_thinking_switch"));
            ui.horizontal(|ui| {
                let mut checked = ed.disable_thinking && !ed.thinking_unavailable;
                let resp = ui
                    .checkbox(&mut checked, lt_i18n::t("thinking_switch_on"))
                    .on_hover_text(lt_i18n::t("thinking_switch_hint"));
                if resp.changed() {
                    ed.disable_thinking = checked;
                    if checked {
                        ed.thinking_unavailable = false;
                    }
                }
                if ed.thinking_unavailable {
                    ui.label(
                        RichText::new(lt_i18n::t("thinking_unavailable_hint"))
                            .size(11.0)
                            .color(pal.warn),
                    )
                    .on_hover_text(lt_i18n::t("thinking_unavailable_hint_tip"));
                }
            });
            ui.end_row();

            // W1（方案 §2.1）：温度——勾选才发送（勾选前连数值控件都不显示）；
            // 不勾选 = 请求中不出现该参数（2026-09-10 附裁决：默认不勾选）
            ui.label(lt_i18n::t("label_temperature"));
            ui.horizontal(|ui| {
                ui.checkbox(&mut ed.temperature_enabled, lt_i18n::t("send_param"))
                    .on_hover_text(lt_i18n::t("temperature_hint"));
                if ed.temperature_enabled {
                    ui.add(
                        egui::DragValue::new(&mut ed.temperature_value)
                            .range(0.0..=2.0)
                            .speed(0.05),
                    );
                }
            });
            ui.end_row();

            // 价格（原版 price_row：输入/输出 $/1M，0 显示 "—"）
            ui.label(lt_i18n::t("label_pricing"));
            ui.horizontal(|ui| {
                // D-85：币种下拉（跟随界面语言 / 人民币 / 美元）——价格单位与
                // 费用显示符号都取它；旧实现没有单位说明、符号还按界面语言切
                ui.label(lt_i18n::t("currency_label"));
                let cur =
                    lt_proto::effective_currency(ed.currency.as_deref(), &lt_i18n::get_lang());
                let selected = match ed.currency.as_deref() {
                    None => lt_i18n::t("currency_follow_lang"),
                    Some(_) => lt_i18n::t(match cur {
                        lt_proto::Currency::Cny => "currency_cny",
                        lt_proto::Currency::Usd => "currency_usd",
                    }),
                };
                egui::ComboBox::from_id_salt("model_edit_currency")
                    .selected_text(selected)
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(
                                ed.currency.is_none(),
                                lt_i18n::t("currency_follow_lang"),
                            )
                            .clicked()
                        {
                            ed.currency = None;
                        }
                        if ui
                            .selectable_label(
                                ed.currency.as_deref() == Some("cny"),
                                lt_i18n::t("currency_cny"),
                            )
                            .clicked()
                        {
                            ed.currency = Some("cny".into());
                        }
                        if ui
                            .selectable_label(
                                ed.currency.as_deref() == Some("usd"),
                                lt_i18n::t("currency_usd"),
                            )
                            .clicked()
                        {
                            ed.currency = Some("usd".into());
                        }
                    });
                ui.label(lt_i18n::t("label_input_price"));
                price_drag(ui, "model_edit_ip", &mut ed.input_price);
                ui.label(lt_i18n::t("label_output_price"));
                price_drag(ui, "model_edit_op", &mut ed.output_price);
                // 单位随生效币种（不再无单位）
                ui.label(
                    RichText::new(lt_i18n::t(cur.unit_key()))
                        .size(11.0)
                        .color(pal.weak),
                );
            });
            ui.end_row();
        });

    // 方案 §4.7 就地校验（软提示：**不阻断保存**、**不擅自改用户填的地址**——
    // 判断只 trim + 去尾斜杠；文案只提示，保存仍按用户原文）
    for w in config_warnings(ed) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("\u{26A0}").size(11.0).color(pal.warn));
            ui.label(RichText::new(w).size(11.0).color(pal.warn));
        });
    }

    // 行为复选（W1/方案 §2.4：json_response 移出界面——翻译不需要 JSON 模式且
    // 兼容面窄；no_system_role 移入高级区；上下文数移入高级区）
    ui.add_space(2.0);
    checkbox_row(
        ui,
        "model_edit_streaming",
        &mut ed.streaming,
        "streaming",
        "streaming_hint",
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
            // W1/方案 §2.3：关闭方式（总开关未勾选时置灰；已选值仍保留，
            // 重新勾选即沿用）
            ui.label(lt_i18n::t("label_thinking_style"));
            ui.add_enabled_ui(ed.disable_thinking, |ui| {
                let styles = crate::state::thinking_methods()
                    .iter()
                    .map(|v| lt_i18n::t(&format!("thinking_style_{v}")))
                    .collect::<Vec<_>>();
                let idx = ed.thinking_index.min(styles.len() - 1);
                egui::ComboBox::from_id_salt("model_edit_thinking")
                    .selected_text(styles[idx].clone())
                    .width(240.0)
                    .show_ui(ui, |ui| {
                        for (i, label) in styles.iter().enumerate() {
                            if ui.selectable_label(idx == i, label.clone()).clicked()
                                && ed.thinking_index != i
                            {
                                ed.thinking_index = i;
                            }
                        }
                    });
            });
            ui.end_row();

            // W1：高级区保留的主流采样项（其余三项不再渲染但值保留）
            for &i in ADV_OVERRIDE_ROWS.iter() {
                ui.label(lt_i18n::t(OVERRIDE_LABEL_KEYS[i]));
                let row = &mut ed.overrides[i];
                ui.horizontal(|ui| {
                    let mut current = *row;
                    if ui
                        .checkbox(&mut current.enabled, lt_i18n::t("override_enable"))
                        .changed()
                    {
                        *row = current;
                    }
                    ui.add_enabled(current.enabled, override_drag(i, &mut row.value));
                });
                ui.end_row();
            }

            // 上下文数（W1：自基本区移入高级区；原版 QSpinBox 0-20 + setToolTip
            // (context_turns_hint)——tooltip 此前漏移植，此处补回）
            ui.label(lt_i18n::t("label_context_turns"));
            let mut turns = ed.context_turns;
            let resp = ui
                .add(egui::DragValue::new(&mut turns).range(0..=20).speed(1.0))
                .on_hover_text(lt_i18n::t("context_turns_hint"));
            if resp.changed() {
                ed.context_turns = turns;
            }
            ui.end_row();

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
    // W1：少数端点兼容开关（自基本区移入高级区）
    checkbox_row(
        ui,
        "model_edit_nsr",
        &mut ed.no_system_role,
        "no_system_role",
        "no_system_role_hint",
    );
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
                    .custom_formatter(|v, _| {
                        if v <= 0.0 {
                            "—".into()
                        } else {
                            format!("{v:.2}")
                        }
                    }),
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
        ed.overrides[0] = OverrideRow {
            enabled: true,
            value: 0.7,
        }; // temperature
        ed.overrides[2] = OverrideRow {
            enabled: true,
            value: 512.0,
        }; // max_tokens
        ed.overrides[1] = OverrideRow {
            enabled: false,
            value: 0.9,
        }; // 未勾选不写
        ed.extra_body_text = r#"{"thinking": {"type": "disabled"}}"#.into();
        let cfg = ed.build().expect("extra_body 合法");

        let obj = serde_json::to_value(&cfg)
            .unwrap()
            .as_object()
            .unwrap()
            .clone();
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
        assert!(
            obj.get("json_schema_mode").is_none(),
            "json_schema_mode 不在契约内"
        );
        assert!(
            obj.get("no_think").is_none(),
            "no_think 已迁移为 thinking_style"
        );
        // overrides 只含勾选行（BTreeMap 键序）
        assert_eq!(
            obj["overrides"],
            serde_json::json!({"max_tokens": 512, "temperature": 0.7})
        );
        // 反序列化往返（形状即契约）
        let round: ModelConfig =
            serde_json::from_value(serde_json::to_value(&cfg).unwrap()).unwrap();
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
        let auto = ModelConfig {
            thinking_style: Some("auto".into()),
            ..Default::default()
        };
        assert!(!serde_json::to_value(auto)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("thinking_style"));
    }

    /// 模型列表摘要行（refresh_model_list 形状：前缀/proxy 标签/第二行）
    #[test]
    fn model_row_text_formats_like_original() {
        let plain = ModelConfig {
            name: "A".into(),
            api_base: "http://b".into(),
            model: "m1".into(),
            ..Default::default()
        };
        assert_eq!(model_row_text(0, 0, &plain), ">>> A\n     http://b  |  m1");
        assert_eq!(model_row_text(1, 0, &plain), "    A\n     http://b  |  m1");
        let proxied = ModelConfig {
            proxy: "http://p:7890".into(),
            name: "B".into(),
            api_base: "u".into(),
            model: "m2".into(),
            ..Default::default()
        };
        let text = model_row_text(0, 0, &proxied);
        assert!(text.contains(">>> B  [proxy: http://p:7890]"), "{text}");
        assert!(text.contains("\n     u  |  m2"));
        // system 代理同样带标签；none 不带
        assert!(model_row_text(
            0,
            0,
            &ModelConfig {
                proxy: "system".into(),
                ..plain.clone()
            }
        )
        .contains("[proxy: system]"));
        assert!(!model_row_text(0, 0, &plain).contains("[proxy"));
    }

    /// prompt 预设索引映射（空/DEFAULT → daily；精确匹配；其余 custom）
    #[test]
    fn prompt_preset_index_mapping() {
        assert_eq!(prompt_preset_index(""), 0, "空 = DEFAULT_PROMPT → daily");
        assert_eq!(prompt_preset_index(lt_proto::DEFAULT_PROMPT), 0);
        for (i, (_, body)) in lt_proto::PROMPT_PRESETS.iter().enumerate() {
            assert_eq!(prompt_preset_index(body), i, "预设 {i} 应精确匹配");
            // 原版按 trim 比较
            assert_eq!(prompt_preset_index(&format!("  {body}\n")), i);
        }
        assert_eq!(prompt_preset_index("You are a pirate."), 4, "其余 → custom");
        assert_eq!(PROMPT_PRESET_KEYS.len(), 5);
        assert_eq!(lt_proto::PROMPT_PRESETS.len(), 4);
    }

    /// 删除模型：单行守卫 + active 钳制 + 行前移修正
    #[test]
    fn remove_model_guards_and_active_adjustment() {
        let mk = |n: &str| ModelConfig {
            name: n.into(),
            ..Default::default()
        };
        let mut models = vec![mk("a"), mk("b"), mk("c")];
        let mut active = 2;
        assert!(remove_model(&mut models, &mut active, 0));
        assert_eq!(
            (models.len(), active),
            (2, 1),
            "删行在 active 前 → active 前移"
        );
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
        let mk = |n: &str| ModelConfig {
            name: n.into(),
            model: "m".into(),
            ..Default::default()
        };
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
            overrides: Some(BTreeMap::from([(
                "top_p".to_string(),
                serde_json::json!(0.9),
            )])),
            ..Default::default()
        };
        let ed = ModelEditState::new_edit(0, &cfg);
        assert_eq!(ed.build().unwrap(), cfg);
    }

    /// 模态区无头渲染冒烟：ModelEditDialog 打开态整页面跑两帧不 panic
    #[test]
    fn model_editor_modal_smoke_renders_headless() {
        let ctx = egui::Context::default();
        let mut st = crate::state::AppUi::new(lt_proto::Settings::default());
        st.panel.state.page = crate::state::PanelPage::Translation;
        let mut ed = ModelEditState::new_edit(0, &st.settings.models[0]);
        ed.extra_body_text = "{\"a\": 1}".into();
        st.panel.state.model_editor = Some(ed);
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                crate::windows::dispatch(crate::state::WinId::Panel, ui, &mut st)
            });
            assert!(!out.shapes.is_empty(), "编辑器打开态应产出图元");
            out.textures_delta.clear();
        }
        // 确定按钮不在帧内点击；编辑器保持打开（状态未被意外消费）
        assert!(st.panel.state.model_editor.is_some());
    }

    /// item 5 + §4.7 的新 UI 分支无头渲染冒烟：关不掉提示行与就地校验提示
    /// 同帧共存时不 panic（两处都是纯展示，不阻断保存）
    #[test]
    fn model_editor_smoke_with_unavailable_and_warnings() {
        let ctx = egui::Context::default();
        let mut st = crate::state::AppUi::new(lt_proto::Settings::default());
        st.panel.state.page = crate::state::PanelPage::Translation;
        let mut ed = ModelEditState::new_edit(0, &st.settings.models[0]);
        ed.apply_thinking_unavailable(); // 未勾选 + 提示行
        ed.api_base = String::new(); // 空地址告警
        ed.model = String::new(); // 空模型名告警
        ed.extra_body_text = "[1,2]".into(); // 非 object 告警
        st.panel.state.model_editor = Some(ed);
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                crate::windows::dispatch(crate::state::WinId::Panel, ui, &mut st)
            });
            assert!(!out.shapes.is_empty());
            out.textures_delta.clear();
        }
        assert!(st.panel.state.model_editor.is_some());
    }

    /// §4.7 就地校验：空/无 scheme/缺版本段/尾斜杠/非 object 各自成提示；
    /// 合规配置零告警（不阻断保存、不改写用户输入——本函数只读）
    #[test]
    fn config_warnings_cover_shape_and_url_issues() {
        // 合规样例：/v1 + 无尾斜杠 → 零告警
        let mut ed = ModelEditState::new_add();
        ed.api_base = "http://127.0.0.1:1234/v1".into();
        ed.model = "qwen2.5".into();
        assert!(
            config_warnings(&ed).is_empty(),
            "{:?}",
            config_warnings(&ed)
        );
        // 厂商自有版本段（GLM /api/paas/v4）不报"缺 /v1"
        ed.api_base = "https://open.bigmodel.cn/api/paas/v4".into();
        assert!(config_warnings(&ed).is_empty());
        // 空地址 + 空模型名
        ed.api_base = "   ".into();
        ed.model = "".into();
        let w = config_warnings(&ed);
        assert!(w.contains(&lt_i18n::t("cfg_warn_api_base_empty")));
        assert!(w.contains(&lt_i18n::t("cfg_warn_model_empty")));
        assert!(
            !w.contains(&lt_i18n::t("cfg_warn_api_base_v1")),
            "空地址不叠加"
        );
        // 无 scheme
        ed.api_base = "127.0.0.1:1234/v1".into();
        assert!(config_warnings(&ed).contains(&lt_i18n::t("cfg_warn_api_base_scheme")));
        // 缺版本段 + 尾斜杠（两者可叠加；尾斜杠由原文判断）。
        // 用非预设主机：厂商官方地址现已豁免该提示（D-85/J），
        // 豁免行为另有 preset_api_base_skips_v1_warning 覆盖
        ed.api_base = "https://api.example.com/".into();
        let w = config_warnings(&ed);
        assert!(w.contains(&lt_i18n::t("cfg_warn_api_base_v1")));
        assert!(w.contains(&lt_i18n::t("cfg_warn_api_base_slash")));
        // 尾斜杠不误报"缺版本段"（判断副本去尾斜杠）
        ed.api_base = "http://127.0.0.1:1234/v1/".into();
        let w = config_warnings(&ed);
        assert!(!w.contains(&lt_i18n::t("cfg_warn_api_base_v1")));
        assert!(w.contains(&lt_i18n::t("cfg_warn_api_base_slash")));
        // extra_body 形态（类型化判定）
        ed.api_base = "http://127.0.0.1:1234/v1".into();
        ed.model = "qwen2.5".into();
        ed.extra_body_text = "[1,2]".into();
        assert!(config_warnings(&ed).contains(&lt_i18n::t("cfg_warn_extra_body_nonobject")));
        ed.extra_body_text = "{oops".into();
        assert!(config_warnings(&ed).contains(&lt_i18n::t("cfg_warn_extra_body_invalid")));
        ed.extra_body_text = "{\"a\": 1}".into();
        assert!(config_warnings(&ed).is_empty());
        // 提示函数只读——不改写用户填的地址（trim 只用于本地判断）
        ed.api_base = "  http://127.0.0.1:1234/v1/  ".into();
        let _ = config_warnings(&ed);
        assert_eq!(ed.api_base, "  http://127.0.0.1:1234/v1/  ", "不得改写输入");
        assert_eq!(ed.extra_body_shape(), crate::state::ExtraBodyShape::Object);
    }

    /// 面板窗口实际逻辑尺寸（app.rs：535×781，visual-parity 对齐原版实机）——
    /// 无头探针按真实视口渲染，顺带钉住"新控件在首屏可见"
    const PANEL_VIEWPORT: egui::Vec2 = egui::vec2(535.0, 781.0);

    /// 无头渲染翻译页（逐帧送事件），返回末帧文本图元 (rect, 文本)。
    /// rect 取 galley 尺寸 + `Shape::Text::pos`（egui 的 galley.rect 相对 pos）。
    fn render_translation_page(
        st: &mut crate::state::AppUi,
        ctx: &egui::Context,
        frames: Vec<Vec<egui::Event>>,
    ) -> Vec<(egui::Rect, String)> {
        let mut last = Vec::new();
        for events in frames {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, PANEL_VIEWPORT)),
                    ..Default::default()
                },
                |ui| crate::windows::dispatch(crate::state::WinId::Panel, ui, st),
            );
            out.textures_delta.clear();
            last = out
                .shapes
                .iter()
                .filter_map(|cl| match &cl.shape {
                    egui::Shape::Text(t) => Some((
                        egui::Rect::from_min_size(t.pos, t.galley.size()),
                        t.galley.text().to_string(),
                    )),
                    _ => None,
                })
                .collect();
        }
        last
    }

    /// i18n 取值候选（zh/en 两份内嵌表）：并行测试都可能改全局语言，
    /// 断言一律按"任一表命中"匹配，避免被 `set_lang` 竞争打飞（t_for_lang
    /// 不触碰全局状态）。`suffix` 用于控件标签这类渲染时拼接了空格的文本。
    fn text_variants(key: &str, suffix: &str) -> Vec<String> {
        ["zh", "en"]
            .iter()
            .map(|l| format!("{}{suffix}", lt_i18n::t_for_lang(l, key)))
            .collect()
    }

    /// 渲染文本里找 key 命中的图元（zh/en 任一）
    fn find_by_key<'a>(
        texts: &'a [(egui::Rect, String)],
        key: &str,
        suffix: &str,
    ) -> Option<&'a (egui::Rect, String)> {
        let variants = text_variants(key, suffix);
        texts.iter().find(|(_, t)| variants.iter().any(|v| v == t))
    }

    /// 「上下文数」说明必须落在**模型配置组内**（控件同组），不得再像修复前那样
    /// 错挂在「网络配置」组下（本页看得见说明、找不到控件）。
    #[test]
    fn context_hint_sits_in_model_group_not_network_group() {
        let _ = lt_i18n::set_lang("zh");
        let ctx = egui::Context::default();
        let mut st = crate::state::AppUi::new(Settings::default());
        st.panel.state.page = crate::state::PanelPage::Translation;
        // 两帧：egui 即时模式布局第二帧才收敛
        let texts = render_translation_page(&mut st, &ctx, vec![vec![], vec![]]);
        let top_of = |key: &str, suffix: &str| {
            find_by_key(&texts, key, suffix)
                .map(|(r, _)| r.top())
                .unwrap_or_else(|| panic!("{key} 应出现在翻译页"))
        };
        let model_y = top_of("group_model_configs", "");
        let net_y = top_of("group_network", "");
        let label_y = top_of("label_context_turns", " ");
        let hint_y = top_of("context_turns_hint", "");
        let hint_count = texts
            .iter()
            .filter(|(_, t)| text_variants("context_turns_hint", "").contains(t))
            .count();
        assert_eq!(
            hint_count, 1,
            "说明只应出现一次（修复前它挂在网络配置组下）"
        );
        assert!(
            model_y < label_y && label_y < hint_y && hint_y < net_y,
            "标签与说明都必须在模型配置组内：模型组 {model_y} < 标签 {label_y} < 说明 {hint_y} < 网络组 {net_y}"
        );
        assert!(
            hint_y < PANEL_VIEWPORT.y,
            "上下文数行必须落在面板首屏内（y={hint_y}，视口高 {}）",
            PANEL_VIEWPORT.y
        );
        // 值就在标签同一行右侧（控件可达）
        let label = find_by_key(&texts, "label_context_turns", " ")
            .expect("标签 rect")
            .0;
        assert!(
            texts.iter().any(|(r, t)| {
                !t.is_empty()
                    && t.chars().all(|c| c.is_ascii_digit())
                    && (r.top() - label.top()).abs() < 6.0
                    && r.left() > label.left()
            }),
            "上下文数标签右侧应有可拖拽的数值控件"
        );
    }

    /// 拖拽面板行 → 写回**当前活跃模型**的 context_turns，并登记 600ms 翻译器
    /// 重建与 300ms 落盘（原版 main.py:493 取 model_config.context_turns 后
    /// set_context_turns 的语义；面板行是 Rust 版直达入口）。
    /// 渲染按真实面板视口（535×781）——控件若被挤出可视区，egui 交互判定
    /// （clip rect 命中）会失败、拖拽无反应，本用例同时钉住"入口真实可用"。
    #[test]
    fn panel_context_drag_writes_active_model_and_schedules_rebuild() {
        let _ = lt_i18n::set_lang("zh");
        let ctx = egui::Context::default();
        let mut st = crate::state::AppUi::new(Settings::default());
        st.panel.state.page = crate::state::PanelPage::Translation;
        st.settings.active_model = 0;
        assert_eq!(st.settings.models[0].context_turns, 0, "默认不携带上下文");
        let texts = render_translation_page(&mut st, &ctx, vec![vec![]]);
        // 控件中心 = 标签同一行右侧的数字图元（DragValue 本体即该文本）
        let label = find_by_key(&texts, "label_context_turns", " ")
            .map(|(r, _)| *r)
            .expect("上下文数标签");
        let value = texts
            .iter()
            .filter(|(r, t)| {
                !t.is_empty()
                    && t.chars().all(|c| c.is_ascii_digit())
                    && (r.top() - label.top()).abs() < 6.0
                    && r.left() > label.left()
            })
            .min_by(|a, b| a.0.left().total_cmp(&b.0.left()))
            .map(|(r, _)| *r)
            .expect("面板行数值控件");
        let p = value.center();
        let down = egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        };
        let up = egui::Event::PointerButton {
            pos: p + egui::vec2(15.0, 0.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        };
        // 悬停 → 按下 → 单帧横拖（越过 egui 拖动判定阈值；speed=1.0 → +15）→ 抬起
        let _ = render_translation_page(
            &mut st,
            &ctx,
            vec![
                vec![egui::Event::PointerMoved(p)],
                vec![down],
                vec![egui::Event::PointerMoved(p + egui::vec2(15.0, 0.0))],
                vec![up],
            ],
        );
        assert_eq!(
            st.settings.models[0].context_turns, 15,
            "拖拽 15px（speed=1.0）应写入活跃模型 context_turns=15（实际 {}）",
            st.settings.models[0].context_turns
        );
        assert!(
            st.panel.state.prompt_apply_due.is_some(),
            "应登记 600ms 防抖翻译器重建"
        );
        assert!(
            st.session
                .ticks
                .iter()
                .any(|t| t.kind == crate::state::TickKind::PromptApply),
            "应排入 PromptApply 节拍"
        );
        assert!(st.session.settings_apply_pending, "应登记设置落盘防抖");
    }

    /// 版本段判定（避免对厂商自有路径误报）
    #[test]
    fn version_segment_detection_is_conservative() {
        assert!(has_version_segment("http://x/v1"));
        assert!(has_version_segment("https://x/api/paas/v4"));
        assert!(has_version_segment("http://x/v1beta"));
        assert!(!has_version_segment("https://api.deepseek.com"));
        assert!(!has_version_segment("http://x/compatible-mode"));
        assert!(!has_version_segment(""));
    }

    // ── D-85：每行「测试 / 中断」+ 行点击不再切换（裁决 E） ──

    /// 面板视口内按文本找控件的中心点（"控件的可交互性"由 egui 的 clip 判定，
    /// 用真实视口渲染即等于验证入口可达）
    fn click_at(texts: &[(egui::Rect, String)], key: &str, suffix: &str) -> egui::Pos2 {
        find_by_key(texts, key, suffix)
            .map(|(r, _)| r.center())
            .unwrap_or_else(|| panic!("{key} 应出现在翻译页"))
    }

    fn click_events(pos: egui::Pos2) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ]
    }

    /// 裁决 E 回归：点行**只选中**——不写 active_model、不发 SwitchTranslator
    #[test]
    fn row_click_only_selects_no_switch_cmd() {
        let ctx = egui::Context::default();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut settings = Settings::default();
        // 造两行，确保"点第二行"在旧实现里会触发切换
        let mut second = settings.models[0].clone();
        second.name = "second".into();
        second.model = "m2".into();
        settings.models.push(second);
        settings.active_model = 0;
        let mut st = crate::state::AppUi::new(settings);
        st.session.cmd_tx = Some(tx);
        st.panel.state.page = crate::state::PanelPage::Translation;
        // 第一帧：布局收敛并拿到行控件位置
        let texts = render_translation_page(&mut st, &ctx, vec![vec![], vec![]]);
        let row = texts
            .iter()
            .find(|(_, t)| t.contains("second"))
            .map(|(r, _)| r.center())
            .expect("第二行应可见");
        // 第二帧：点它
        render_translation_page(&mut st, &ctx, vec![click_events(row), vec![]]);

        assert_eq!(
            st.panel.state.model_selected,
            Some(1),
            "点行应把该行设为选中（编辑/复制/删除的目标）"
        );
        assert_eq!(st.settings.active_model, 0, "点行不得改变当前使用的模型");
        let mut cmds = Vec::new();
        while let Ok(c) = rx.try_recv() {
            cmds.push(c);
        }
        assert!(
            !cmds
                .iter()
                .any(|c| matches!(c, lt_proto::Cmd::SwitchTranslator(_))),
            "点行不得发出切换命令，实际: {cmds:?}"
        );
    }

    /// 每行「测试」按钮：发号 + 置在途 + 携带该行配置（点哪测哪）
    #[test]
    fn probe_row_button_sends_cmd_with_id_and_config() {
        let ctx = egui::Context::default();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut st = crate::state::AppUi::new(Settings::default());
        st.session.cmd_tx = Some(tx);
        st.panel.state.page = crate::state::PanelPage::Translation;
        let texts = render_translation_page(&mut st, &ctx, vec![vec![], vec![]]);
        let btn = click_at(&texts, "probe_btn", "");
        render_translation_page(&mut st, &ctx, vec![click_events(btn), vec![]]);

        let run = st.panel.probe.running.as_ref().expect("应进入在途态");
        assert_eq!(run.row, 0, "默认单行配置 → 第 0 行");
        assert_eq!(run.id, 1, "首个探测号应为 1（0 是 shell 侧无在途哨兵）");
        match rx.try_recv() {
            Ok(lt_proto::Cmd::TestTranslator { config, probe_id }) => {
                assert_eq!(probe_id, 1);
                assert_eq!(config.model, st.settings.models[0].model);
            }
            other => panic!("期望 TestTranslator，实际 {other:?}"),
        }
        // D-85 评审修复：点击必须**首次排班**走秒/看门狗节拍——ProbeTick 只有
        // 自续拍，缺首排则 on_probe_tick 永不被调用（走秒冻结、看门狗死）
        assert!(
            st.session
                .ticks
                .iter()
                .any(|t| t.kind == TickKind::ProbeTick),
            "点击「测试」应排入 ProbeTick 节拍"
        );
    }

    /// 号源回归：前两个探测号是 1、2（永不发 0），旧结果 `id` 随之
    #[test]
    fn probe_ids_start_at_one() {
        let mut st = crate::state::ProbeUiState::default();
        let t0 = Instant::now();
        let key = (
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        );
        let first = st.begin(0, key.clone(), t0);
        st.running = None;
        let second = st.begin(0, key, t0);
        assert_eq!((first, second), (1, 2), "号源从 1 起且单调递增");
    }

    /// 在途时该行按钮变「中断」、其他行按钮禁用；点中断 → 发取消命令 + 本地立即落"已中断"
    #[test]
    fn probe_cancel_settles_locally_and_other_rows_disabled() {
        let ctx = egui::Context::default();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut settings = Settings::default();
        let mut second = settings.models[0].clone();
        second.name = "second".into();
        settings.models.push(second);
        let mut st = crate::state::AppUi::new(settings);
        st.session.cmd_tx = Some(tx);
        st.panel.state.page = crate::state::PanelPage::Translation;
        // 直接置在途（等价于点过测试）
        let running_id = st.panel.probe.begin(
            0,
            crate::state::cfg_key(&st.settings.models[0]),
            Instant::now(),
        );

        let texts = render_translation_page(&mut st, &ctx, vec![vec![], vec![]]);
        // 在途行走秒提示 + 该行按钮是「中断」
        assert!(
            find_by_key(&texts, "probe_running", "").is_none(),
            "走秒文本带秒数后缀，find_by_key 精确匹配不该命中"
        );
        assert!(
            texts
                .iter()
                .any(|(_, t)| t.starts_with(&lt_i18n::t_for_lang("zh", "probe_running"))),
            "在途行下应有「测试中… N.Ns」"
        );
        // 其他行的「测试」按钮虽渲染但在**行为上禁用**：点它不得发出新探测
        // （2026-09-11 评审补齐 §6.1 other_rows_disabled_while_running——
        // 像素断言不可行，改以"点击无效"的行为断言）
        let other_btn = click_at(&texts, "probe_btn", "");
        render_translation_page(&mut st, &ctx, vec![click_events(other_btn), vec![]]);
        assert_eq!(
            st.panel.probe.running.as_ref().map(|r| r.id),
            Some(running_id),
            "在途期间点其他行的「测试」不得启动新探测"
        );
        // 该行按钮此时变「中断」——点中断：发取消命令 + 本地立即落"已中断"
        let texts = render_translation_page(&mut st, &ctx, vec![vec![], vec![]]);
        let cancel = click_at(&texts, "probe_cancel", "");
        render_translation_page(&mut st, &ctx, vec![click_events(cancel), vec![]]);

        assert!(st.panel.probe.running.is_none(), "中断后应退出在途态");
        match st.panel.probe.result.as_ref().map(|r| &r.outcome) {
            Some(lt_proto::ProbeOutcome::Cancelled) => {}
            other => panic!("本地应立即落「已中断」，实际 {other:?}"),
        }
        let mut cmds = Vec::new();
        while let Ok(c) = rx.try_recv() {
            cmds.push(c);
        }
        assert!(
            cmds.iter()
                .any(|c| matches!(c, lt_proto::Cmd::CancelTranslatorTest { probe_id } if *probe_id == running_id)),
            "应携带在途号发出取消命令（在途 #{running_id}），实际: {cmds:?}"
        );
        assert!(
            !cmds
                .iter()
                .any(|c| matches!(c, lt_proto::Cmd::TestTranslator { .. })),
            "被禁用的「测试」不得发出 TestTranslator，实际: {cmds:?}"
        );
    }

    /// 2026-09-11 评审补齐 §6.1：双击行仍进编辑、四按钮目标 = 选中行（回归）。
    /// 双击 = 相邻两帧各一次点击（egui 无显式时钟时按 ~1/60s 递推，
    /// 必落在双击间隔内）。
    #[test]
    fn row_double_click_opens_editor_and_buttons_target_selection() {
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        let mut second = settings.models[0].clone();
        second.name = "second".into();
        settings.models.push(second);
        let mut st = crate::state::AppUi::new(settings);
        st.panel.state.page = crate::state::PanelPage::Translation;
        let texts = render_translation_page(&mut st, &ctx, vec![vec![], vec![]]);
        let row = texts
            .iter()
            .find(|(_, t)| t.contains("second"))
            .map(|(r, _)| r.center())
            .expect("第二行应可见");

        // 双击 → 编辑该行（is_new=false，index=1）
        render_translation_page(&mut st, &ctx, vec![click_events(row), click_events(row)]);
        let ed = st
            .panel
            .state
            .model_editor
            .as_ref()
            .expect("双击行应打开编辑对话框");
        assert!(!ed.is_new, "双击既有行应进编辑而非新建");
        assert_eq!(ed.index, 1, "应编辑被双击的那一行");

        // 关掉对话框，选中第 1 行后点「复制」→ 复制的是选中行
        st.panel.state.model_editor = None;
        st.panel.state.model_selected = Some(1);
        let texts = render_translation_page(&mut st, &ctx, vec![vec![], vec![]]);
        let dup = click_at(&texts, "btn_duplicate", "");
        render_translation_page(&mut st, &ctx, vec![click_events(dup)]);
        assert_eq!(st.settings.models.len(), 3, "复制应作用于选中行");
        assert_eq!(
            st.settings.models[2].name, "second (copy)",
            "复制行 = 选中行 + \" (copy)\" 后缀（原版 _dup_model）"
        );
    }

    /// 2026-09-11 评审补齐 §6.1：删除**正在使用**的行仍须内部切换翻译器
    /// （与裁决 E 不冲突：正确性保留，见方案 §4.8 表格）
    #[test]
    fn delete_active_row_still_switches_internally() {
        let ctx = egui::Context::default();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut settings = Settings::default();
        let mut second = settings.models[0].clone();
        second.name = "second".into();
        settings.models.push(second);
        settings.active_model = 0;
        let mut st = crate::state::AppUi::new(settings);
        st.session.cmd_tx = Some(tx);
        st.panel.state.page = crate::state::PanelPage::Translation;
        st.panel.state.model_selected = Some(0);

        let texts = render_translation_page(&mut st, &ctx, vec![vec![], vec![]]);
        let del = click_at(&texts, "btn_remove", "");
        render_translation_page(&mut st, &ctx, vec![click_events(del), vec![]]);

        assert_eq!(st.settings.models.len(), 1, "删除后应只剩一行");
        assert_eq!(st.settings.models[0].name, "second");
        assert_eq!(st.settings.active_model, 0, "活动模型钳制到存活行");
        let mut cmds = Vec::new();
        while let Ok(c) = rx.try_recv() {
            cmds.push(c);
        }
        assert!(
            cmds.iter()
                .any(|c| matches!(c, lt_proto::Cmd::SwitchTranslator(_))),
            "删掉活动行后必须按钳制结果重建翻译器，实际: {cmds:?}"
        );
    }

    /// 2026-09-11 评审补齐 §6.1：价格行单位随**生效币种**（D-85/I），
    /// 指定币种即按指定显示（不随界面语言）
    #[test]
    fn price_unit_label_follows_currency() {
        for (cur, key) in [
            (Some("cny"), "price_unit_cny"),
            (Some("usd"), "price_unit_usd"),
        ] {
            let ctx = egui::Context::default();
            let mut st = crate::state::AppUi::new(Settings::default());
            st.panel.state.page = crate::state::PanelPage::Translation;
            let mut cfg = st.settings.models[0].clone();
            cfg.currency = cur.map(str::to_string);
            let mut ed = ModelEditState::new_edit(0, &cfg);
            ed.currency = cur.map(str::to_string);
            st.panel.state.model_editor = Some(ed);
            let texts = render_translation_page(&mut st, &ctx, vec![vec![], vec![]]);
            let variants = text_variants(key, "");
            assert!(
                texts.iter().any(|(_, t)| variants.iter().any(|v| t == v)),
                "币种 {cur:?} 的价格单位应为 {key}，实际 {texts:?}"
            );
        }
    }

    /// 目标行被删/改后双校验全行失配：仍须有可点的「中断」（兜底入口不与行绑定）
    #[test]
    fn probe_target_row_lost_keeps_cancel_affordance() {
        let ctx = egui::Context::default();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut st = crate::state::AppUi::new(Settings::default());
        st.session.cmd_tx = Some(tx);
        st.panel.state.page = crate::state::PanelPage::Translation;
        // 在途目标行号越界（模拟：探测期间目标行被删除）
        let id = st.panel.probe.begin(
            9,
            ("x".into(), "y".into(), "z".into(), "w".into()),
            Instant::now(),
        );
        let texts = render_translation_page(&mut st, &ctx, vec![vec![], vec![]]);
        let cancel = click_at(&texts, "probe_cancel", "");
        render_translation_page(&mut st, &ctx, vec![click_events(cancel), vec![]]);

        assert!(st.panel.probe.running.is_none(), "兜底中断后应退出在途态");
        let mut cmds = Vec::new();
        while let Ok(c) = rx.try_recv() {
            cmds.push(c);
        }
        assert!(
            cmds.iter().any(
                |c| matches!(c, lt_proto::Cmd::CancelTranslatorTest { probe_id } if *probe_id == id)
            ),
            "兜底中断应发出携带在途号的取消命令，实际: {cmds:?}"
        );
    }

    /// 结果行按 id/配置身份失效：改行配置后旧结果不再渲染
    #[test]
    fn probe_result_invalidated_on_config_change() {
        let ctx = egui::Context::default();
        let mut st = crate::state::AppUi::new(Settings::default());
        st.panel.state.page = crate::state::PanelPage::Translation;
        st.panel.probe.result = Some(crate::state::ProbeResult {
            id: 0,
            row: 0,
            cfg_key: crate::state::cfg_key(&st.settings.models[0]),
            name: "x".into(),
            outcome: lt_proto::ProbeOutcome::Ok,
            ms: 12,
            step_note: None,
            preview: Some("你好".into()),
        });
        let texts = render_translation_page(&mut st, &ctx, vec![vec![], vec![]]);
        assert!(
            texts.iter().any(|(_, t)| t.contains("你好")),
            "配置未变时应渲染结果行"
        );
        // 改配置 → 结果失效
        st.settings.models[0].api_key = "changed".into();
        let texts = render_translation_page(&mut st, &ctx, vec![vec![], vec![]]);
        assert!(
            !texts.iter().any(|(_, t)| t.contains("你好")),
            "配置变更后旧结果必须失效"
        );
    }

    /// D-85/F2：面板「当前使用：X」只读展示；收到 TranslatorSwitched 后
    /// 短暂追加「已切换生效」；**区域内不含任何切换控件**（裁决 E）
    #[test]
    fn active_model_status_line_renders_name_and_has_no_control() {
        let ctx = egui::Context::default();
        let mut st = crate::state::AppUi::new(Settings::default());
        st.panel.state.page = crate::state::PanelPage::Translation;
        let texts = render_translation_page(&mut st, &ctx, vec![vec![], vec![]]);
        let want = lt_i18n::t_for_lang("zh", "status_active_model")
            .replace("{name}", &st.settings.models[0].name);
        let row = texts
            .iter()
            .find(|(_, t)| t == &want)
            .map(|(r, _)| *r)
            .expect("状态行应显示「当前使用：<名字>」");
        // 状态行右侧（同一行）不得有任何可点控件：用"文字图元"近似断言——
        // 该行只有这一条文本，且其后无按钮文字（切换控件是下拉/按钮，必有文字）
        let same_row: Vec<&str> = texts
            .iter()
            .filter(|(r, _)| (r.top() - row.top()).abs() < 10.0)
            .map(|(_, t)| t.as_str())
            .collect();
        assert_eq!(
            same_row.len(),
            1,
            "状态行应独占一行（只读），实际: {same_row:?}"
        );
        // 切换生效提示
        st.panel.state.active_model_note =
            Some((st.settings.models[0].name.clone(), Instant::now()));
        let texts = render_translation_page(&mut st, &ctx, vec![vec![], vec![]]);
        let want_switched = format!("{want} · {}", lt_i18n::t_for_lang("zh", "status_switched"));
        assert!(
            texts.iter().any(|(_, t)| t == &want_switched),
            "新鲜的回执应追加「已切换生效」"
        );
        // 使用说明在场（裁决 E 的现场指引）
        assert!(
            find_by_key(&texts, "models_group_hint", "").is_some(),
            "模型组内应有使用说明行"
        );
    }

    // ── D-85/J：预设填充语义 ──

    /// 选预设 → 覆盖地址/模型/关闭姿态/币种；显示名仅在为空时填。
    /// **断言走 `build()`（真实保存路径）**——2026-09-11 评审修复：旧测试直接
    /// `THINKING_STYLES[索引]` 读回，恰好绕开"对话框值域 5 项"的钳制行为
    #[test]
    fn preset_fills_all_fields() {
        let mut ed = ModelEditState::new_add();
        assert!(ed.name.is_empty(), "新增时显示名为空");
        apply_preset(&mut ed, lt_proto::preset_by_key("deepseek").unwrap());
        let p = lt_proto::preset_by_key("deepseek").unwrap();
        let saved = ed.build().expect("默认草稿应可保存");
        assert_eq!(saved.api_base, p.api_base);
        assert_eq!(saved.model, p.model);
        assert_eq!(saved.currency.as_deref(), p.currency);
        assert_eq!(saved.thinking_style.as_deref(), Some(p.thinking_style));
        assert_eq!(saved.disable_thinking, p.disable_thinking);
        assert_eq!(ed.name, "DeepSeek", "空名应被填上");
        assert_eq!(ed.preset_key, Some("deepseek"), "下拉选中态应跟随");
    }

    /// 每个预设经保存路径落出的关闭姿态 = 预设意图（2026-09-11 评审修复：
    /// OpenAI 的 `"off"` 曾被值域钳成 `"openai"` → 保存后实际发送
    /// `reasoning_effort:"none"`，正是方案 §十 明令避免的形态）
    #[test]
    fn preset_saved_config_matches_intended_posture() {
        for p in lt_proto::PROVIDER_PRESETS.iter().filter(|p| !p.is_custom()) {
            let mut ed = ModelEditState::new_add();
            apply_preset(&mut ed, p);
            let saved = ed.build().expect("预设草稿应可保存");
            if p.thinking_style == "off" {
                // "off" 的规范编码：总开关 false（= 不发送任何关闭参数，见 presets.rs 注）
                assert!(
                    !saved.disable_thinking,
                    "{} 预设不得尝试发送关闭参数",
                    p.key
                );
                assert_eq!(
                    saved.thinking_style, None,
                    "{}：\"off\" 不落盘（sanitize 归一语义）",
                    p.key
                );
            } else {
                assert_eq!(
                    saved.thinking_style.as_deref(),
                    Some(p.thinking_style),
                    "{} 的关闭姿态应原样落盘",
                    p.key
                );
                assert!(saved.disable_thinking, "{} 应勾选关闭思考", p.key);
            }
            assert_eq!(
                ed.preset_key,
                Some(p.key),
                "{} 下拉选中态应跟随最近选择",
                p.key
            );
        }
    }

    /// 关闭方式下拉索引永远落在对话框值域内（5 项）——越界会被 `build()` 钳到
    /// 邻居项（旧 bug 的机制）；此处对全部预设逐一钉住
    #[test]
    fn preset_thinking_index_stays_within_dialog_domain() {
        let n = crate::state::thinking_methods().len();
        for p in lt_proto::PROVIDER_PRESETS.iter().filter(|p| !p.is_custom()) {
            let mut ed = ModelEditState::new_add();
            apply_preset(&mut ed, p);
            assert!(
                ed.thinking_index < n,
                "{} 的关闭方式索引 {} 越出对话框值域 {}",
                p.key,
                ed.thinking_index,
                n
            );
        }
    }

    /// 用户已填的内容一律不覆盖（密钥/代理/温度/覆写/价格/上下文数），
    /// 显示名非空也不覆盖
    #[test]
    fn preset_never_overwrites_user_data() {
        let mut ed = ModelEditState::new_edit(
            0,
            &lt_proto::ModelConfig {
                name: "我的自建".into(),
                api_key: "sk-user".into(),
                proxy: "http://127.0.0.1:7890".into(),
                temperature: Some(0.7),
                input_price: 3.0,
                output_price: 6.0,
                context_turns: 5,
                ..lt_proto::ModelConfig::default()
            },
        );
        ed.temperature_enabled = true;
        ed.temperature_value = 0.7;
        ed.proxy_index = 2;
        ed.proxy_url = "http://127.0.0.1:7890".into();
        let before = (
            ed.api_key.clone(),
            ed.proxy_index,
            ed.proxy_url.clone(),
            ed.temperature_value,
            ed.input_price,
            ed.output_price,
            ed.context_turns,
        );
        apply_preset(&mut ed, lt_proto::preset_by_key("openai").unwrap());
        assert_eq!(ed.api_key, before.0, "密钥绝不被覆盖");
        assert_eq!(ed.proxy_index, before.1, "代理模式绝不被覆盖");
        assert_eq!(ed.proxy_url, before.2, "代理地址绝不被覆盖");
        assert_eq!(ed.temperature_value, before.3, "温度绝不被覆盖");
        assert_eq!(ed.input_price, before.4, "价格绝不被覆盖");
        assert_eq!(ed.output_price, before.5, "价格绝不被覆盖");
        assert_eq!(ed.context_turns, before.6, "上下文数绝不被覆盖");
        assert_eq!(ed.name, "我的自建", "非空显示名绝不被覆盖");
        // 但目标字段确实换了
        assert_eq!(ed.api_base, "https://api.openai.com/v1");
    }

    /// `custom` 项一个字段都不填
    #[test]
    fn preset_custom_fills_nothing() {
        let mut ed = ModelEditState::new_add();
        ed.api_base = "http://my.gateway/v1".into();
        ed.model = "m".into();
        apply_preset(&mut ed, lt_proto::preset_by_key("custom").unwrap());
        assert_eq!(ed.api_base, "http://my.gateway/v1");
        assert_eq!(ed.model, "m");
        assert!(ed.name.is_empty());
        assert_eq!(ed.currency, None);
        assert_eq!(ed.preset_key, Some("custom"), "选中态应落在 custom 项");
        // 专属键不得与样式页 `preset_custom` 撞名（yaml 重复键曾静默覆盖）
        assert_ne!(
            lt_i18n::t_for_lang("zh", "preset_provider_custom"),
            "preset_provider_custom",
            "厂商预设 custom 标签键缺失"
        );
        assert_eq!(
            lt_i18n::t_for_lang("zh", "preset_custom"),
            "自定义",
            "样式页既有标签不得被厂商预设键污染"
        );
    }

    /// 官方形态地址豁免"缺 /v1"软提示；自建地址仍提示（回归）
    #[test]
    fn preset_api_base_skips_v1_warning() {
        let mut ed = ModelEditState::new_add();
        apply_preset(&mut ed, lt_proto::preset_by_key("deepseek").unwrap());
        let warns = config_warnings(&ed);
        assert!(
            !warns
                .iter()
                .any(|w| w.contains(&lt_i18n::t_for_lang("zh", "cfg_warn_api_base_v1"))),
            "预设的官方地址不该弹缺 /v1 提示：{warns:?}"
        );
        // 自建无版本段地址 → 仍提示
        ed.api_base = "https://my.gateway.example".into();
        let warns = config_warnings(&ed);
        assert!(
            warns
                .iter()
                .any(|w| w.contains(&lt_i18n::t_for_lang("zh", "cfg_warn_api_base_v1"))),
            "自建地址应保留提示：{warns:?}"
        );
    }
}
