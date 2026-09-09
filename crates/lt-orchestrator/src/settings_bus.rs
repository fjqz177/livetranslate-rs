//! 设置总线（架构 2.0 W4，docs/architecture-v2.md §3.2.3）。
//!
//! 病灶：Settings 运行时存在 7 个副本 + 人肉同步边（§8.2-3/§3.2.3 表），
//! qwen3 钳制直接写穿共享 VAD 生效值（R18）。
//!
//! 目标：**arc-swap 不可变快照，单写者发布，读者无锁**——发布提交一份
//! `Settings`，派生视图（VAD 生效值/ASR 语言+双 pad/翻译目标+超时）在同一
//! 次发布内整体重算（幂等：ApplySettings 重放与专用快捷命令殊途同归）。
//!
//! 写者/读者不变量（INV7）：
//! - `publish` 唯一调用点 = 组合根（lt-app::AppShell）在 **winit 主线程**
//!   （UI 帧编辑 / shell 命令处理），任何其它线程不得发布；
//! - 读者（capture 逐轮、ASR transcribe 前、翻译池提交前、下载 targets）
//!   任意线程 `load()` 无锁（arc_swap load 是 lock-free wait-free）。
//!
//! 与方案 §3.2.3 的差异注记：方案示例含 `engine: EngineKey` 视图——W4
//! 实测**无读者**（引擎切换走命令载荷、StartDownload 读 `raw` 全集、
//! 钳制只读 `raw.asr_engine`），按"派生视图必须有读者"原则不落空字段；
//! 档位规范化（R21）待 W6 统一处理。

use lt_audio::VadSettings;
use lt_proto::Settings;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// qwen3 生效段长上限（AH-8/D-28，自 pipeline.rs 随总线迁入）：`MAX_TOTAL_LEN=512`
/// 为 audio+输出共享 token 预算，超长段有静默截尾风险；15s 为保守取值，
/// S0 校准（docs/archive/asr-hardening.md §6-T1）实测后可调。
pub(crate) const QWEN3_MAX_SEGMENT_SECS: f64 = 15.0;

/// 目标语言视图（D-74：同语言免翻译比较的目标语言侧归一——`zh` vs
/// `zh-CN` 的比较不再误判；归一仅发生在本派生点，用户设置原值永存）
#[derive(Debug, Clone, PartialEq)]
pub struct TlView {
    pub target_language: String,
    pub timeout: u32,
}

/// 一次发布的不可变生效快照（内部字段仅读；Clone 廉价——均为 Arc/String）
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveSettings {
    /// 提交态设置（UI 编辑/落盘真值；publish 时克隆一次，永不被派生改写）
    pub raw: Arc<Settings>,
    /// VAD 生效值：已按当前引擎钳制（overlay）——qwen3 时 max_speech
    /// 收敛到 15s，**raw 的 max_speech_duration 保持用户值**（R18 根除：
    /// 现状引擎切换直接写穿共享 VAD 生效值，切离后靠用户"应用"恢复）
    pub vad: VadSettings,
    /// ASR 生效视图：语言（段过滤/worker 装配）+ 双 pad（引擎切换装配，
    /// AH-3 语义——运行时改 pad 后切换不再回退启动快照旧值）
    pub asr_lang: lt_asr::AsrEffectiveSettings,
    /// 翻译生效视图（提交翻译前读；同语言判定用归一目标）
    pub tl: TlView,
    /// 单调号：读者廉价比对"是否变了"（捕捉 version 变化即重应用总是一次
    /// 完整的发布，无半应用窗口）
    pub version: u64,
}

/// 设置总线（arc-swap 不可变快照；单写者发布，读者无锁）。
///
/// 生命周期：组合根创建（首次发布即版本 1）→ 持有分支注入管道各线程
/// （capture 逐轮读格、ASR transcribe 前、翻译池提交前、下载 targets）。
pub struct SettingsBus {
    current: Arc<arc_swap::ArcSwap<EffectiveSettings>>,
    version: AtomicU64,
}

impl SettingsBus {
    /// 以初始设置创建并发布版本 1
    pub fn new(raw: Settings) -> Self {
        Self {
            current: Arc::new(arc_swap::ArcSwap::from_pointee(derive(&raw, 1))),
            version: AtomicU64::new(1),
        }
    }

    /// 唯一写入口（INV7：仅在 winit 主线程调用；幂等——同一份 raw 的重复
    /// 发布只增版本号，派生视图重算结果一致）。返回新版本号。
    pub fn publish(&self, raw: Settings) -> u64 {
        let v = self.version.fetch_add(1, Ordering::Relaxed) + 1;
        self.current.store(Arc::new(derive(&raw, v)));
        v
    }

    /// 任意线程无锁读取当前生效快照
    pub fn load(&self) -> Arc<EffectiveSettings> {
        self.current.load_full()
    }
}

