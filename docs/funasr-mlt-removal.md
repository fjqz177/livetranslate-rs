# Fun-ASR-MLT-Nano 全量移除施工方案

| | |
|---|---|
| 状态 | **待用户批准，未施工**（批准后本文件先独立 `docs(funasr-mlt-removal)` 提交，再按清单施工） |
| 日期 | 2026-09-12 |
| 偏差编号 | **D-86**（取代 D-14 为 mlt 安排的"灰显幽灵值"占位机制；D-14 决策史本身不改写） |
| 契约影响 | `FUNASR_MODELS` 值域成员删除 = 删除既有契约项 → 按冻结规则须评审留痕，**PROTO_VERSION 6 → 7**（D-81 删变体递增版本的同一先例） |
| 风险评级 | **低**——mlt 在现网行为 = 界面灰显不可选、无引擎、无下载源，正常用户零感知；唯一行为面是旧 settings.json 收敛，由 sanitize 兜底臂天然覆盖 |
| 预期测试变化 | 基线 603+9 → **604+9**（净 +1：新增旧档收敛锁；其余为改写/注释更新） |

## 0. 一句话结论

把 `funasr-mlt-nano-2512` 从设置合法值域、注册表幽灵值机制、UI 灰显项、引擎别名表、i18n 提示键中全部删掉；旧配置文件里残留的 mlt 值统一收敛为 `funasr + sensevoice-small`。

## 1. 背景与动机

- mlt（Fun-ASR-MLT-Nano-2512，31 语种版）自 D-14 起**从未可用**：上游 sherpa-onnx 无官方 ONNX 转换包，注册表无条目，UI 灰显占位，运行时回退 sensevoice-small。
- 2026-09-12 ASR 模型调研（docs/asr-model-survey-2026.md §3.5，未跟踪稿）实证：官方导出仍缺，仅社区个人导出（不采信，D-14 先例）。**用户裁决：与其永等上游，不如整体清除死代码**。
- 现状代价：每个经手 funasr 值域的人都要理解"幽灵值"概念（`funasr_key_is_ghost`、R25 防线测试的幽灵例外、UI 的 enabled 门控分支、三处 fallback 独立表达）——全部是为一个永不落地的模型维持的机制税。

## 2. 旧档兼容性保证（唯一的行为面）

删除后，任何旧 `settings.json` 里的 mlt 残留按下列路径收敛，**无数据丢失、无启动失败**（sanitize 兜底臂既有行为，本方案不改 sanitize 结构）：

| 旧档内容 | 收敛路径 | 终态 |
|---|---|---|
| `funasr_model: "funasr-mlt-nano-2512"` | 不在 `FUNASR_MODELS` → `normalize_funasr_model` 落 `_ =>` 兜底臂 | `sensevoice-small` |
| `asr_engine: "funasr-mlt-nano"`（原版 Python 迁移档） | 别名表行已删 → 不在 `ASR_ENGINES` → 非法引擎回退 | `funasr`（funasr_model 再走上行走 sensevoice-small） |

两路径都会在启动日志留修正行（`fixed` 向量既有机制）。收敛语义由新增测试 `retired_mlt_values_converge` 钉死（见 §5）。

## 3. 施工清单（8 个源文件 + 2 个 i18n 键）

> 行号会漂移，以下均以**锚文本**定位；每块给"现状 → 改为"。`……` 表示上下文省略。

### 3.1 `crates/lt-proto/src/lib.rs`

**① PROTO_VERSION 递增**

```rust
// 现状
pub const PROTO_VERSION: u32 = 6;
// 改为（注释说明 D-86）
pub const PROTO_VERSION: u32 = 7; // D-86：FUNASR_MODELS 删 funasr-mlt-nano-2512（值域成员删除，评审留痕）
```

全仓无测试钉住该值、无握手消费它（已核实：除定义外仅注释引用），递增零行为影响，纯契约记账。

### 3.2 `crates/lt-proto/src/settings.rs`

**① FUNASR_MODELS 收缩 3→2**

