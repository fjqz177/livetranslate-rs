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
            sys: None,
            sys_last: None,
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
