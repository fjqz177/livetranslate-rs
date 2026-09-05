//! UI 共享状态 —— 全部只被 UI 线程读写（事件经 UiMsg 进入，无锁竞争）。

use lt_proto::{Cmd, Settings};
use std::time::{Duration, Instant};

/// 窗口标识（4 个常驻窗口 + 启动流对话框）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WinId {
    Overlay,
    Subtitle,
    Panel,
    Log,
    /// 启动流对话框（首启向导/缺模型下载/模型加载共用一个常规装饰窗口；
    /// 标题随启动流阶段由窗口层 set_title 动态化）
    Setup,
}

impl WinId {
    /// 稳定字符串 id（viewport 哈希源）
    pub fn key(self) -> &'static str {
        match self {
            WinId::Overlay => "overlay",
            WinId::Subtitle => "subtitle",
            WinId::Panel => "panel",
            WinId::Log => "log",
            WinId::Setup => "setup",
        }
    }

    pub fn title(self) -> String {
        match self {
            WinId::Overlay => "LiveTranslate".into(),
            WinId::Subtitle => "LiveTranslate Subtitle".into(),
            // 面板/日志标题走 i18n；初始化后由窗口层刷新
            WinId::Panel => lt_i18n::t("window_control_panel"),
            WinId::Log => lt_i18n::t("window_log"),
            // 创建时的兜底标题；向导/下载/加载阶段由窗口层按流设置真实标题
            WinId::Setup => "LiveTranslate".into(),
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

/// 悬浮窗单条消息（对照原版 ChatMessage 的数据字段）
#[derive(Debug, Clone)]
pub struct OverlayMessage {
    pub id: u64,
    /// "HH:MM:SS"（ASR 事件已格式化）
    pub timestamp: String,
    pub original: String,
    /// 源语言标签（"zh"/"en"…）
    pub lang: String,
    pub asr_ms: f64,
    /// 译文（流式期间为累积部分文本；同语言为空串）
    pub translation: Option<String>,
    /// 翻译耗时（流式期间 0，完成事件时更新）
    pub tl_ms: f64,
}

/// 翻译/用量统计（UpdateStats 事件；MonitorBar stats 段渲染，M4 完备）
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct OverlayStats {
    pub asr_n: u64,
    pub tl_n: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost: f64,
}

/// 首启向导的阶段（原版 SetupWizardDialog 用控件可用性表达，这里显式化）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardPhase {
    /// 倒计时进行中：按钮带 "(Ns)"，点击或归零自动开始下载
    Idle,
    /// 下载中：按钮禁用、hub/代理控件禁用（原版 setEnabled(False)）
    Downloading,
    /// 失败可重试：按钮变 t("btn_retry")，控件恢复可用
    Failed,
    /// 成功：追加 t("download_complete")，500ms 后收尾关窗（原版 singleShot(500, accept)）
    Done,
}

/// 首启向导状态（原版 SetupWizardDialog）
#[derive(Debug, Clone)]
pub struct WizardState {
    /// 0=ModelScope 1=HF；默认 zh→0 否则 1（lt_i18n::get_lang()，原版按系统语言选源）
    pub hub_index: usize,
    /// 0 不使用 / 1 系统代理 / 2 自定义；默认 1（原版跟随系统代理）
    pub proxy_index: usize,
    /// 自定义代理 URL；仅 proxy_index==2 可编辑，占位提示 http://127.0.0.1:7890
    pub proxy_url: String,
    /// 15 起步、每秒递减、仅 Idle 期有效；任一控件变更重置 15（原版 _reset_countdown）
    pub countdown: i32,
    pub phase: WizardPhase,
    /// 下载日志（上限 500 条，满删最旧）
    pub log: Vec<String>,
}

impl WizardState {
    /// 按当前界面语言取默认值（原版构造函数：中文系统 → ModelScope，其余 → HuggingFace）
    pub fn new() -> Self {
        let hub_index = if lt_i18n::get_lang() == "zh" { 0 } else { 1 };
        Self {
            hub_index,
            proxy_index: 1,
            proxy_url: String::new(),
            countdown: 15,
            phase: WizardPhase::Idle,
            log: Vec::new(),
        }
    }

    /// hub 下拉索引 → 命令契约字符串（原版 `"ms" if index == 0 else "hf"`）
    pub fn hub_arg(&self) -> String {
        if self.hub_index == 0 { "ms".into() } else { "hf".into() }
    }

    /// 代理选择 → 命令契约字符串（原版 _download_proxy：custom 且空白回退 "system"）
    pub fn proxy_arg(&self) -> String {
        match self.proxy_index {
            1 => "system".into(),
            2 => {
                let trimmed = self.proxy_url.trim();
                if trimmed.is_empty() { "system".into() } else { trimmed.to_string() }
            }
            _ => "none".into(),
        }
    }
}

/// 启动流状态机（替代原版 main() 里的模态对话框序列：SetupWizardDialog / ModelDownloadDialog）
#[derive(Debug, Clone)]
pub enum StartupFlow {
    /// 首启向导（settings 文件不存在）
    Wizard(WizardState),
    /// 非首启但模型缺失（names 为逗号连接的模型显示名）
    DownloadMissing {
        names: String,
        log: Vec<String>,
        failed: Option<String>,
        finished: bool,
    },
    /// 正常运行
    Ready,
}

/// lt-app 装配层构造初始启动流（首启=向导；缺模型=下载对话框；否则 Ready）。
/// lt-app 不直接触碰 WizardState 的构造细节。
pub fn startup_flow(first_launch: bool, missing_names: Vec<String>) -> StartupFlow {
    if first_launch {
        StartupFlow::Wizard(WizardState::new())
    } else if !missing_names.is_empty() {
        // 原版 ModelDownloadDialog：names = ", ".join(m["name"] for m in missing_models)
        StartupFlow::DownloadMissing {
            names: missing_names.join(", "),
            log: Vec::new(),
            failed: None,
            finished: false,
        }
    } else {
        StartupFlow::Ready
    }
}

/// 追加一行日志；超过 500 条删最旧（原版 QTextEdit 无限追加，这里按规格限幅防内存膨胀）
pub fn push_log_line(log: &mut Vec<String>, line: String) {
    log.push(line);
    if log.len() > 500 {
        log.remove(0);
    }
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
    /// 翻译/用量统计（UpdateStats 事件更新）
    pub stats: OverlayStats,
    /// ASR 设备标签（"SenseVoice Small" 等；不可用时 "ASR unavailable"）
    pub asr_label: Option<String>,
    /// 启动流状态机（首启向导/缺模型下载/Ready）
    pub startup: StartupFlow,
    /// 模型加载对话框（_ModelLoadDialog）：Some(label)=显示中
    pub load_dialog: Option<String>,
    /// Worker 命令出口（宿主构造时注入；widget 代码经 send_cmd 直接发送）
    pub cmd_tx: Option<std::sync::mpsc::Sender<Cmd>>,
    /// 缺模型下载失败后点"关闭"：请求退出应用（原版 reject → main 返回退出）
    pub quit_requested: bool,
    /// sysinfo 实例与上次采样时刻（1s 节流）
    sys: Option<sysinfo::System>,
    sys_last: Option<Instant>,
}

impl AppState {
    /// 常规构造：无启动流（flow=Ready），主窗口按默认可见性
    pub fn new(settings: Settings) -> Self {
        Self::with_startup(settings, StartupFlow::Ready)
    }

    /// 指定初始启动流构造。对照原版 main()：启动流进行中（首启向导/缺模型下载）
    /// 4 个主窗口初始全部不可见，仅 Setup 对话框可见，accept 之后才 reveal 主窗口。
    pub fn with_startup(settings: Settings, flow: StartupFlow) -> Self {
        let startup_pending = !matches!(flow, StartupFlow::Ready);
        let mut visible = std::collections::HashMap::new();
        visible.insert(WinId::Overlay, !startup_pending);
        visible.insert(WinId::Subtitle, !startup_pending && settings.subtitle_mode.enabled);
        visible.insert(WinId::Panel, !startup_pending);
        visible.insert(WinId::Log, false); // 原版：启动即建但隐藏
        // Setup 对话框窗口：仅启动流进行中初始可见（运行期 load_dialog 单独控制）
        visible.insert(WinId::Setup, startup_pending);
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
            stats: OverlayStats::default(),
            asr_label: None,
            startup: flow,
            load_dialog: None,
            cmd_tx: None,
            quit_requested: false,
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

    /// 按 id 从最新往回找消息索引（消息链短，线性即可）
    fn find_message_mut(&mut self, id: u64) -> Option<&mut OverlayMessage> {
        self.messages.iter_mut().rev().find(|m| m.id == id)
    }

    /// 流式译文增量（原版 update_streaming：写入累积部分文本）
    pub fn update_streaming(&mut self, id: u64, partial: String) {
        if let Some(m) = self.find_message_mut(id) {
            m.translation = Some(partial);
        }
    }

    /// 译文完成（原版 update_translation；空文本=同语言/无翻译，同样置 Some）
    pub fn update_translation(&mut self, id: u64, text: String, tl_ms: f64) {
        if let Some(m) = self.find_message_mut(id) {
            m.translation = Some(text);
            m.tl_ms = tl_ms;
        }
    }

    /// 统计快照更新（原版 update_stats）
    pub fn update_stats(&mut self, stats: OverlayStats) {
        self.stats = stats;
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
        let at = Instant::now() + Duration::from_secs(1);
        if let Some(t) = self.ticks.iter_mut().find(|t| t.win == win) {
            t.at = at;
        } else {
            self.ticks.push(Tick { at, win });
        }
    }

    /// 安排一次 Setup 窗节拍（同窗去重：覆盖既有节拍时刻）。
    /// 向导倒计时（1s/拍）与下载成功后的 500ms 收尾延迟共用。
    pub fn schedule_setup_tick(&mut self, delay: Duration) {
        let at = Instant::now() + delay;
        if let Some(t) = self.ticks.iter_mut().find(|t| t.win == WinId::Setup) {
            t.at = at;
        } else {
            self.ticks.push(Tick { at, win: WinId::Setup });
        }
    }

    /// 取消 Setup 节拍（下载开始即停倒计时定时器，等价原版 _auto_timer.stop()）
    pub fn cancel_setup_tick(&mut self) {
        self.ticks.retain(|t| t.win != WinId::Setup);
    }

    /// 若启动流需要节拍（向导倒计时 / 成功后 500ms 收尾延迟）则安排 Setup 节拍。
    /// 倒计时按 1s 一拍；收尾延迟按 500ms。
    pub fn kick_setup_tick(&mut self) {
        if !crate::windows::setup::needs_setup_tick(self) {
            return;
        }
        let idle_countdown = matches!(
            &self.startup,
            StartupFlow::Wizard(w) if w.phase == WizardPhase::Idle
        );
        let delay = if idle_countdown {
            Duration::from_secs(1)
        } else {
            Duration::from_millis(500)
        };
        self.schedule_setup_tick(delay);
    }

    /// 向导"开始下载"统一入口：倒计时归零自动触发与按钮点击共用（原版 _start_download）。
    /// 切 Downloading、停倒计时、按当前 hub/proxy 选择发 StartDownload；重复触发忽略。
    pub fn wizard_auto_start(&mut self) {
        let (hub, proxy) = match &self.startup {
            StartupFlow::Wizard(w) if !matches!(w.phase, WizardPhase::Downloading) => {
                (w.hub_arg(), w.proxy_arg())
            }
            _ => return,
        };
        if let StartupFlow::Wizard(w) = &mut self.startup {
            w.phase = WizardPhase::Downloading;
        }
        self.cancel_setup_tick();
        self.send_cmd(Cmd::StartDownload { hub, proxy });
    }

    /// UI → 管道命令出口（cmd_tx 由宿主构造时注入）；发送失败仅记 debug 防打屏
    pub fn send_cmd(&self, cmd: Cmd) {
        match &self.cmd_tx {
            Some(tx) => {
                if let Err(e) = tx.send(cmd) {
                    tracing::debug!("Cmd 发送失败（接收端已关闭）: {e}");
                }
            }
            None => tracing::debug!("cmd_tx 未注入，命令被丢弃: {cmd:?}"),
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

    #[test]
    fn translation_updates_target_latest_message_with_id() {
        let mut st = AppState::new(Settings::default());
        // 同 id 消息出现两次（现实中不发生，防御语义：取最新）
        st.push_message(msg(1));
        st.push_message(msg(2));

        st.update_streaming(2, "partial".into());
        assert_eq!(st.messages.last().unwrap().translation.as_deref(), Some("partial"));
        assert_eq!(st.messages.last().unwrap().tl_ms, 0.0);

        st.update_translation(2, "done".into(), 320.0);
        let m = st.messages.last().unwrap();
        assert_eq!(m.translation.as_deref(), Some("done"));
        assert_eq!(m.tl_ms, 320.0);

        // 不存在的 id：无害 no-op
        st.update_translation(999, "ghost".into(), 1.0);
        assert!(st.messages.iter().all(|m| m.id != 999));

        // 淘汰边界：id=1 已被挤出 50 条窗口（仅 2 条时不适用，直接验证 id=1 更新）
        st.update_translation(1, "first".into(), 5.0);
        assert_eq!(st.messages[0].translation.as_deref(), Some("first"));
    }

    #[test]
    fn same_language_empty_translation_still_marked() {
        let mut st = AppState::new(Settings::default());
        st.push_message(msg(7));
        // 同语言回空译文：translation=Some("")，消息标记完成
        st.update_translation(7, String::new(), 0.0);
        let m = st.messages.last().unwrap();
        assert_eq!(m.translation.as_deref(), Some(""));
    }

    #[test]
    fn stats_snapshot_replaces_wholesale() {
        let mut st = AppState::new(Settings::default());
        st.update_stats(OverlayStats {
            asr_n: 10,
            tl_n: 8,
            prompt_tokens: 1000,
            completion_tokens: 500,
            cost: 0.002,
        });
        assert_eq!(st.stats.asr_n, 10);
        assert_eq!(st.stats.cost, 0.002);
    }

    #[test]
    fn log_lines_cap_at_500_keep_newest() {
        let mut log = Vec::new();
        for i in 1..=601 {
            push_log_line(&mut log, format!("line {i}"));
        }
        // 上限 500 条，保留的是最新的 500 条（首行应为第 102 行）
        assert_eq!(log.len(), 500);
        assert_eq!(log[0], "line 102");
        assert_eq!(log.last().unwrap(), "line 601");
    }

    #[test]
    fn wizard_defaults_follow_lang() {
        // 中文界面 → ModelScope，其余 → HuggingFace（原版按系统语言选源）
        lt_i18n::set_lang("zh");
        let w = WizardState::new();
        assert_eq!(w.hub_index, 0);
        assert_eq!(w.proxy_index, 1); // 默认跟随系统代理
        assert_eq!(w.countdown, 15);
        assert_eq!(w.phase, WizardPhase::Idle);
        assert!(w.log.is_empty());
        lt_i18n::set_lang("en");
        let w = WizardState::new();
        assert_eq!(w.hub_index, 1);
    }

    #[test]
    fn wizard_hub_proxy_args_mapping() {
        let mut w = WizardState::new();
        // hub 映射：0=ModelScope→"ms"，1=HuggingFace→"hf"
        w.hub_index = 0;
        assert_eq!(w.hub_arg(), "ms");
        w.hub_index = 1;
        assert_eq!(w.hub_arg(), "hf");
        // 代理映射（原版 _download_proxy）
        w.proxy_index = 0;
        assert_eq!(w.proxy_arg(), "none");
        w.proxy_index = 1;
        assert_eq!(w.proxy_arg(), "system");
        w.proxy_index = 2;
        w.proxy_url = "".into();
        assert_eq!(w.proxy_arg(), "system"); // custom 空白回退系统代理
        w.proxy_url = " http://127.0.0.1:7890 ".into();
        assert_eq!(w.proxy_arg(), "http://127.0.0.1:7890"); // 去首尾空白
    }

    #[test]
    fn wizard_auto_start_sends_start_download() {
        let mut w = WizardState::new();
        w.hub_index = 1;
        w.proxy_index = 0;
        let (tx, rx) = std::sync::mpsc::channel();
        let mut st = AppState::with_startup(Settings::default(), StartupFlow::Wizard(w));
        st.cmd_tx = Some(tx);
        st.schedule_setup_tick(Duration::from_secs(1));

        st.wizard_auto_start();

        let StartupFlow::Wizard(w) = &st.startup else {
            panic!("应仍处于向导流");
        };
        assert_eq!(w.phase, WizardPhase::Downloading);
        // 下载开始即停倒计时定时器（原版 _auto_timer.stop()）
        assert!(st.ticks.iter().all(|t| t.win != WinId::Setup));
        match rx.try_recv() {
            Ok(Cmd::StartDownload { hub, proxy }) => {
                assert_eq!(hub, "hf");
                assert_eq!(proxy, "none");
            }
            other => panic!("应发送 StartDownload，实际 {other:?}"),
        }
        // 下载中重复触发应被忽略（不重复发命令）
        st.wizard_auto_start();
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn with_startup_hides_main_windows_when_pending() {
        // 首启向导：4 个主窗口初始全部不可见，仅 Setup 可见
        let st = AppState::with_startup(Settings::default(), startup_flow(true, vec![]));
        for id in [WinId::Overlay, WinId::Subtitle, WinId::Panel, WinId::Log] {
            assert!(!*st.visible.get(&id).unwrap(), "{id:?} 应隐藏");
        }
        assert!(*st.visible.get(&WinId::Setup).unwrap());

        // 缺模型下载：同样全隐藏；names 为逗号连接的显示名
        let flow = startup_flow(false, vec!["Silero VAD".into(), "SenseVoice Small".into()]);
        let StartupFlow::DownloadMissing { names, .. } = &flow else {
            panic!("应为缺模型下载流");
        };
        assert_eq!(names, "Silero VAD, SenseVoice Small");
        let st = AppState::with_startup(Settings::default(), flow);
        assert!(!*st.visible.get(&WinId::Overlay).unwrap());
        assert!(*st.visible.get(&WinId::Setup).unwrap());

        // Ready：维持现有默认（Log 仍隐藏），Setup 不可见
        let st = AppState::new(Settings::default());
        assert!(*st.visible.get(&WinId::Overlay).unwrap());
        assert!(!*st.visible.get(&WinId::Setup).unwrap());
        assert!(!*st.visible.get(&WinId::Log).unwrap());
    }
}