```rust
// 现状
/// 合法 funasr 模型（mlt 置灰但仍是合法存量值）
pub const FUNASR_MODELS: [&str; 3] = [
    "sensevoice-small",
    "funasr-nano-2512",
    "funasr-mlt-nano-2512",
];
// 改为
/// 合法 funasr 模型（D-86：mlt 值域成员移除——退役键经 sanitize 回退 sensevoice-small）
pub const FUNASR_MODELS: [&str; 2] = ["sensevoice-small", "funasr-nano-2512"];
```

**② ASR_ENGINE_LEGACY_ALIASES 删 mlt 行（3→2）**

```rust
// 现状
pub const ASR_ENGINE_LEGACY_ALIASES: [(&str, &str); 3] = [
    ("sensevoice", "sensevoice-small"),
    ("funasr-nano", "funasr-nano-2512"),
    ("funasr-mlt-nano", "funasr-mlt-nano-2512"),
];
// 改为（D-86 注记：旧档 funasr-mlt-nano 经"非法引擎→funasr"+"模型归一→sensevoice-small"双重收敛）
pub const ASR_ENGINE_LEGACY_ALIASES: [(&str, &str); 2] = [
    ("sensevoice", "sensevoice-small"),
    ("funasr-nano", "funasr-nano-2512"),
];
```

**③ normalize_funasr_model 删两处 mlt 臂**

```rust
// 现状
fn normalize_funasr_model(v: &str) -> String {
    match v {
        "sensevoice" => "sensevoice-small".into(),
        "funasr-nano" => "funasr-nano-2512".into(),
        "funasr-mlt-nano" => "funasr-mlt-nano-2512".into(),
        "sensevoice-small" | "funasr-nano-2512" | "funasr-mlt-nano-2512" => v.into(),
        _ => "sensevoice-small".into(),
    }
}
// 改为（mlt 两串自然落 _ 兜底臂 → sensevoice-small，即 §2 的收敛路径）
fn normalize_funasr_model(v: &str) -> String {
    match v {
        "sensevoice" => "sensevoice-small".into(),
        "funasr-nano" => "funasr-nano-2512".into(),
        "sensevoice-small" | "funasr-nano-2512" => v.into(),
        _ => "sensevoice-small".into(),
    }
}
```

**④ 测试 `legacy_engine_alias_migration`：mlt 子案例改写为收敛断言**

```rust
// 现状（973-978 附近）
// funasr-mlt-nano → funasr + funasr-mlt-nano-2512
let s = Settings::from_value_compatible(serde_json::json!({
    "asr_engine": "funasr-mlt-nano"
}));
assert_eq!(s.asr_engine, "funasr");
assert_eq!(s.funasr_model, "funasr-mlt-nano-2512");
// 改为
// D-86：funasr-mlt-nano 别名已删——旧档经"非法引擎→funasr"+"模型归一"收敛
let s = Settings::from_value_compatible(serde_json::json!({
    "asr_engine": "funasr-mlt-nano"
}));
assert_eq!(s.asr_engine, "funasr");
assert_eq!(s.funasr_model, "sensevoice-small");
```

**⑤ 测试 `normal_engine_values_untouched`：mlt 子案例换成真正常值**

```rust
// 现状（986-989 附近）
let s = Settings::from_value_compatible(serde_json::json!({
    "asr_engine": "funasr",
    "funasr_model": "funasr-mlt-nano-2512"
}));
assert_eq!(s.asr_engine, "funasr");
assert_eq!(s.funasr_model, "funasr-mlt-nano-2512");
// 改为
let s = Settings::from_value_compatible(serde_json::json!({
    "asr_engine": "funasr",
    "funasr_model": "funasr-nano-2512"
}));
assert_eq!(s.asr_engine, "funasr");
assert_eq!(s.funasr_model, "funasr-nano-2512");
```

**⑥ 新增测试 `retired_mlt_values_converge`（D-86 旧档兼容锁）**

```rust
/// D-86：退役键两条收敛路径的端到端锁（mlt 从值域删除后旧档不炸、不落死值）
#[test]
fn retired_mlt_values_converge() {
    let s = Settings::from_value_compatible(serde_json::json!({
        "asr_engine": "funasr",
        "funasr_model": "funasr-mlt-nano-2512"
    }));
    assert_eq!(s.asr_engine, "funasr");
    assert_eq!(s.funasr_model, "sensevoice-small");
}
```

