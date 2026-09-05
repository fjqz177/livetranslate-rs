//! UI 共享状态 —— 全部只被 UI 线程读写（事件经 UiMsg 进入，无锁竞争）。

use lt_proto::Settings;
use std::time::Instant;

/// 窗口标识（4 个常驻窗口）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WinId {
    Overlay,
    Subtitle,
    Panel,
    Log,
}

impl WinId {
    /// 稳定字符串 id（viewport 哈希源）
    pub fn key(self) -> &'static str {
        match self {
            WinId::Overlay => "overlay",
            WinId::Subtitle => "subtitle",
            WinId::Panel => "panel",
            WinId::Log => "log",
        }
    }

    pub fn title(self) -> String {
        match self {
            WinId::Overlay => "LiveTranslate".into(),
            WinId::Subtitle => "LiveTranslate Subtitle".into(),
            // 面板/日志标题走 i18n；初始化后由窗口层刷新
            WinId::Panel => lt_i18n::t("window_control_panel"),
            WinId::Log => lt_i18n::t("window_log"),
        }
    }
}

/// 定时重绘条目（监视节拍 / 动画驱动等；WaitUntil 调度的依据）
#[derive(Debug, Clone)]
pub struct Tick {
    pub at: Instant,
    pub win: WinId,
}

/// 监视条数据：音频侧来自 UpdateMonitor 事件（每 chunk），系统侧 1s 节流采样
#[derive(Debug, Clone, Copy, Default)]
pub struct MonitorData {
    pub rms: f32,
    pub vad: f32,
    pub mic_rms: Option<f32>,
    /// CPU 全局占用 %（sysinfo 1s 采样）
    pub cpu: f32,
    /// 内存 GB（used / total）
    pub ram_used_gb: f64,
    pub ram_total_gb: f64,
}

/// 悬浮窗单条消息（对照原版 ChatMessage 的数据字段）。
/// translation / tl_ms 为 M3 翻译链路占位（UpdateTranslation 接线前恒为 None / 0.0）。
#[derive(Debug, Clone)]
pub struct OverlayMessage {
    pub id: u64,
    /// "HH:MM:SS"（ASR 事件已格式化）
    pub timestamp: String,
    pub original: String,
    /// 源语言标签（"zh"/"en"…）
    pub lang: String,
    pub asr_ms: f64,
    /// 译文（M3 翻译链路占位）
    pub translation: Option<String>,
    /// 翻译耗时（M3 翻译链路占位）
    pub tl_ms: f64,
}

/// 全局 UI 状态。随里程碑逐步扩充（消息流/监视数据/统计……）。
pub struct AppState {
    pub settings: Settings,
    /// 管道运行中（托盘"暂停/恢复"与悬浮窗启停按钮共用）
    pub running: bool,
    /// 各窗口可见性（托盘与 CloseRequested 控制）
    pub visible: std::collections::HashMap<WinId, bool>,
    /// 悬浮窗穿透/置顶/自动滚动/任务栏（托盘子菜单与 DragHandle 复选框三向同步）
    pub ov_click_through: bool,
    pub ov_topmost: bool,
    pub ov_auto_scroll: bool,
    pub ov_taskbar: bool,
    /// 待处理的定时重绘
    pub ticks: Vec<Tick>,
    /// 监视条数据链（任务 1.6）
    pub monitor: MonitorData,
    /// 悬浮窗消息链（AddMessage 事件追加，上限 50 条、删最旧）
    pub messages: Vec<OverlayMessage>,
    /// ASR 设备标签（"SenseVoice Small" 等；不可用时 "ASR unavailable"）
    pub asr_label: Option<String>,
    /// sysinfo 实例与上次采样时刻（1s 节流）
    sys: Option<sysinfo::System>,
    sys_last: Option<Instant>,
}

impl AppState {
    pub fn new(settings: Settings) -> Self {
        let mut visible = std::collections::HashMap::new();
        visible.insert(WinId::Overlay, true);
        visible.insert(WinId::Subtitle, settings.subtitle_mode.enabled);
        visible.insert(WinId::Panel, true);
        visible.insert(WinId::Log, false); // 原版：启动即建但隐藏
        Self {
            settings,
            running: true,
            visible,
            // 原版默认：置顶√、自动滚动√、穿透×、任务栏×
            ov_click_through: false,
            ov_topmost: true,
            ov_auto_scroll: true,
            ov_taskbar: false,
            ticks: Vec::new(),
            monitor: MonitorData::default(),
            messages: Vec::new(),
            asr_label: None,
            sys: None,
            sys_last: None,
        }
    }

    /// 追加一条识别消息；超过 50 条删最旧（原版 _max_messages = 50）。
    pub fn push_message(&mut self, msg: OverlayMessage) {
        self.messages.push(msg);
        if self.messages.len() > 50 {
            self.messages.remove(0);
        }
    }

    /// 1s 节流的系统采样（CPU/RAM）；在监视节拍触发时调用。
    /// sysinfo 的 CPU 占用需要两次间隔采样才有意义，首次为 0 属预期。
    pub fn sample_system(&mut self) {
        let now = Instant::now();
        if !self
            .sys_last
            .map_or(true, |t| now.duration_since(t) >= std::time::Duration::from_secs(1))
        {
            return;
        }
        self.sys_last = Some(now);
        let sys = self.sys.get_or_insert_with(sysinfo::System::new);
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        let m = &mut self.monitor;
        m.cpu = sys.global_cpu_usage();
        const GB: f64 = 1024.0 * 1024.0 * 1024.0;
        m.ram_used_gb = sys.used_memory() as f64 / GB;
        m.ram_total_gb = sys.total_memory() as f64 / GB;
    }

    /// 设置 1s 监视节拍（悬浮窗 MonitorBar；M0 用于占位时钟）
    pub fn schedule_monitor_tick(&mut self, win: WinId) {
        let at = Instant::now() + std::time::Duration::from_secs(1);
        if let Some(t) = self.ticks.iter_mut().find(|t| t.win == win) {
            t.at = at;
        } else {
            self.ticks.push(Tick { at, win });
        }
    }

    /// 取走所有到期的节拍；返回需要重绘的窗口集合
    pub fn drain_due_ticks(&mut self) -> Vec<WinId> {
        let now = Instant::now();
        let mut due = Vec::new();
        self.ticks.retain(|t| {
            if t.at <= now {
                due.push(t.win);
                false
            } else {
                true
            }
        });
        due.dedup();
        due
    }

    /// 最近的节拍时刻（None = 无定时事项，可无限期等待）
    pub fn next_tick(&self) -> Option<Instant> {
        self.ticks.iter().map(|t| t.at).min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: u64) -> OverlayMessage {
        OverlayMessage {
            id,
            timestamp: format!("12:00:{:02}", id % 60),
            original: format!("消息 {id}"),
            lang: "zh".into(),
            asr_ms: 100.0,
            translation: None,
            tl_ms: 0.0,
        }
    }

    #[test]
    fn messages_cap_at_50_keep_newest() {
        let mut st = AppState::new(Settings::default());
        for i in 1..=55u64 {
            st.push_message(msg(i));
        }
        // 上限 50 条，保留的是最新的 50 条（首条 id 应为第 6 条）
        assert_eq!(st.messages.len(), 50);
        assert_eq!(st.messages[0].id, 6);
        assert_eq!(st.messages.last().unwrap().id, 55);
    }
}