/// 发布时整体重算全部派生视图（VAD 钳制 overlay + ASR 视图 + 翻译视图）
fn derive(raw: &Settings, version: u64) -> EffectiveSettings {
    EffectiveSettings {
        raw: Arc::new(raw.clone()),
        vad: clamp_vad_for_engine(&raw.asr_engine, vad_from_settings(raw)),
        asr_lang: lt_asr::AsrEffectiveSettings {
            language: raw.asr_language.clone(),
            sensevoice_pad: raw.sensevoice_pad_seconds,
            whisper_pad: raw.whisper_pad_seconds,
        },
        tl: TlView {
            target_language: lt_proto::normalize_language(&raw.target_language),
            timeout: raw.timeout,
        },
        version,
    }
}

fn vad_from_settings(s: &Settings) -> VadSettings {
    VadSettings {
        mode: s.vad_mode.clone(),
        threshold: s.vad_threshold as f64,
        energy_threshold: s.energy_threshold as f64,
        min_speech_duration: s.min_speech_duration as f64,
        max_speech_duration: s.max_speech_duration as f64,
        silence_mode: s.silence_mode.clone(),
        silence_duration: s.silence_duration as f64,
    }
}

/// 按引擎钳制 VAD 生效值（AH-8/D-28）：settings/UI 保存原值，仅生效值收敛。
/// R18 根治：本函数是**唯一**钳制点，且只在发布派生时调用——引擎切换不再
/// 写穿共享 VAD（旧 pipeline.rs 直改 `clamp_max_speech` 的路径删除）。
fn clamp_vad_for_engine(engine: &str, mut s: VadSettings) -> VadSettings {
    if engine == "qwen3" && s.max_speech_duration > QWEN3_MAX_SEGMENT_SECS {
        tracing::info!(
            "qwen3: max_speech_duration 生效值钳制为 {QWEN3_MAX_SEGMENT_SECS}s（设置值 {}s）",
            s.max_speech_duration
        );
        s.max_speech_duration = QWEN3_MAX_SEGMENT_SECS;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_with(engine: &str, max: f32) -> Settings {
        Settings {
            asr_engine: engine.into(),
            max_speech_duration: max,
            target_language: "zh-CN".into(),
            ..Settings::default()
        }
    }

    /// R18 回归：qwen3 钳制只改生效视图，raw 永不改写；切离后立即解除
    #[test]
    fn clamp_is_overlay_never_write_through() {
        let bus = SettingsBus::new(settings_with("qwen3", 30.0));
        let eff = bus.load();
        assert_eq!(eff.vad.max_speech_duration, QWEN3_MAX_SEGMENT_SECS);
        assert_eq!(eff.raw.max_speech_duration, 30.0, "raw 必须保留用户值");

        // 切离 qwen3 → 下一次发布生效值立即恢复完整（旧实现靠用户"应用"）
        bus.publish(settings_with("funasr", 30.0));
        let eff2 = bus.load();
        assert_eq!(eff2.vad.max_speech_duration, 30.0);
        assert_eq!(eff2.raw.max_speech_duration, 30.0);
    }

    /// 未超上限不钳制；其它引擎原值放行
    #[test]
    fn clamp_vad_only_affects_qwen3_and_only_down() {
        let mk = |engine: &str, max: f32| {
            let bus = SettingsBus::new(settings_with(engine, max));
            bus.load().vad.max_speech_duration
        };
        assert_eq!(mk("qwen3", 8.0), 8.0);
        assert_eq!(mk("funasr", 30.0), 30.0);
        assert_eq!(mk("whisper", 30.0), 30.0);
    }

    /// AH-3 场景回归：ASR 语言/pad 运行时更新经发布抵达 asr_lang 视图
    /// （旧实现靠 TlSwitch 七镜像人肉同步；总线发布即同步，无"记得补边"）
    #[test]
    fn asr_view_follows_publish() {
        let bus = SettingsBus::new(Settings::default());
        let eff = bus.load();
        assert_eq!(eff.asr_lang.language, "auto");
        assert_eq!(eff.asr_lang.sensevoice_pad, 0.5);

        let mut s = Settings::default();
        s.asr_language = "yue".into();
        s.sensevoice_pad_seconds = 1.25;
        s.whisper_pad_seconds = 2.0;
        bus.publish(s);
        let eff = bus.load();
        assert_eq!(eff.asr_lang.language, "yue");
        assert_eq!(eff.asr_lang.sensevoice_pad, 1.25);
        assert_eq!(eff.asr_lang.whisper_pad, 2.0);
    }

    /// D-74：目标语言归一发生在派生点（raw 保留原值）
    #[test]
    fn tl_view_normalizes_target_language() {
        let bus = SettingsBus::new(settings_with("funasr", 8.0));
        let eff = bus.load();
        assert_eq!(eff.tl.target_language, "zh");
        assert_eq!(eff.raw.target_language, "zh-CN");
        assert_eq!(eff.tl.timeout, 10);
    }

    /// 版本单调递增（读者廉价比对依据）
    #[test]
    fn version_monotonic() {
        let bus = SettingsBus::new(Settings::default());
        assert_eq!(bus.load().version, 1);
        let v2 = bus.publish(Settings::default());
        assert_eq!(v2, 2);
        let v3 = bus.publish(Settings::default());
        assert_eq!(v3, 3);
        assert_eq!(bus.load().version, 3);
    }
}