### 3.3 `crates/lt-models/src/registry.rs`

**① FUNASR_KEYS 收缩 3→2**

```rust
// 现状
pub const FUNASR_KEYS: [&str; 3] = [
    "sensevoice-small",
    "funasr-nano-2512",
    "funasr-mlt-nano-2512",
];
// 改为
pub const FUNASR_KEYS: [&str; 2] = ["sensevoice-small", "funasr-nano-2512"];
```

**② `funasr_entry` 删 mlt 臂**

```rust
// 现状
pub fn funasr_entry(key: &str) -> Option<ModelEntry> {
    match key {
        "sensevoice-small" => Some(SENSEVOICE_SMALL.clone()),
        "funasr-nano-2512" => Some(FUNASR_NANO.clone()),
        // mlt 是独立模型，无上游 ONNX 转换，待上游产出（D-14）；不得用 nano 冒充
        "funasr-mlt-nano-2512" => None,
        _ => None,
    }
}
// 改为（mlt 串与任意非法串同落 _ 臂；文档注释补一句 D-86）
pub fn funasr_entry(key: &str) -> Option<ModelEntry> {
    match key {
        "sensevoice-small" => Some(SENSEVOICE_SMALL.clone()),
        "funasr-nano-2512" => Some(FUNASR_NANO.clone()),
        _ => None, // 非法/退役键（D-86 后含 mlt 旧档残留；sanitize 已收敛，此为防御面）
    }
}
```

**③ `funasr_key_is_ghost` 整函数删除**

唯一在案幽灵（mlt）不复存在 → 该判定恒假 → 函数与其文档注释整体删除。其仅有的两处消费同步消失：`pipeline.rs:2229`（见 3.5③）与下条测试。

**④ 测试 `every_funasr_key_has_entry_or_is_the_documented_ghost` → 更名 `every_funasr_key_has_entry`，删幽灵例外**

```rust
// 改为
/// R25 防线：合法值域 ↔ 注册表一致性。FUNASR_MODELS 每个合法值必有注册表条目
/// （D-86：幽灵值机制随 mlt 移除——要么有表项，要么根本不进值域）。
/// 今后给值域加键而忘登记表（幽灵值复发）在此爆掉；FUNASR_KEYS 镜像与
/// lt_proto::FUNASR_MODELS 漂移同样在此爆掉。
#[test]
fn every_funasr_key_has_entry() {
    for key in lt_proto::settings::FUNASR_MODELS {
        assert!(funasr_entry(key).is_some(), "合法值 '{key}' 缺注册表条目（幽灵值复发）");
    }
    assert_eq!(FUNASR_KEYS, lt_proto::settings::FUNASR_MODELS, "键表镜像漂移");
}
```

（原测试中 `const GHOST`、幽灵例外分支、`funasr_key_is_ghost` 三条断言全删。）

**⑤ 测试 `funasr_entry_keys`：注释改写，断言保留**

```rust
// 现状
// mlt 无上游 ONNX 转换（D-14）→ None，运行时回退 sensevoice-small
assert!(funasr_entry("funasr-mlt-nano-2512").is_none());
// 改为（断言不动——语义变为"退役键与非法键同路"的兼容锁）
// D-86：退役键 mlt 无条目（旧档经 sanitize 收敛，正常流不会到此）
assert!(funasr_entry("funasr-mlt-nano-2512").is_none());
```

### 3.4 `crates/lt-models/src/cache.rs`

**① `missing_models` 注释更新（代码不动）**

```rust
// 现状
// mlt/非法键回退 sensevoice-small（与 pipeline 装载一致，D-14）
// 改为
// 非法/退役键回退 sensevoice-small（与 pipeline 装载一致，D-86）
```

**② 测试 `missing_models_semantics` mlt 子案例注释改写（断言不动）**

```rust
// 现状
// mlt 键回退后同样报 sensevoice-small 缺失（D-14）
// 改为
// D-86：退役键 mlt 与非法键同路——回退后报 sensevoice-small 缺失
```

### 3.5 `crates/lt-orchestrator/src/pipeline.rs`

**① `resolve_funasr_entry` 文档注释更新（函数体不动）**

```rust
// 现状
/// funasr 模型键 → (条目, 是否回退)。
/// mlt 无上游 ONNX 转换、待上游产出（D-14）；mlt/非法键统一回退 sensevoice-small（bool = true）。
// 改为
/// funasr 模型键 → (条目, 是否回退)。
/// 非法/退役键（D-86 后含 mlt 旧档残留）统一回退 sensevoice-small（bool = true）。
```

**② 启动装配处注释更新（约 2219 行）**

```rust
// 现状
// 模型键 → 条目；mlt/非法键回退 sensevoice-small（不阻断 UI，也不得用 nano 冒充）。
// 改为
// 模型键 → 条目；非法/退役键回退 sensevoice-small（不阻断 UI）。
```

**③ fell_back 日志分支删幽灵判定（唯一 `funasr_key_is_ghost` 生产调用点）**

```rust
// 现状
if fell_back {
    let reason = if registry::funasr_key_is_ghost(&settings.funasr_model) {
        "mlt 无上游 ONNX 转换，待上游产出（D-14）"
    } else {
        "非法模型键"
    };
    tracing::warn!(
        "funasr 模型 {:?} 不可用（{reason}），回退 sensevoice-small",
        settings.funasr_model
    );
}
// 改为
if fell_back {
    tracing::warn!(
        "funasr 模型 {:?} 非法/退役（D-86），回退 sensevoice-small",
        settings.funasr_model
    );
}
```

**④ 测试 `funasr_dispatch_nano_vs_sensevoice`：mlt 子案例注释改写（断言不动）**

```rust
// 现状
// mlt 无注册表条目（D-14）→ 回退 sensevoice-small 条目装配
// 改为
// D-86：退役键 mlt 与非法键同路 → 回退 sensevoice-small 条目装配（旧档兼容锁）
```

**⑤ 测试组注释与 `mlt_falls_back_to_sensevoice`：改名留义**

```rust
// 现状
// ── resolve_funasr_entry：mlt/非法键回退（D-14） ──
#[test]
fn mlt_falls_back_to_sensevoice() {
    let (entry, fell_back) = resolve_funasr_entry("funasr-mlt-nano-2512");
    assert!(fell_back);
    // 绝不能静默用 nano 冒充 mlt（D-14）
    assert_eq!(entry, registry::funasr_entry("sensevoice-small").unwrap());
}
// 改为（断言全部不动，仅更名与注释——保留为旧档收敛回归锁）
// ── resolve_funasr_entry：非法/退役键回退（D-86） ──
#[test]
fn retired_mlt_falls_back_to_sensevoice() {
    let (entry, fell_back) = resolve_funasr_entry("funasr-mlt-nano-2512");
    assert!(fell_back);
    // D-86：退役键与 bogus 同路回退，绝不冒充 nano
    assert_eq!(entry, registry::funasr_entry("sensevoice-small").unwrap());
}
```

### 3.6 `crates/lt-ui/src/windows/panel/vad.rs`

**① 文件头 doc 注释：删 D-14 行**

```rust
// 删除这一行
//! - D-14：funasr-mlt-nano-2512 无上游 ONNX 转换 → 模型下拉灰显；
```

**② `FunasrModelItem` 删 `enabled` 字段（该字段唯一语义就是 mlt 灰显）**

```rust
// 现状
/// FunASR 模型下拉项（原版 funasr_model_options()：(键, 显示名)；enabled 为 Rust 版
/// 附加语义——mlt 无上游 ONNX（D-14）灰显，nano 标注实验性）
pub struct FunasrModelItem {
    pub key: &'static str,
    pub display: String,
    pub enabled: bool,
}
// 改为
/// FunASR 模型下拉项（原版 funasr_model_options()：(键, 显示名)；nano 标注实验性。
/// D-86：mlt 灰显项与 enabled 门控随幽灵值机制移除）
pub struct FunasrModelItem {
    pub key: &'static str,
    pub display: String,
}
```

**③ `funasr_model_items` 删 None/幽灵分支**

```rust
// 改为（值域成员必有表项由 every_funasr_key_has_entry 钉死，expect 有防线背书）
pub fn funasr_model_items() -> Vec<FunasrModelItem> {
    let exp = lt_i18n::t("model_experimental");
    lt_proto::settings::FUNASR_MODELS
        .into_iter()
        .map(|key| {
            let entry = lt_models::registry::funasr_entry(key)
                .expect("R25 防线：值域成员必有注册表条目");
            // 实验性标注为 UI 侧语义（能力未定，默认放行但显式提示）
            let display = if key == "funasr-nano-2512" {
                format!("{}{exp}", entry.display)
            } else {
                entry.display.to_string()
            };
            FunasrModelItem { key, display }
        })
        .collect()
}
```

**④ 模型下拉渲染：删灰显分支与 m_idx==2 提示**

```rust
// 现状（ComboBox 内循环体）
for (i, item) in items.iter().enumerate() {
    if !item.enabled {
        // D-14：mlt 灰显（无上游 ONNX）
        ui.add_enabled(
            false,
            egui::Button::selectable(false, item.display.clone()),
        );
    } else if ui
        .selectable_label(m_idx == i, item.display.clone())
        .clicked()
        && m_idx != i
    {
        settings.funasr_model = item.key.to_string();
        send_switch_engine(settings, session);
    }
}
// ……
if m_idx == 2 {
    hint_line(ui, pal, &lt_i18n::t("model_mlt_disabled_hint"));
}
// 改为
for (i, item) in items.iter().enumerate() {
    if ui
        .selectable_label(m_idx == i, item.display.clone())
        .clicked()
        && m_idx != i
    {
        settings.funasr_model = item.key.to_string();
        send_switch_engine(settings, session);
    }
}
//（m_idx == 2 提示块整体删除——下拉已无第三项）
```

**⑤ `CacheStatus::Unavailable` 文档注释改写（变体保留，见 §6）**

```rust
// 现状
/// 无注册表条目（mlt，D-14）
Unavailable,
// 改为
/// 无注册表条目（未知/退役键——防御面：sanitize 后正常不可达）
Unavailable,
```

**⑥ 缓存组 Unavailable 渲染臂换通用提示键**

```rust
// 现状
CacheStatus::Unavailable => {
    hint_line(ui, pal, &lt_i18n::t("model_mlt_disabled_hint"));
}
// 改为
CacheStatus::Unavailable => {
    hint_line(ui, pal, &lt_i18n::t("model_unavailable_hint"));
}
```

**⑦ `engine_status` 注释去 mlt（行为不动）**

```rust
// 现状
/// 可下载；mlt 等无条目模型同样落此文案，与既有 funasr 行为一致）
// 改为
/// 可下载；未知/退役键等无条目情形同样落此文案）
```

**⑧ 测试 `funasr_model_items_order_and_gating` → 更名 `funasr_model_items_order_and_labels`，两值断言**

```rust
// 改为
/// FunASR 模型表：顺序同 FUNASR_MODELS，nano 带实验性标注（D-86：mlt 项已移除）
#[test]
fn funasr_model_items_order_and_labels() {
    let items = funasr_model_items();
    assert_eq!(
        items.iter().map(|m| m.key).collect::<Vec<_>>(),
        ["sensevoice-small", "funasr-nano-2512"]
    );
    assert!(
        items[1].display.contains("experimental") || items[1].display.contains("实验性"),
        "nano 应带实验性标注: {}",
        items[1].display
    );
    // 索引映射：合法值命中、非法值回退 0（sanitize 语义）
    assert_eq!(funasr_index_for("funasr-nano-2512"), 1);
    assert_eq!(funasr_index_for("bogus"), 0);
    assert_eq!(funasr_index_for("funasr-mlt-nano-2512"), 0, "D-86：退役键回退首项");
}
```

**⑨ 测试 `cache_status_probes_models_dir`：mlt 断言保留，注释改写**

```rust
// 现状
// mlt 无条目 → Unavailable（D-14）
assert_eq!(
    model_cache_status(&dir, "funasr", "funasr-mlt-nano-2512", ""),
    CacheStatus::Unavailable
);
// 改为（断言不动——退役键仍走 funasr_entry None 防御臂，锁住该分支不回归）
// D-86：退役键无条目 → Unavailable（防御分支）
assert_eq!(
    model_cache_status(&dir, "funasr", "funasr-mlt-nano-2512", ""),
    CacheStatus::Unavailable
);
```

### 3.7 `crates/lt-ui/src/windows/panel/data.rs`

**① 注释去 mlt（代码不动——`FUNASR_KEYS` 迭代随收缩自然变 2 项）**

```rust
// 现状
// funasr 系（注册表键序；mlt 无上游条目自然跳过）
// 改为
// funasr 系（注册表键序）
```

### 3.8 `crates/lt-ui/src/windows/panel/mod.rs`

**① 冒烟测试注释改写（测试体不动——仍设 `funasr_model = "funasr-mlt-nano-2512"` 强制走 Unavailable 防御分支，该分支删 mlt 后依然可达，测试继续有效）**

```rust
// 现状
// VAD/ASR 页强制走设备枚举 + 缓存探测双分支（mlt 保存值 → Unavailable 提示）
// 改为
// VAD/ASR 页强制走设备枚举 + 缓存探测双分支（退役键 → Unavailable 防御分支）
```

### 3.9 `assets/i18n/zh.yaml` + `assets/i18n/en.yaml`（同步修改，键集断言要求两份对齐）

**① 键更名 + 通用化措辞**（原键名带 mlt，且原措辞"上游尚未发布 ONNX 转换"是 mlt 专属语境；Unavailable 分支删 mlt 后语义泛化为未知模型防御面）

```yaml
# zh.yaml 现状（489 行附近）
model_mlt_disabled_hint: "上游尚未发布该模型的 ONNX 转换，暂不可选。"
# zh.yaml 改为
model_unavailable_hint: "未知模型，无法探测缓存状态。"

# en.yaml 现状（485 行附近）
model_mlt_disabled_hint: "Upstream ONNX export for this model is not available yet — currently unselectable."
# en.yaml 改为
model_unavailable_hint: "Unknown model — cache status unavailable."
```

## 4. 用户可见变化（全部 3 条，均为低风险）

1. 识别页 FunASR 模型下拉从 3 项（含 1 灰显）变 **2 项**（SenseVoice Small / Fun-ASR-Nano 实验性）。
2. 手改/迁移的旧档含 mlt 值 → 启动日志多一行归一修正记录，实际落 SenseVoice Small。
3. 其余零变化——mlt 本来就不可选、不可下载、无引擎实现、不占下载带宽。

## 5. 测试处置总表

| 文件 | 测试 | 处置 |
|---|---|---|
| lt-proto settings.rs | `legacy_engine_alias_migration` | 改写 mlt 子案例为收敛断言（3.2④） |
| lt-proto settings.rs | `normal_engine_values_untouched` | mlt 子案例换 `funasr-nano-2512`（3.2⑤） |
| lt-proto settings.rs | `retired_mlt_values_converge` | **新增**（3.2⑥） |
| lt-models registry.rs | `funasr_entry_keys` | 注释改写，断言保留（3.3⑤） |
| lt-models registry.rs | `every_funasr_key_has_entry_or_is_the_documented_ghost` | **更名** `every_funasr_key_has_entry`，删幽灵例外与 ghost 断言（3.3④） |
| lt-models cache.rs | `missing_models_semantics` | mlt 子案例注释改写（断言不动，仍过：回退报 sensevoice 缺失）（3.4） |
| lt-orchestrator pipeline.rs | `funasr_dispatch_nano_vs_sensevoice` | mlt 子案例注释改写（断言不动） |
| lt-orchestrator pipeline.rs | `mlt_falls_back_to_sensevoice` | **更名** `retired_mlt_falls_back_to_sensevoice`（断言不动） |
| lt-ui vad.rs | `funasr_model_items_order_and_gating` | **更名** `funasr_model_items_order_and_labels`，两值断言（3.6⑧） |
| lt-ui vad.rs | `cache_status_probes_models_dir` | mlt 断言保留，注释改写（3.6⑨） |
| lt-ui mod.rs | `panel_ui_smoke_renders_all_pages_headless` | 注释改写（测试体不动，防御分支仍可达） |

净变化：**+1 测试**（603+9 → 604+9）。测试里**刻意保留** 4 处 `"funasr-mlt-nano-2512"` 字符串作为旧档兼容回归锁（settings 收敛 / registry None / pipeline 回退 / UI 索引回退各一层）——它们锁的是"退役键不炸、收敛正确"，不是 mlt 功能。

## 6. 明确不动清单（防止施工顺手超范围）

| 项 | 理由 |
|---|---|
| `FUNASR_NANO` / `SENSEVOICE_SMALL` 注册表条目、`engines/nano.rs`、worker nano 臂 | Fun-ASR-Nano 本体，与 mlt 无关 |
| `resolve_funasr_entry` 函数本体 | 非法键回退语义仍在（D-86 后 mlt 残留键即非法键，走同一回退） |
| `CacheStatus::Unavailable` 变体 | 两个可达路径仍在：未知引擎早退 + `funasr_entry` None 防御臂；删变体反而缩小防御面 |
| sanitize 结构（别名优先→引擎域→模型域的顺序与兜底臂） | 正是旧档收敛的机制本体，一字不动 |
| `docs/archive/**` | 只读纪律；D-14 决策史保留原文 |
| `docs/architecture-review.md` R25 条目、`docs/architecture-v2.md` R25 行 | 评审/修复的历史记录，记录的是当时事实 |
| `docs/asr-model-survey-2026.md` | 未跟踪待裁决稿；批准本方案即视为其决策 G（维持幽灵键）改判为移除，稿件本身施工时不改 |
| lt-asr crate 全部 | mlt 从未有引擎实现，零触点 |
| `model_experimental` i18n 键与 nano 实验性标注 | nano 的 UI 语义，保留 |

## 7. 验收门禁（施工收工前提）

1. `powershell -File scripts/precommit.ps1` 六项全过（fmt / clippy -D warnings / 四守护脚本）。
2. `cargo test --workspace` 全绿，**604+9** + 9 ignored。
3. `cargo build --release -p lt-app` 通过（无 C 侧/构建脚本改动，预期无风险，可并入例行构建）。
4. 冒烟（可选，需真实模型缓存）：临时 `LIVETRANSLATE_CONFIG_DIR`，settings.json 手写 `"funasr_model": "funasr-mlt-nano-2512"` → 启动 → 日志出现修正行、下拉落 SenseVoice Small、ASR 正常拉起。
5. 提交纪律：本文件先独立 `docs(funasr-mlt-removal): D-86 移除方案定稿` 提交；实现提交 `feat(models): 移除 Fun-ASR-MLT-Nano 幽灵值与灰显机制（D-86，PROTO_VERSION=7）`；AGENTS.md 待办区补一行 D-86 完工记录（随实现提交）。

## 8. 待拍板项（推荐已内嵌于上文，均可改）

| # | 决策点 | 推荐 | 备选与代价 |
|---|---|---|---|
| A | PROTO_VERSION 6→7 | **递增**：冻结规则对"删除既有契约项"的硬要求，D-81 同款先例 | 不递增 = 契约记账失真，违反修订案字面义；无技术代价差异（无人消费该值），纯纪律问题 |
| B | 测试保留 4 处 mlt 字符串作兼容锁 | **保留**：锁住旧档收敛不回归，四层各司其职 | 全删净 = 字面零残留，但 registry/pipeline/UI 三层收敛锁丢失（settings 一层仍在） |
| C | i18n 键 `model_mlt_disabled_hint` 处置 | **更名 `model_unavailable_hint` + 通用措辞**：Unavailable 防御分支仍需文案 | 直接删键 + 分支显示空 = 防御面静默，不可观测，不推荐 |
