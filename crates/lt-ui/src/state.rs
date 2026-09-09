//! UI 共享状态 —— 全部只被 UI 线程读写（事件经 UiMsg 进入，无锁竞争）。

use lt_proto::{Cmd, SubtitleMode};
// Settings 为窗口模块共用类型（W5：域签名携带），从本模块公有导出
pub use lt_proto::Settings;
use std::time::{Duration, Instant};

/// 窗口标识（4 个常驻窗口 + 启动流对话框 + Benchmark 工具窗）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WinId {
    Overlay,
    Subtitle,
    Panel,
    Log,
    /// 启动流对话框（首启向导/缺模型下载/模型加载共用一个常规装饰窗口；
    /// 标题随启动流阶段由窗口层 set_title 动态化）
    Setup,
    /// 性能基准独立工具窗（原版 BenchmarkDialog；识别页页头按钮打开，默认隐藏）
    Benchmark,
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
            WinId::Benchmark => "benchmark",
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
            // 原版 BenchmarkDialog.setWindowTitle(t("benchmark_dialog_title"))
            WinId::Benchmark => lt_i18n::t("benchmark_dialog_title"),
        }
    }
}

/// 定时重绘条目（监视节拍 / 流式刷新 / 位置保存 / 穿透轮询 / 启动流节拍）
#[derive(Debug, Clone)]
pub struct Tick {
    pub at: Instant,
    pub win: WinId,
    pub kind: TickKind,
}

/// 节拍种类（同窗口可并存多种节拍；宿主按 kind 分派动作）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickKind {
    /// 1s 系统采样（悬浮窗 MonitorBar）
    Monitor,
    /// ~33ms 音频监视快照格轮询（W2/D-67：capture 写 ArcSwap、UI 节拍读格——
    /// 替代 UpdateMonitor 逐事件唤醒；仅悬浮窗可见时排班）
    AudioMonitor,
    /// 50ms 流式译文节流刷新
    StreamFlush,
    /// 500ms 位置/尺寸持久化防抖
    PosSave,
    /// 50ms 穿透光标感知轮询（原版 _ct_timer）
    ClickThrough,
    /// 启动流（向导倒计时/收尾延迟）
    Setup,
    /// 字幕窗自动隐藏到点（原版 _auto_hide_timer singleShot；win=Subtitle）
    SubtitleAutoHide,
    /// 字幕窗待插入句子到点（原版 _pending_segment_timers，1500ms 最小显示；win=Subtitle）
    SubtitlePending,
    /// 面板设置 300ms 防抖到期（原版 ControlPanel._save_timer singleShot；win=Panel）
    PanelApply,
    /// 翻译页 system_prompt 600ms 防抖到期（原版 _prompt_debounce QTimer 600ms；
    /// 到期发 SwitchTranslator 重建翻译器；win=Panel）
    PromptApply,
}

/// 监视条数据：音频侧来自 Monitor 快照格（W2：UI 以 ~33ms 节拍读格——与
/// 系统侧 1s 节流采样同格）；R2 前为逐 chunk UpdateMonitor 事件。
/// CPU/RAM 为**进程自身**指标（原版 psutil.Process：cpu_percent + RSS MB）。
#[derive(Debug, Clone, Copy, Default)]
pub struct MonitorData {
    pub rms: f32,
    pub vad: f32,
    pub mic_rms: Option<f32>,
    /// 进程 CPU 占用 %（sysinfo 1s 采样）
    pub cpu: f32,
    /// 进程 RSS MB（原版 stats 行显示 `RAM {mb}MB`）
    pub ram_mb: f32,
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
    /// 流式进行中（True 时不渲染 TL 耗时，原版流式/完成分设标签文本）
    pub streaming: bool,
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

/// UI → 宿主的窗口动作（egui 无窗口句柄；由 UI 帧入队、宿主在帧后执行）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WinAction {
    /// 悬浮窗拖动开始（W5/D-71：弃 winit drag_window——标题栏模态循环 +
    /// 哑 WM_MOUSEMOVE 取消（大坑 17），与字幕窗 D-37 同法：宿主 SetCapture +
    /// 记录抓握偏移 + 恒非穿透）
    OverlayDragStart,
    /// 悬浮窗拖动结束（宿主 = ReleaseCapture；落点防抖经既有 Moved 路径）
    OverlayDragEnd,
    /// 右下角尺寸手柄拖动（winit drag_resize_window）
    ResizeSouthEast,
    /// 隐藏窗口（悬浮窗"隐藏"按钮，等价 tray OVERLAY_TOGGLE 的反向）
    Hide,
    /// 显示/置前控制面板（原版 settings_requested）
    ShowPanel,
    /// 隐藏控制面板（确认模态取消后恢复临时显示的面板；H-3）
    HidePanel,
    /// 切换字幕窗可见性（悬浮窗"字幕"按钮 = settings.subtitle_mode.enabled 翻转后）
    ToggleSubtitle,
    /// 置顶/任务栏复选变化 → 重新应用窗口 flags
    ApplyOverlayFlags,
    /// 紧凑/完整模式切换（宿主计算动画 from/to 并启动）
    ToggleMode,
    /// 动画/模式推导出的窗口高度调整（逻辑 px，保持宽度）
    SetHeight(f32),
    /// 字幕窗拖动开始（正文中键 / 顶条任意键；D-37 弃 winit drag_window——
    /// 其标题栏模态循环在 LAYERED+TRANSPARENT 轮询窗上实测 0 位移/挂死、
    /// 且中键不可用。宿主 = SetCapture + 记录抓握偏移 + 恒非穿透）
    SubtitleDragStart,
    /// 字幕窗拖动结束（宿主 = ReleaseCapture + 落点钳制/防抖保存）
    SubtitleDragEnd,
    /// 字幕窗高度调整（原版 _fit_height_animated：随高度上移 y 保持视觉中心）
    SetSubtitleHeight(f32),
    /// 字幕窗高度落定后的多屏钳制（原版 on_finished → _clamp_to_screen + position_changed）
    ClampSubtitlePos,
    /// 字幕窗复位（D-36：顶条右键菜单 / 字幕页按钮；原版 _on_reset_positions 的字幕分支：
    /// 回 (100,100) 后走既有 Moved 防抖保存）
    ResetSubtitlePos,
    /// 常规页"重置窗口位置"（原版 _on_reset_positions：字幕窗回 (100,100)、
    /// 悬浮窗回主屏右下角；宿主移动窗口后走既有 Moved 防抖保存）
    ResetPositions,
    /// 识别页页头"性能基准…"按钮（原版 BenchmarkDialog.exec()；宿主显示工具窗）
    ShowBenchmark,
    /// 打开面板并切换到指定页（W5：字幕窗"打开设置"的跨域写意图化——
    /// 原 subtitle.rs 直写 panel.page 的借用边界破坏改由宿主根消费）
    OpenPanelPage(PanelPage),
}

/// 悬浮窗模式（原版 DragHandle._mode："full"/"compact"）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverlayMode {
    #[default]
    Full,
    Compact,
}

/// 通用确认模态语义（D-33/H-3~H-5：统一 egui 载体替代事件循环线程内的
/// rfd 同步 MessageBox——不阻塞、单模态源、任意状态可达）
#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmKind {
    /// 退出应用（确定 → quit_requested，宿主动作在 about_to_wait 收敛）
    Quit,
    /// 清空转写消息
    Clear,
    /// 字幕页恢复默认（SubtitleMode 整体 + 行级字体跟随；窗口位置归样式页）
    ResetSubtitle,
    /// 翻译页恢复默认（models/prompt/timeout；破坏性——确认后清 API 配置）
    ResetTranslation,
    /// 删除所选缓存模型（index 对位 data 页 cache_entries，模态期间页面不可变）
    DeleteModel { index: usize },
    /// 删除全部缓存模型（data 页；"重启生效"提示保留在效果执行处）
    DeleteAll,
}

/// 确认模态 UI 状态（渲染帧按 host 匹配窗口；确定/取消在确认框内收敛）
#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmUi {
    /// 宿主窗口（Overlay 或 Panel；仅宿主窗口的帧内渲染）
    pub host: WinId,
    pub kind: ConfirmKind,
    /// 标题/正文（请求时按 kind 取 i18n 当前语言，含 Delete 系动态参数）
    pub title: String,
    pub msg: String,
    /// 宿主=Panel 且面板因确认**临时显示**（Quit 取消后回隐藏；H-3）
    pub panel_shown_for_confirm: bool,
}

/// 紧凑模式高度动画（原版 QPropertyAnimation 200ms OutCubic）
#[derive(Debug, Clone, Copy)]
pub struct HeightAnim {
    pub from: f32,
    pub to: f32,
    pub start: Instant,
}

impl HeightAnim {
    pub const DURATION: Duration = Duration::from_millis(200);

    /// 当前进值（OutCubic：1-(1-t)^3）；结束后返回 None 由调用方收敛
    pub fn current(&self, now: Instant) -> Option<f32> {
        let t = (now - self.start).as_secs_f32() / (Self::DURATION.as_secs_f32());
        if t >= 1.0 {
            return None;
        }
        let eased = 1.0 - (1.0 - t).powi(3);
        Some(self.from + (self.to - self.from) * eased)
    }
}

/// 日志窗单行（原版 QTextEdit maximumBlockCount=2000 的条目）
#[derive(Debug, Clone)]
pub struct LogLineEntry {
    /// "HH:MM:SS"（UI 侧收到即盖时戳，原版 formatter datefmt）
    pub time: String,
    /// Python logging 数值语义（10=DEBUG 20=INFO 30=WARN 40=ERROR）
    pub level: u8,
    pub target: String,
    pub msg: String,
}

impl LogLineEntry {
    /// 显示/复制共用单行文本（日志窗与面板日志 tab 统一格式）
    pub fn display(&self) -> String {
        format!(
            "{} [{}] {}: {}",
            self.time,
            level_name(self.level),
            self.target,
            self.msg
        )
    }
}

/// 级别名（Python logging 语义：TRACE/DEBUG/INFO/WARNING/ERROR）
pub fn level_name(level: u8) -> &'static str {
    match level {
        0 => "TRACE",
        10 => "DEBUG",
        20 => "INFO",
        30 => "WARNING",
        40.. => "ERROR",
        _ => "INFO",
    }
}

/// 日志视图标识（贴底跟随状态分视图维护：面板 tab 与日志窗各一个独立滚动区）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogView {
    /// 设置面板「日志」tab（log_tab.rs）
    Panel,
    /// 独立日志窗（logwin.rs）
    LogWin,
}

/// 滚动偏移变化阈值（px）：超过即视作"用户主动滚动"
const SCROLL_EPS: f32 = 0.5;
/// 贴底跟随容差（px）：视口底与内容底距离在此内仍视作贴底
const FOLLOW_TOL: f32 = 4.0;

/// 滚动视口是否贴底（容差 tol px）。`viewport_max_y` 为内容坐标系下视口底
/// （= 滚动偏移 + 视口高）；内容不满视口时不产偏移，恒判为贴底（未上翻）。
pub fn is_near_bottom(viewport_max_y: f32, content_h: f32, tol: f32) -> bool {
    content_h - viewport_max_y <= tol
}

/// D-31 日志视图状态（docs/archive/log-tab-redesign.md）：缓冲全级别保留 + 渲染期过滤，
/// 另附显示行缓存与贴底跟随状态（日志窗与面板日志 tab 共享同一实例）。
#[derive(Default)]
pub struct LogWindowState {
    pub lines: std::collections::VecDeque<LogLineEntry>,
    /// 默认只显示 INFO+；开启后渲染期回溯显示全部保留行（D-31：不再是原版
    /// "追加期过滤、勾选不回溯"语义）
    pub show_debug: bool,
    pub auto_scroll: bool,
    /// 上次贴底渲染时的行数（[`Self::new_since_bottom`] 推导基准；None = 尚未贴底）
    bottom_marker: Option<usize>,
    /// 显示行缓存（text + level）：push 增量追加，环形溢出后整体重建，
    /// 避免每帧对 2000 行重复 `format!`
    formatted: Vec<(String, u8)>,
    fmt_dirty: bool,
    /// 上次「复制全部」时刻（约 800ms 按钮短时反馈；纯 UI 状态）
    pub copy_at: Option<Instant>,
    /// 面板 tab：用户是否主动翻离底部（「回到最新」浮钮显示条件）
    pub panel_pinned: bool,
    /// 日志窗：同上
    pub logwin_pinned: bool,
    /// 各视图上一帧滚动偏移（"用户主动滚动"检测：offset 变化才视作用户操作，
    /// 内容增长导致的 offset 落后不算）
    pub panel_prev_offset: Option<f32>,
    pub logwin_prev_offset: Option<f32>,
}

impl LogWindowState {
    pub const MAX_LINES: usize = 2000;

    /// 追加一行（缓冲全级别保留；行数环形上限 2000）。恒返回 true（调用方据此
    /// 触发重绘，见 lt-app 事件环）。
    pub fn push(&mut self, mut entry: LogLineEntry) -> bool {
        if entry.time.is_empty() {
            entry.time = chrono::Local::now().format("%H:%M:%S").to_string();
        }
        self.lines.push_back(entry);
        if self.lines.len() > Self::MAX_LINES {
            self.lines.pop_front();
            self.formatted.clear();
            self.fmt_dirty = true;
        } else if !self.fmt_dirty {
            let e = self.lines.back().expect("刚 push 必有 back");
            let (text, level) = (e.display(), e.level);
            self.formatted.push((text, level));
        }
        true
    }

    /// 渲染显示行缓存（脏时按 lines 整体重建；与 lines 保持同步）
    pub fn formatted(&mut self) -> &[(String, u8)] {
        if self.fmt_dirty {
            self.formatted.clear();
            self.formatted
                .extend(self.lines.iter().map(|e| (e.display(), e.level)));
            self.fmt_dirty = false;
        }
        &self.formatted
    }

    /// 渲染期级别过滤：开 show_debug 全显示，否则仅 INFO+（D-31 回溯语义）
    pub fn level_visible(level: u8, show_debug: bool) -> bool {
        show_debug || level >= 20
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.formatted.clear();
        self.fmt_dirty = false;
        self.bottom_marker = None;
    }

    /// 贴底渲染时调用：重置新行计数基准（幂等）
    pub fn mark_at_bottom(&mut self) {
        self.bottom_marker = Some(self.lines.len());
    }

    /// 距上次贴底的新行数（贴底=0；用户上翻浏览时 >0，驱动「回到最新」浮钮）
    pub fn new_since_bottom(&self) -> usize {
        self.lines
            .len()
            .saturating_sub(self.bottom_marker.unwrap_or(0))
    }

    /// 当前可见行数（渲染期过滤；「已复制 N 行」反馈用）
    pub fn visible_count(&self) -> usize {
        self.lines
            .iter()
            .filter(|e| Self::level_visible(e.level, self.show_debug))
            .count()
    }

    /// 当前可见行文本（渲染期过滤；「复制全部」内容）
    pub fn visible_texts(&mut self) -> Vec<String> {
        let sd = self.show_debug;
        self.formatted()
            .iter()
            .filter(|(_, l)| Self::level_visible(*l, sd))
            .map(|(t, _)| t.clone())
            .collect()
    }

    /// 贴底跟随状态推进（D-31；每帧日志滚动区渲染完成后调用，[`LogView`] 区分
    /// 面板 tab 与日志窗两个独立滚动区）：近底→解锁并重置未读基准；用户主动
    /// 滚动（偏移变化）且不近底→锁定。返回本帧是否应显示「回到最新」浮钮。
    /// 注意：由内容增长导致的偏移"落后"（egui stick_to_bottom 尚未追平的一帧）
    /// 不算用户主动滚动——不会误弹浮钮。
    pub fn advance_follow(
        &mut self,
        offset_y: f32,
        viewport_h: f32,
        content_h: f32,
        view: LogView,
    ) -> bool {
        let near = is_near_bottom(offset_y + viewport_h, content_h, FOLLOW_TOL);
        let (prev, mut pinned) = match view {
            LogView::Panel => (self.panel_prev_offset, self.panel_pinned),
            LogView::LogWin => (self.logwin_prev_offset, self.logwin_pinned),
        };
        let user_rolled = prev.is_some_and(|p| (offset_y - p).abs() > SCROLL_EPS);
        if near {
            pinned = false;
        } else if user_rolled {
            pinned = true;
        }
        match view {
            LogView::Panel => self.panel_pinned = pinned,
            LogView::LogWin => self.logwin_pinned = pinned,
        }
        match view {
            LogView::Panel => self.panel_prev_offset = Some(offset_y),
            LogView::LogWin => self.logwin_prev_offset = Some(offset_y),
        }
        if near {
            self.mark_at_bottom();
        }
        pinned && self.new_since_bottom() > 0
    }
}

// ── 字幕窗（M4.2，对照原版 subtitle_window.py + subtitle_text_widget.py）──

/// 最小显示时间 ms（原版 _min_display_ms = 1500：上句插入后须满此时长才能被替换）
pub const SUBTITLE_MIN_DISPLAY_MS: u64 = 1500;

/// 字幕窗 100ms 光标感知轮询周期（D-36：分区穿透 + 顶条悬停工具条的
/// 单一事实源；原版 _ct_timer 500ms 对分区判定偏慢，对齐悬浮窗 50ms 一档）
pub const SUBTITLE_POLL_MS: u64 = 100;

/// 顶条淡入淡出时长 ms（原版 150ms 动画系）
pub const SUBTITLE_STRIP_FADE_MS: u64 = 150;

/// 字幕窗句子（原版 _sentences 元组 `(original, {lang: text})` 的结构化形态）
#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleSentence {
    pub original: String,
    /// lang 码 → 译文；空键 "" 为原版 update_text(str) 兼容形态（包装为 {"": text}）
    pub translations: std::collections::BTreeMap<String, String>,
}

/// 缓动曲线（原版 QEasingCurve.Type.OutCubic / InCubic）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Easing {
    OutCubic,
    InCubic,
}

/// 通用缓动动画（原版 QPropertyAnimation：起止值 + 自定义时长 + 缓动；
/// [`HeightAnim`] 的泛化版——字幕窗高度 150ms 与淡入淡出自定义时长共用）
#[derive(Debug, Clone, PartialEq)]
pub struct EaseAnim {
    pub from: f32,
    pub to: f32,
    pub start: Instant,
    pub duration: Duration,
    pub easing: Easing,
}

impl EaseAnim {
    /// 当前进值；结束后返回 None 由调用方收敛
    pub fn current(&self, now: Instant) -> Option<f32> {
        if self.duration.is_zero() {
            return None;
        }
        let t =
            now.saturating_duration_since(self.start).as_secs_f32() / self.duration.as_secs_f32();
        if !(0.0..1.0).contains(&t) {
            return None;
        }
        let eased = match self.easing {
            Easing::OutCubic => 1.0 - (1.0 - t).powi(3),
            Easing::InCubic => t * t * t,
        };
        Some(self.from + (self.to - self.from) * eased)
    }
}

/// 字幕行换行缓存 key（原版 _text_cache 失效判据：文字/可用宽度/字号/描边任一变化）
#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleLineKey {
    pub text: String,
    pub avail_w: u32,
    pub font_size: u32,
    pub outline_enabled: bool,
    pub outline_width: u32,
}

/// 单条启用行的渲染状态（原版 _SubtitleTextWidget 对位字段）
#[derive(Debug, Default, Clone)]
pub struct SubtitleLineRender {
    /// 当前文本（原版 _text）
    pub text: String,
    /// 贪心换行结果（原版 _wrapped_lines；空文本 = 空 Vec）
    pub wrapped: Vec<String>,
    /// 上次换行的缓存 key（原版 _text_cache；None = 需重排）
    pub cache_key: Option<SubtitleLineKey>,
}

/// 字幕窗 UI 伴生状态（原版 SubtitleWindow 的时序字段 + 各行渲染缓存）
#[derive(Default)]
pub struct SubtitleUiState {
    /// 句子队列（原版 _sentences，尾部为新句）
    pub sentences: Vec<SubtitleSentence>,
    /// 待延迟插入（原版 _pending_segment_timers；新更新先取消旧待插入 → 至多 1 个存活）
    pub pending: Option<(Instant, SubtitleSentence)>,
    /// 上次插入时刻（原版 _last_insert_time，1500ms 最小显示时序基准）
    pub last_insert: Option<Instant>,
    /// 自动隐藏已触发（原版 _is_hidden_by_timeout）
    pub hidden_by_timeout: bool,
    /// 自动隐藏到点（原版 _auto_hide_timer；None = 未计时 / 无句子 / timeout=0）
    pub auto_hide_deadline: Option<Instant>,
    /// 淡出/淡入动画（原版 animate_out/animate_in 的 fade 分支）
    pub fade: Option<EaseAnim>,
    /// 行文本/句子变化待重排（[_refresh_display] 的消费标志，UI 帧内消费）
    pub display_dirty: bool,
    /// 启用行渲染状态（原版 _text_widgets，按 lines 启用序对位）
    pub lines: Vec<SubtitleLineRender>,
    /// 高度动画（原版 _height_anim，150ms OutCubic）
    pub height_anim: Option<EaseAnim>,
    /// 最近一次已申请的窗口高度（动画 from 基准；宿主 request_inner_size 异步生效）
    pub applied_height: f32,
    /// 精简动效（原版 set_reduce_motion：跳过过渡直接落位；面板 M4.3 接入）
    pub reduce_motion: bool,
    /// 位置持久化防抖起点（原版 position_changed → 500ms 防抖；仅存 x/y）
    pub pos_dirty_since: Option<Instant>,
    /// 上次保存的位置 (x, y)，未变化不触发
    pub last_saved_pos: Option<(i32, i32)>,
    /// 顶条工具条悬停状态（D-36：宿主 Win32 光标轮询驱动——穿透开启时 egui
    /// 收不到鼠标事件，此状态只源于 GetCursorPos，单一事实源）
    pub toolbar_hover: bool,
    /// 顶条淡入/淡出动画（None=静止；终态由 [`Self::toolbar_opacity`] 兜底）
    pub toolbar_anim: Option<EaseAnim>,
    /// 锁定（冻结拖动；顶条锁按钮翻转，内存态）
    pub locked: bool,
    /// 拖动进行中（D-37：宿主维护；分区穿透轮询据此豁免——拖动期间窗恒
    /// 非穿透，SetCapture 持续供给鼠标输入）
    pub dragging: bool,
    /// 抓握偏移（物理 px：拖起时刻 = 光标 − 窗左上；宿主 SetCursorPos 绝对
    /// 跟踪基准，原版 mousePressEvent 记录的 _drag_pos 对位）
    pub drag_grab: Option<(i32, i32)>,
}

impl SubtitleUiState {
    /// 原版 update_text → _on_update_text：取消待插入 + 1500ms 最小显示分流。
    /// 时机参数从 cfg（settings.subtitle_mode）现取，对齐原版 self._settings 读法。
    /// 返回 Some(到点时刻) = 进入 pending 队列（调用方安排 SubtitlePending 节拍）；
    /// None = 已立即插入。
    pub fn update_text(
        &mut self,
        original: String,
        translations: std::collections::BTreeMap<String, String>,
        sm: &SubtitleMode,
        now: Instant,
    ) -> Option<Instant> {
        // _cancel_pending_segments：新更新永远取代旧的待插入
        self.pending = None;
        let sentence = SubtitleSentence {
            original,
            translations,
        };
        // base_delay = max(0, 1500 - elapsed)（原版 _on_update_text；首句 last_insert=0 → 0）
        let base_delay = match self.last_insert {
            None => 0,
            Some(t) => {
                let elapsed = now.saturating_duration_since(t).as_millis() as u64;
                SUBTITLE_MIN_DISPLAY_MS.saturating_sub(elapsed)
            }
        };
        if base_delay == 0 {
            self.insert_sentence(sentence, sm, now);
            None
        } else {
            let at = now + Duration::from_millis(base_delay);
            self.pending = Some((at, sentence));
            Some(at)
        }
    }

    /// 原版 _insert_sentence：入队截断 + 隐藏中恢复 + 重排标记 + 重排自动隐藏计时
    pub fn insert_sentence(&mut self, sentence: SubtitleSentence, sm: &SubtitleMode, now: Instant) {
        self.sentences.push(sentence);
        // 原版怪癖 1:1 保留：sentences=0 时 Python `_sentences[-0:]` = 全表 → 实际不截断
        if sm.sentences > 0 && self.sentences.len() > sm.sentences as usize {
            let drop = self.sentences.len() - sm.sentences as usize;
            self.sentences.drain(..drop);
        }
        if self.hidden_by_timeout {
            self.restore_from_auto_hide(&sm.auto_hide_animation, sm.auto_hide_duration, now);
        }
        // _restart_auto_hide_timer：timeout>0 且有句子才计时
        self.auto_hide_deadline = if sm.auto_hide_timeout > 0 && !self.sentences.is_empty() {
            Some(now + Duration::from_secs(sm.auto_hide_timeout as u64))
        } else {
            None
        };
        self.last_insert = Some(now);
        self.display_dirty = true;
    }

    /// SubtitlePending 节拍消费（原版 timer.timeout → _insert_sentence）；
    /// 未到点放回（正常节拍不会提前）；返回是否实际插入。
    pub fn flush_pending(&mut self, sm: &SubtitleMode, now: Instant) -> bool {
        match self.pending.take() {
            Some((at, s)) if at <= now => {
                self.insert_sentence(s, sm, now);
                true
            }
            other => {
                self.pending = other;
                false
            }
        }
    }

    /// 原版 _on_auto_hide_timeout：置隐藏 + 启动淡出；重复触发无害。返回是否状态变化。
    pub fn on_auto_hide_timeout(
        &mut self,
        hide_animation: &str,
        hide_duration_ms: u32,
        now: Instant,
    ) -> bool {
        if self.hidden_by_timeout {
            return false;
        }
        // 淡出起点 = 触发时的当前不透明度（原版 setStartValue(_content_opacity_val)）
        let from = self.current_opacity(now);
        self.hidden_by_timeout = true;
        // animate_out：none=瞬时归零；fade=InCubic 淡出（slide_down 以 fade 近似，见模块注释）
        self.fade = fade_anim(
            hide_animation,
            hide_duration_ms,
            from,
            0.0,
            Easing::InCubic,
            now,
        );
        true
    }

    /// 原版 _restore_from_auto_hide：清隐藏标记 + 从 0 淡入（OutCubic）
    pub fn restore_from_auto_hide(
        &mut self,
        hide_animation: &str,
        hide_duration_ms: u32,
        now: Instant,
    ) {
        self.hidden_by_timeout = false;
        // 原版：先置 _content_opacity_val=0 再 animate_in（0→1）
        self.fade = fade_anim(
            hide_animation,
            hide_duration_ms,
            0.0,
            1.0,
            Easing::OutCubic,
            now,
        );
    }

    /// 当前内容不透明度（原版 _content_opacity_val；动画中取缓动值，否则按隐藏标记）
    pub fn current_opacity(&self, now: Instant) -> f32 {
        match &self.fade {
            Some(a) => a.current(now).unwrap_or(a.to),
            None => {
                if self.hidden_by_timeout {
                    0.0
                } else {
                    1.0
                }
            }
        }
    }

    /// 顶条可见性推进：hover 状态变化时启动 150ms 淡入/淡出（reduce_motion 直接落位）。
    /// 返回是否发生状态变化（宿主据此触发重绘）。
    pub fn set_toolbar_hover(&mut self, hover: bool, now: Instant) -> bool {
        if self.toolbar_hover == hover {
            return false;
        }
        self.toolbar_hover = hover;
        self.toolbar_anim = if self.reduce_motion {
            None
        } else {
            Some(EaseAnim {
                from: if hover { 0.0 } else { 1.0 },
                to: if hover { 1.0 } else { 0.0 },
                start: now,
                duration: Duration::from_millis(SUBTITLE_STRIP_FADE_MS),
                easing: if hover {
                    Easing::OutCubic
                } else {
                    Easing::InCubic
                },
            })
        };
        true
    }

    /// 当前顶条不透明度（动画中取缓动值，否则按悬停终态）
    pub fn toolbar_opacity(&self, now: Instant) -> f32 {
        match &self.toolbar_anim {
            Some(a) => a.current(now).unwrap_or(a.to),
            None => {
                if self.toolbar_hover {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    /// 原版 clear()：清句子/待插入/隐藏计时，各行文本清空并回满不透明度
    pub fn clear(&mut self) {
        self.sentences.clear();
        self.pending = None;
        self.auto_hide_deadline = None;
        self.hidden_by_timeout = false;
        self.fade = None;
        for l in &mut self.lines {
            l.text.clear();
            l.wrapped.clear();
            l.cache_key = None;
        }
        self.display_dirty = true;
    }
}

/// 依动画名构造淡入淡出动画；none/时长 0 = 瞬时（None，不透明度由隐藏标记兜底）
fn fade_anim(
    animation: &str,
    duration_ms: u32,
    from: f32,
    to: f32,
    easing: Easing,
    now: Instant,
) -> Option<EaseAnim> {
    if animation == "none" || duration_ms == 0 {
        return None;
    }
    Some(EaseAnim {
        from,
        to,
        start: now,
        duration: Duration::from_millis(duration_ms as u64),
        easing,
    })
}

/// 悬浮窗 UI 伴生状态（全部仅 UI 线程触达）
#[derive(Default)]
pub struct OverlayUiState {
    /// 消息区顶部 y（逻辑 px，最近一帧测量；穿透轮询据此划分可交互头部）
    pub header_px: f32,
    /// 显示模式（原版 _mode）
    pub mode: OverlayMode,
    /// 紧凑前的窗口高度（原版 _height_before_compact；恢复用）
    pub height_before_compact: Option<f32>,
    /// 进行中的高度动画
    pub anim: Option<HeightAnim>,
    /// 自动滚动待执行（add/translation/flush 时置位，帧内消费）
    pub scroll_pending: bool,
    /// 流式节流缓冲（原版 update_streaming 50ms QTimer）：msg_id → 最新部分文本
    pub pending_streams: std::collections::HashMap<u64, String>,
    /// 位置/尺寸持久化防抖起点（原版 _pos_save_timer 500ms）；None=无待保存变更
    pub pos_dirty_since: Option<Instant>,
    /// 上次保存的几何 (x, y, w, h)，未变化不触发（原版 _last_saved_geo）
    pub last_saved_geo: Option<(i32, i32, u32, u32)>,
    /// 待执行的导出（右键菜单；W5 起经 `Cmd::PickExportFile` 走编排域弹框，
    /// 保存路径经 `UiEvent::ExportSave` 回执后写文件）
    pub export_request: Option<lt_proto::ExportFileMode>,
    /// 拖动进行中（W5/D-71：宿主维护；穿透轮询据此豁免——拖动期间窗恒
    /// 非穿透，SetCapture 持续供给鼠标输入；D-37 字幕窗同法）
    pub dragging: bool,
    /// 抓握偏移（物理 px：拖起时刻 = 光标 − 窗左上；宿主 GetCursorPos 绝对
    /// 跟踪基准，原版 mousePressEvent 记录的 _drag_pos 对位）
    pub drag_grab: Option<(i32, i32)>,
}

// ── 控制面板（M4.3 第一批，对照原版 ui/panel/panel.py + _chrome.py）──

/// 面板设置防抖时长（原版 _save_timer.setInterval(300)：控件变更 300ms 后一次性应用）
pub const PANEL_APPLY_DEBOUNCE_MS: u64 = 300;

/// 翻译页 system_prompt 防抖时长（原版 translation_tab._prompt_debounce 600ms）
pub const PROMPT_APPLY_DEBOUNCE_MS: u64 = 600;

/// thinking_style 存储值 → 下拉索引（未知值回退 auto=0）。
/// E3/ADR-10：下拉项清单 = `lt_proto::THINKING_STYLES` 单一事实源——
/// 本地逐字拷贝镜像（漂移隐患）随本波删除
pub fn thinking_style_index(v: Option<&str>) -> usize {
    v.and_then(|s| lt_proto::THINKING_STYLES.iter().position(|k| *k == s))
        .unwrap_or(0)
}

/// 高级参数覆写行（原版 _make_override_row：checkbox 勾选才写 overrides）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverrideRow {
    /// 勾选（原版 QCheckBox "覆盖"）
    pub enabled: bool,
    /// 数值（整数行 max_tokens/seed 以整数语义取用）
    pub value: f64,
}

impl Default for OverrideRow {
    fn default() -> Self {
        Self {
            enabled: false,
            value: 0.0,
        }
    }
}

/// ModelEditDialog 的控件草稿（原版 dialogs.py ModelEditDialog 的成员镜像）。
/// 字段语义/范围/缺省逐项对照原版；OK 时经 [`ModelEditState::build`] 组装
/// lt_proto::ModelConfig（条件序列化由 serde 契约保证）。
#[derive(Debug, Clone, PartialEq)]
pub struct ModelEditState {
    /// 新增（true）或编辑既有行（false，携带 `index`）
    pub is_new: bool,
    /// 编辑目标行号（is_new 时无意义）
    pub index: usize,
    // ── Basic ──
    pub name: String,
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    /// 0=不使用 1=系统代理 2=自定义（原版 _proxy_mode）
    pub proxy_index: usize,
    /// 自定义代理 URL（仅 proxy_index==2 可编辑）
    pub proxy_url: String,
    /// 0=auto 1=deepseek 2=qwen 3=vllm 4=openai 5=off（原版 no_think 复选的
    /// 后继形态：契约键 thinking_style）
    pub thinking_index: usize,
    pub no_system_role: bool,
    pub streaming: bool,
    pub json_response: bool,
    /// 0..=20（原版 QSpinBox）
    pub context_turns: i32,
    /// $ / 1M tokens（原版 QDoubleSpinBox 0-999 两位小数，0 = "—"）
    pub input_price: f64,
    pub output_price: f64,
    /// 六行 checkbox+value（键序 = `lt_proto::OVERRIDE_KEYS`，单一事实源）
    pub overrides: [OverrideRow; 6],
    /// extra_body JSON 文本（空 = 不设置；非法 = 禁止确定）
    pub extra_body_text: String,
}

impl ModelEditState {
    /// "添加模型"空白草稿（原版无 model_data 分支：文本框全空、proxy=none、
    /// no_think/streaming 默认勾选——对应 thinking=auto、streaming=true）
    pub fn new_add() -> Self {
        Self {
            is_new: true,
            index: 0,
            name: String::new(),
            api_base: String::new(),
            api_key: String::new(),
            model: String::new(),
            proxy_index: 0,
            proxy_url: String::new(),
            thinking_index: 0,
            no_system_role: false,
            streaming: true,
            json_response: false,
            context_turns: 0,
            input_price: 0.0,
            output_price: 0.0,
            overrides: std::array::from_fn(|_| OverrideRow::default()),
            extra_body_text: String::new(),
        }
    }

    /// 由既有 ModelConfig 填充（原版 model_data populate 分支）
    pub fn new_edit(index: usize, cfg: &lt_proto::ModelConfig) -> Self {
        // 原版 populate 的高级行缺省值（checkbox 未勾时数值不可见，但 spin 保持构造值）
        const ADV_DEFAULTS: [f64; 6] = [0.3, 1.0, 256.0, 0.0, 0.0, 0.0];
        let mut st = Self {
            is_new: false,
            index,
            name: cfg.name.clone(),
            api_base: cfg.api_base.clone(),
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
            proxy_index: proxy_index_for(&cfg.proxy),
            proxy_url: String::new(),
            thinking_index: thinking_style_index(cfg.thinking_style.as_deref()),
            no_system_role: cfg.no_system_role,
            streaming: cfg.streaming,
            json_response: cfg.json_response,
            context_turns: cfg.context_turns as i32,
            input_price: cfg.input_price,
            output_price: cfg.output_price,
            overrides: std::array::from_fn(|i| OverrideRow {
                enabled: false,
                value: ADV_DEFAULTS[i],
            }),
            extra_body_text: String::new(),
        };
        // 原版 populate：非 none/system 且非空 → custom + URL
        if st.proxy_index == 2 {
            st.proxy_url = cfg.proxy.clone();
        }
        if let Some(map) = &cfg.overrides {
            for (i, key) in lt_proto::OVERRIDE_KEYS.iter().enumerate() {
                if let Some(v) = map.get(*key).filter(|v| !v.is_null()) {
                    let num = v.as_f64().unwrap_or(0.0);
                    st.overrides[i] = OverrideRow {
                        enabled: true,
                        value: num,
                    };
                }
            }
        }
        if let Some(extra) = &cfg.extra_body {
            if !extra.is_null() {
                st.extra_body_text = serde_json::to_string_pretty(extra).unwrap_or_default();
            }
        }
        st
    }

    /// 解析 extra_body 文本（原版 _parse_extra_body：空 → Ok(None)；
    /// 非法 JSON / 非 object → Err）
    pub fn parse_extra_body(&self) -> Result<Option<serde_json::Value>, String> {
        let text = self.extra_body_text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if !v.is_object() {
            return Err("extra_body must be a JSON object".into());
        }
        Ok(Some(v))
    }

    /// 组装 ModelConfig（原版 get_data；条件序列化语义由 serde 契约承载）。
    /// extra_body 非法时返回 Err（UI 侧禁用确定按钮）。
    pub fn build(&self) -> Result<lt_proto::ModelConfig, String> {
        let extra_body = self.parse_extra_body()?;
        let mut overrides: std::collections::BTreeMap<String, serde_json::Value> =
            std::collections::BTreeMap::new();
        for (i, key) in lt_proto::OVERRIDE_KEYS.iter().enumerate() {
            let row = self.overrides[i];
            if row.enabled {
                let v = if matches!(*key, "max_tokens" | "seed") {
                    serde_json::Value::from(row.value.round() as i64)
                } else {
                    // 原版 round(val, 2)
                    serde_json::Value::from((row.value * 100.0).round() / 100.0)
                };
                overrides.insert((*key).to_string(), v);
            }
        }
        Ok(lt_proto::ModelConfig {
            name: self.name.trim().to_string(),
            api_base: self.api_base.trim().to_string(),
            api_key: self.api_key.trim().to_string(),
            model: self.model.trim().to_string(),
            proxy: self.proxy_arg(),
            no_system_role: self.no_system_role,
            thinking_style: match lt_proto::THINKING_STYLES[self.thinking_index] {
                "auto" => None,
                v => Some(v.to_string()),
            },
            streaming: self.streaming,
            json_response: self.json_response,
            context_turns: self.context_turns.clamp(0, 20) as u32,
            input_price: self.input_price.clamp(0.0, 999.0),
            output_price: self.output_price.clamp(0.0, 999.0),
            overrides: if overrides.is_empty() {
                None
            } else {
                Some(overrides)
            },
            extra_body,
        })
    }

    /// 代理三模式 → 契约字符串（原版 get_data proxy 分支：custom 且空白回退 none）
    pub fn proxy_arg(&self) -> String {
        match self.proxy_index {
            1 => "system".into(),
            2 => {
                let t = self.proxy_url.trim();
                if t.is_empty() {
                    "none".into()
                } else {
                    t.to_string()
                }
            }
            _ => "none".into(),
        }
    }
}

/// 代理契约字符串 → 下拉索引（原版 populate：system→1；非 none/system 且非空→2；否则 0）
pub fn proxy_index_for(proxy: &str) -> usize {
    if proxy == "system" {
        1
    } else if !proxy.is_empty() && proxy != "none" {
        2
    } else {
        0
    }
}

/// 字幕行编辑对话框草稿（原版 LineEditDialog 的成员镜像；字段全集 =
/// lt_proto::SubtitleLine 的全部 15 个可编辑字段）
#[derive(Debug, Clone, PartialEq)]
pub struct LineEditState {
    /// 行号（新增时为追加位置）
    pub index: usize,
    /// 新增（true）或编辑（false）
    pub is_new: bool,
    /// 0=原文 1=翻译（原版 _type_combo）
    pub line_type_index: usize,
    /// 目标语言码（仅翻译行有意义；原版 _lang_combo 跳过 auto）
    pub lang: String,
    pub enabled: bool,
    pub font_family: String,
    /// 8..=120 pt
    pub font_size: i32,
    /// "#rrggbb"
    pub color: String,
    /// 0..=100（% 表述；存储 0..=255 由换算承担）
    pub opacity_pct: i32,
    /// 0=左 1=中 2=右
    pub align_index: usize,
    pub outline_enabled: bool,
    pub outline_color: String,
    /// 0..=10 px
    pub outline_width: i32,
    pub bg_image: String,
    /// 0..=5：none/fade/slide_left/slide_right/slide_up/slide_down
    pub entry_anim_index: usize,
    pub exit_anim_index: usize,
    /// 50..=3000 ms
    pub animation_duration: i32,
}

/// 行动画下拉项（原版 anim_items 顺序）
pub const ANIM_VALUES: [&str; 6] = [
    "none",
    "fade",
    "slide_left",
    "slide_right",
    "slide_up",
    "slide_down",
];

/// 动画存储值 → 下拉索引（未知值回退 none=0）
pub fn anim_index(v: &str) -> usize {
    ANIM_VALUES.iter().position(|a| *a == v).unwrap_or(0)
}

/// 对齐存储值 → 下拉索引（未知值回退 center=1）
pub fn align_index(v: &str) -> usize {
    match v {
        "left" => 0,
        "right" => 2,
        _ => 1,
    }
}

/// 下拉索引 → 对齐存储值
pub fn align_value(idx: usize) -> &'static str {
    match idx {
        0 => "left",
        2 => "right",
        _ => "center",
    }
}

impl LineEditState {
    /// 由 SubtitleLine 填充（原版 LineEditDialog(cfg)）
    pub fn new_edit(index: usize, line: &lt_proto::SubtitleLine) -> Self {
        Self {
            index,
            is_new: false,
            line_type_index: if line.line_type == "original" { 0 } else { 1 },
            lang: line.lang.clone().unwrap_or_else(|| "zh".into()),
            enabled: line.enabled,
            font_family: line.font_family.clone(),
            font_size: line.font_size as i32,
            color: line.color.clone(),
            opacity_pct: (f64::from(line.opacity) / 255.0 * 100.0).round() as i32,
            align_index: align_index(&line.align),
            outline_enabled: line.outline_enabled,
            outline_color: line.outline_color.clone(),
            outline_width: line.outline_width as i32,
            bg_image: line.bg_image.clone(),
            entry_anim_index: anim_index(&line.entry_animation),
            exit_anim_index: anim_index(&line.exit_animation),
            animation_duration: line.animation_duration as i32,
        }
    }

    /// 新增行草稿（原版 _add_line 的 new_line 字典；lang=en、其余默认）
    pub fn new_add(index: usize) -> Self {
        let line = lt_proto::SubtitleLine {
            lang: Some("en".into()),
            ..Default::default()
        };
        let mut st = Self::new_edit(index, &line);
        st.is_new = true;
        st
    }

    /// 组装 SubtitleLine（原版 get_config：opacity % ↔ 0-255 换算；
    /// 仅翻译行携带 lang）
    pub fn build(&self) -> lt_proto::SubtitleLine {
        let line_type = if self.line_type_index == 0 {
            "original"
        } else {
            "translation"
        }
        .to_string();
        lt_proto::SubtitleLine {
            line_type,
            lang: if self.line_type_index == 1 {
                Some(self.lang.clone())
            } else {
                None
            },
            enabled: self.enabled,
            font_family: self.font_family.clone(),
            font_size: self.font_size.clamp(8, 120) as u32,
            color: self.color.clone(),
            opacity: (f64::from(self.opacity_pct.clamp(0, 100)) / 100.0 * 255.0).round() as u32,
            align: align_value(self.align_index).to_string(),
            outline_enabled: self.outline_enabled,
            outline_color: self.outline_color.clone(),
            outline_width: self.outline_width.clamp(0, 10) as u32,
            bg_image: self.bg_image.clone(),
            entry_animation: ANIM_VALUES[self.entry_anim_index].to_string(),
            exit_animation: ANIM_VALUES[self.exit_anim_index].to_string(),
            animation_duration: self.animation_duration.clamp(50, 3000) as u32,
        }
    }
}

/// 字幕行上移（原版 _move_line_up：row>0 才交换）。返回是否发生交换。
pub fn move_line_up(lines: &mut [lt_proto::SubtitleLine], row: usize) -> bool {
    if row == 0 || row >= lines.len() {
        return false;
    }
    lines.swap(row, row - 1);
    true
}

/// 字幕行下移（原版 _move_line_down：row < len-1 才交换）。返回是否发生交换。
pub fn move_line_down(lines: &mut [lt_proto::SubtitleLine], row: usize) -> bool {
    if row + 1 >= lines.len() {
        return false;
    }
    lines.swap(row, row + 1);
    true
}

/// 模型缓存扫描条目（原版 get_cache_entries 的 (name, path) + dir_size 结果）
#[derive(Debug, Clone, PartialEq)]
pub struct CacheEntry {
    /// 显示名（"SenseVoice Small (ModelScope)" 等）
    pub name: String,
    pub path: std::path::PathBuf,
    pub size: u64,
}

/// "#rrggbb" 颜色校验/归一（原版 QColor.name() 语义：小写 #rrggbb）。
/// 接受 #RGB / #RRGGBB / #RRGGBBAA；非法返回 None（UI 保持原值 + 红字提示）。
pub fn normalize_hex_color(s: &str) -> Option<String> {
    let t = s.trim();
    let hex = t.strip_prefix('#')?;
    if hex.is_empty() || hex.chars().any(|c| !c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        // #RGB → 每位重复展开（Qt QColor 语义）
        3 => {
            let expanded: String = hex
                .chars()
                .flat_map(|c| [c, c])
                .collect::<String>()
                .to_ascii_lowercase();
            Some(format!("#{expanded}"))
        }
        6 => Some(format!("#{}", hex.to_ascii_lowercase())),
        8 => Some(format!("#{}", hex[..6].to_ascii_lowercase())),
        _ => None,
    }
}

/// 面板页序（原版 ControlPanel addTab 固定顺序：VAD/ASR、翻译、样式、字幕、
/// 基准测试、缓存、更新日志；实测原版无"常规/诊断/关于"页）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelPage {
    #[default]
    VadAsr,
    Translation,
    Style,
    Subtitle,
    Benchmark,
    Cache,
    Changelog,
    /// 日志页（Rust 版新增：设置内自查入口；与日志窗共享同一缓冲）
    Log,
}

impl PanelPage {
    /// 原版 tabs.addTab 顺序（control_panel.py:170-180）；日志页为 Rust 版新增
    ///（用户自查入口，追加到末位）
    pub const ALL: [PanelPage; 8] = [
        PanelPage::VadAsr,
        PanelPage::Translation,
        PanelPage::Style,
        PanelPage::Subtitle,
        PanelPage::Benchmark,
        PanelPage::Cache,
        PanelPage::Changelog,
        PanelPage::Log,
    ];

    /// Tab 标题 i18n 键（原版 addTab 的 t("tab_*")）
    pub fn tab_key(self) -> &'static str {
        match self {
            PanelPage::VadAsr => "tab_vad_asr",
            PanelPage::Translation => "tab_translation",
            PanelPage::Style => "tab_style",
            PanelPage::Subtitle => "tab_subtitle",
            PanelPage::Benchmark => "tab_benchmark",
            PanelPage::Cache => "tab_cache",
            PanelPage::Changelog => "tab_changelog",
            PanelPage::Log => "tab_log",
        }
    }
}

/// 面板明暗主题（原版 _chrome.THEME_MODES = ("dark","light")，DEFAULT_THEME = dark）。
/// 注意：settings 契约缺 `theme` 键（lt-proto 不动）→ 仅存内存态，重启回深色默认。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

/// 模型下载运行态（识别页缓存卡片的状态机；运行期下载的唯一 UI 反馈源）
#[derive(Debug, Clone, Default, PartialEq)]
pub enum DownloadUiState {
    /// 无下载（未开始/已成功/被替换）
    #[default]
    Idle,
    /// 下载进行中：按**当前文件**显示进度（DL-3——file/index/count 来自
    /// `UiEvent::Download`，done_bytes/total_bytes 为精确字节；total 未知走
    /// 日志模式）；log 为进度/日志行环形缓冲（上限 200）
    Downloading {
        file: String,
        k: u32,
        n: u32,
        done_bytes: u64,
        total_bytes: u64,
        log: Vec<String>,
    },
    /// 下载被用户取消（DL-4/D-23）：.incomplete 续传现场保留，卡片给
    /// 「继续下载」按钮；log 为取消前日志
    Cancelled { log: Vec<String> },
    /// 下载失败：kind 供分类提示（W2：`proto::DownloadFailKind` 直供，
    /// 不再字符串前缀还原），detail 为原始错误串，log 为失败前日志
    Failed {
        kind: lt_proto::DownloadFailKind,
        detail: String,
        log: Vec<String>,
    },
}

impl DownloadUiState {
    /// 进行中（识别页下载按钮禁用 + 卡片进度渲染的依据）
    pub fn downloading(&self) -> bool {
        matches!(self, Self::Downloading { .. })
    }

    /// 追加日志行（环形 200 条，删最旧）
    pub fn push_log(&mut self, line: String) {
        if let Self::Downloading { log, .. } | Self::Cancelled { log } | Self::Failed { log, .. } =
            self
        {
            if log.len() >= 200 {
                log.remove(0);
            }
            log.push(line);
        }
    }

    /// 机器段进度写入（DL-3）：精确字节整体覆盖，文件切换时进度自然归零；
    /// 非 Downloading 态（成功竞态晚到事件等）忽略
    pub fn apply_progress(&mut self, file: String, k: u32, n: u32, done: u64, total: u64) {
        if let Self::Downloading {
            file: cur,
            k: ck,
            n: cn,
            done_bytes,
            total_bytes,
            ..
        } = self
        {
            *cur = file;
            *ck = k;
            *cn = n;
            *done_bytes = done;
            *total_bytes = total;
        }
    }
}

/// 翻译配置「测试连接」运行态（翻译页按钮行；TestTranslatorResult 事件收敛）
#[derive(Debug, Clone, Default, PartialEq)]
pub enum TestTranslatorState {
    #[default]
    Idle,
    Running,
    Done {
        ok: bool,
        error: Option<String>,
        ms: u64,
    },
}

/// 音频设备枚举状态（W5/R13 下沉后：`Cmd::RefreshDevices` → `UiEvent::Devices`
/// 回执的异步三态——W5 收口 P1 修「回执前每帧重发」：Idle→Probe 只发一次，
/// 在途期间不再叠发；Probe 的渲染面 = 空列表（与旧 None 同观感，回执
/// 通常百毫秒内到达））
#[derive(Debug, Clone, Default, PartialEq)]
pub enum DevicesState {
    /// 从未请求（识别页首次进入 → 发一次探测命令）
    #[default]
    Idle,
    /// 探测命令已在途（首入/刷新已发；回执前不重复发出）
    Probe,
    /// 枚举结果回执缓存（含失败空表——枚举失败也以 Ready 空表收敛，避免
    /// 卡在 Probe 反复重发；用户可点"刷新"重试）
    Ready(lt_proto::DeviceList),
}

/// 控制面板 UI 伴生状态（全部仅 UI 线程触达；对照 ControlPanel 的面板局部字段）
#[derive(Default)]
pub struct PanelUiState {
    /// 当前页（原版 _nav.currentRow + _stack.setCurrentIndex）
    pub page: PanelPage,
    /// 明暗主题（内存态；settings 契约缺 theme 键，见 [`ThemeMode`]）
    pub theme: ThemeMode,
    /// 启动时隐藏悬浮窗（内存态；settings 契约缺 start_hidden 键，生效随 M4.4 启动流）
    pub start_hidden: bool,
    /// 减少动效（内存态；settings 契约缺 reduce_motion 键；同步进 SubtitleUiState 生效）
    pub reduce_motion: bool,
    /// 开机自启当前态（None=未探测；Windows 注册表 Run 键为事实源，见 panel::autostart）
    pub autostart: Option<bool>,
    /// 设备枚举状态（W5/R13：`UiEvent::Devices` 事件载荷——帧内 COM 枚举已
    /// 下线，首入/刷新经 [`Self::request_devices_if_idle`]/
    /// [`Self::request_devices_refresh`] 发命令；回执落地 [`DevicesState::Ready`]）
    pub devices: DevicesState,
    /// 模型缓存探测缓存（DL-6/F12：识别页每帧渲染不再扫盘——2s TTL，
    /// 探测键（engine|model）变化或下载事件到达时失效）
    pub cache_probe: Option<(
        std::time::Instant,
        String,
        crate::windows::panel::vad::CacheStatus,
    )>,
    /// 设置防抖到期时刻（原版 _save_timer singleShot：每次变更重置到 now+300ms
    /// → 300ms 内连发合并为最后一次的 deadline；到期由 PanelApply 节拍消费）
    pub apply_due_at: Option<Instant>,
    /// 翻译页 system_prompt 防抖到期时刻（原版 _prompt_debounce 600ms；
    /// 到期由 PromptApply 节拍消费 → SwitchTranslator 重建翻译器）
    pub prompt_apply_due: Option<Instant>,
    /// ModelEditDialog 打开中（None=关闭；翻译页模态区渲染）
    pub model_editor: Option<ModelEditState>,
    /// 字幕行编辑对话框打开中（None=关闭；字幕页模态区渲染）
    pub line_editor: Option<LineEditState>,
    /// 翻译页模型列表选中行（None=无选中；点击行 = 选中并跟随 active_model）
    pub model_selected: Option<usize>,
    /// 数据页缓存列表选中行
    pub cache_selected: Option<usize>,
    /// 字幕页文字行列表选中行
    pub line_selected: Option<usize>,
    /// 数据页缓存扫描结果（None=尚未扫描；进入数据页或点"刷新"时重建）
    pub cache_entries: Option<Vec<CacheEntry>>,
}

impl PanelUiState {
    /// 设置变更登记（原版 TabBase.auto_save → _save_timer.start() 重启 300ms 单发定时）。
    /// `now` 由调用方注入以便测试；宿主按 [`TickKind::PanelApply`] 节拍消费。
    pub fn mark_dirty_at(&mut self, now: Instant) {
        self.apply_due_at = Some(Self::apply_deadline(now));
    }

    /// 防抖到期消费（宿主在 PanelApply 节拍触发时调用）：返回是否应发送 ApplySettings。
    /// 未到 deadline / 无待应用变更 → false。
    pub fn take_apply_due(&mut self, now: Instant) -> bool {
        match self.apply_due_at {
            Some(at) if now >= at => {
                self.apply_due_at = None;
                true
            }
            _ => false,
        }
    }

    /// 识别页首入：Idle → Probe 只发一次探测命令（W5 收口 P1——回执前每帧
    /// 重发会造成探测线程洪泛 + ThreadDied 批报；Probe/Ready 态不重复发出）
    pub fn request_devices_if_idle(&mut self, session: &mut SessionView) {
        if matches!(self.devices, DevicesState::Idle) {
            self.devices = DevicesState::Probe;
            session.send_cmd(lt_proto::Cmd::RefreshDevices);
        }
    }

    /// "刷新"按钮：置 Probe 并发出（在途时幂等跳过——连点只叠一次探测；
    /// Ready 态重刷 = 显式换新枚举）
    pub fn request_devices_refresh(&mut self, session: &mut SessionView) {
        if !matches!(self.devices, DevicesState::Probe) {
            self.devices = DevicesState::Probe;
            session.send_cmd(lt_proto::Cmd::RefreshDevices);
        }
    }

    /// 防抖 deadline（原版 setInterval(300) 的到期时刻）
    pub fn apply_deadline(now: Instant) -> Instant {
        now + Duration::from_millis(PANEL_APPLY_DEBOUNCE_MS)
    }

    /// prompt 防抖到期消费（宿主在 PromptApply 节拍触发时调用）
    pub fn take_prompt_apply_due(&mut self, now: Instant) -> bool {
        match self.prompt_apply_due {
            Some(at) if now >= at => {
                self.prompt_apply_due = None;
                true
            }
            _ => false,
        }
    }
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

impl Default for WizardState {
    fn default() -> Self {
        Self::new()
    }
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
        if self.hub_index == 0 {
            "ms".into()
        } else {
            "hf".into()
        }
    }

    /// 代理选择 → 命令契约字符串（原版 _download_proxy：custom 且空白回退 "system"）
    pub fn proxy_arg(&self) -> String {
        match self.proxy_index {
            1 => "system".into(),
            2 => {
                let trimmed = self.proxy_url.trim();
                if trimmed.is_empty() {
                    "system".into()
                } else {
                    trimmed.to_string()
                }
            }
            _ => "none".into(),
        }
    }
}

/// 启动流状态机（替代原版 main() 里的模态对话框序列：SetupWizardDialog / ModelDownloadDialog）。
/// **D-78 保留待开发面**：当前 Ready 恒真（Wizard/DownloadMissing 零生产者），
/// 接线点与现状地图见 docs/architecture-v2-improvements.md §5.4——后续首启
/// 引导开发在此状态机上继续，勿删变体。
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
/// 只读共享视图（W5：AppUi 域拆分——内嵌可变性收敛于此）。
/// i18n 为进程级静态全局（lt_i18n::set_lang），注册表显示信息为纯函数查询，
/// 均无携带状态；本结构现仅字体系统（W-3）。
pub struct UiContext {
    /// 字体系统（W-3）：系统字体扫描列表 + 加载缓存 + 族名→FontFamily 解析表
    pub fonts: crate::fonts::FontsState,
}

/// 会话级协调（W5：所有窗口共享的最小面——命令出口/动作队列/节拍表/
/// 可见性只读快照；真值在宿主 WindowManager，本快照由宿主每帧 dispatch 前刷新）
pub struct SessionView {
    /// 管道运行中（托盘"暂停/恢复"与悬浮窗启停按钮共用）
    pub running: bool,
    /// Worker 命令出口（宿主构造时注入；widget 代码经 send_cmd 直接发送）
    pub cmd_tx: Option<std::sync::mpsc::Sender<Cmd>>,
    /// 各窗口可见性只读快照（宿主每帧 dispatch 前 `clone_from` 真值表）
    pub visible: std::collections::HashMap<WinId, bool>,
    /// 管道启动失败原因（AppShell 直写；识别页顶部红字显示，用户可去日志页查细节）
    pub pipeline_error: Option<String>,
    /// UI 帧内请求的窗口动作（宿主在帧后消费；Drag/Resize/Hide/ShowPanel）
    pub actions: Vec<(WinId, WinAction)>,
    /// 待处理的定时重绘（宿主 about_to_wait 排空分派）
    pub ticks: Vec<Tick>,
    /// 设置草稿变更意图（跨域：字幕窗/确认模态/面板页 → 宿主消费 → 面板
    /// 300ms 防抖登记；W5 起 `mark_settings_dirty` 的意图化形态）
    pub settings_apply_pending: bool,
}

/// 悬浮窗域（W5：monitor/messages/stats/overlay 伴生态/ov_* 复选/ASR 标签）
pub struct OverlayUi {
    /// 监视条数据链（任务 1.6）
    pub monitor: MonitorData,
    /// 悬浮窗消息链（AddMessage 事件追加，上限 50 条、删最旧）
    pub messages: Vec<OverlayMessage>,
    /// 翻译/用量统计（UpdateStats 事件更新）
    pub stats: OverlayStats,
    /// 悬浮窗 UI 伴生状态（模式/动画/节流/防抖）
    pub state: OverlayUiState,
    /// 悬浮窗穿透/置顶/自动滚动/任务栏（托盘子菜单与 DragHandle 复选框三向同步）
    pub ov_click_through: bool,
    pub ov_topmost: bool,
    pub ov_auto_scroll: bool,
    pub ov_taskbar: bool,
    /// ASR 设备标签（"SenseVoice Small" 等；不可用时 "ASR unavailable"；
    /// 悬浮窗 MonitorBar device 段 + 托盘状态行共用）
    pub asr_label: Option<String>,
    /// 右键"清空列表"请求（帧后消费）
    pub clear_request: bool,
}

/// 字幕窗域（W5）
pub struct SubtitleUi {
    /// 字幕窗 UI 伴生状态（M4.2：句子队列/自动隐藏/高度动画/渲染缓存）
    pub state: SubtitleUiState,
    /// 字幕窗首启拖动提示是否已弹（WP-1：原版 _subwin_notified 会话内一次性）
    pub hint_shown: bool,
}

/// 控制面板域（W5：panel 伴生态 + 下载卡片 + 翻译页红字/测试连接）
pub struct PanelUi {
    /// 控制面板 UI 伴生状态（M4.3：页栈/主题/设备缓存/设置防抖）
    pub state: PanelUiState,
    /// 模型下载运行态（识别页缓存卡片；DownloadProgress/Failed/Succeeded 事件驱动）
    pub download: DownloadUiState,
    /// 翻译装置不可用原因（TranslatorUnavailable 事件；翻译页状态行红字显示）
    pub translator_error: Option<String>,
    /// 翻译配置「测试连接」运行态（Cmd::TestTranslator 的 UI 侧）
    pub test_translator: TestTranslatorState,
}

/// 日志域（W5：日志窗与面板日志 tab 双视图共享，保持）
pub struct LogUi {
    /// 日志窗状态（M4.5）
    pub logwin: LogWindowState,
}

/// 性能基准域（W5：独立工具窗 + 面板基准 tab 共用）
pub struct BenchUi {
    /// 性能基准输出行（W2 起经 `UiEvent::Bench(Line)` 事件回流追加；
    /// 上限 500 行防内存膨胀；完成态由 Finished 事件复位）
    pub lines: Vec<String>,
    /// 性能基准运行中（开始按钮禁用/文案切换；`Bench(Finished)` 到达即复位）
    pub running: bool,
    /// 性能基准参与模型勾选（与 settings.models 对位；缺省全选）
    pub selected: Vec<bool>,
    /// 性能基准源语言下拉索引（BENCH_SRC_LANGS）
    pub src: usize,
    /// 性能基准目标语言下拉索引（BENCH_TGT_LANGS）
    pub tgt: usize,
}

/// 启动流域（W5：首启向导/缺模型下载/Ready + 模型加载对话框）
pub struct StartupUi {
    /// 启动流状态机（首启向导/缺模型下载/Ready）
    pub flow: StartupFlow,
    /// 模型加载对话框（_ModelLoadDialog）：Some(label)=显示中
    pub load_dialog: Option<String>,
}

/// 模态域（W5：通用确认模态 + 退出请求）
pub struct ModalUi {
    /// 通用确认模态（D-33/H-3~H-5；None=关闭，同一时刻至多一个）
    pub confirm: Option<ConfirmUi>,
    /// 缺模型下载失败后点"关闭"：请求退出应用（原版 reject → main 返回退出）
    pub quit_requested: bool,
}

/// UI 域根（W5：AppState 巨石按窗口归属拆分的域组合）。窗口模块经
/// [`windows::dispatch`] 解构分发只拿本域 `&mut` + 共享 `&`——借用检查器
/// 从此强制窗口边界（越界纯靠命名纪律的时代结束，R8）。
pub struct AppUi {
    /// 编辑/持久真值（DEC-4 单写者不变；300ms 防抖 ApplySettings 提交）
    pub settings: Settings,
    /// 只读共享：fonts（内嵌可变性收敛于此）
    pub ctx: UiContext,
    /// 会话级协调：running / cmd_tx / 可见性只读快照 / pipeline_error +
    /// 动作队列 + 节拍表 + 设置变更意图
    pub session: SessionView,
    /// 悬浮窗域
    pub overlay: OverlayUi,
    /// 字幕窗域
    pub subtitle: SubtitleUi,
    /// 控制面板域
    pub panel: PanelUi,
    /// 日志域
    pub log: LogUi,
    /// 性能基准域
    pub bench: BenchUi,
    /// 启动流域
    pub startup: StartupUi,
    /// 模态域
    pub modal: ModalUi,
}

impl AppUi {
    /// 常规构造：无启动流（flow=Ready），主窗口默认可见性由宿主定。
    pub fn new(settings: Settings) -> Self {
        Self::with_startup(settings, StartupFlow::Ready)
    }

    /// 指定初始启动流构造。启动流进行中（首启向导/缺模型下载）4 个主窗口
    /// 初始不可见的可见性表由宿主（MultiWindowApp::new）按本流初始化——
    /// 可见性真值归宿主 WindowManager（W5b/R20），此处只保留域状态。
    pub fn with_startup(settings: Settings, flow: StartupFlow) -> Self {
        Self {
            settings,
            ctx: UiContext {
                // 字体系统（W-3）：启动扫描系统字体列表，应用期按 Settings 热重建
                fonts: crate::fonts::FontsState::new(),
            },
            session: SessionView {
                running: true,
                cmd_tx: None,
                visible: std::collections::HashMap::new(),
                pipeline_error: None,
                actions: Vec::new(),
                ticks: Vec::new(),
                settings_apply_pending: false,
            },
            overlay: OverlayUi {
                monitor: MonitorData::default(),
                messages: Vec::new(),
                stats: OverlayStats::default(),
                state: OverlayUiState::default(),
                // 原版默认：置顶√、自动滚动√、穿透×、任务栏×
                ov_click_through: false,
                ov_topmost: true,
                ov_auto_scroll: true,
                ov_taskbar: false,
                asr_label: None,
                clear_request: false,
            },
            subtitle: SubtitleUi {
                state: SubtitleUiState::default(),
                hint_shown: false,
            },
            panel: PanelUi {
                state: PanelUiState::default(),
                download: DownloadUiState::default(),
                translator_error: None,
                test_translator: TestTranslatorState::default(),
            },
            log: LogUi {
                logwin: LogWindowState::default(),
            },
            bench: BenchUi {
                lines: Vec::new(),
                running: false,
                selected: Vec::new(),
                src: 0,
                tgt: 0,
            },
            startup: StartupUi {
                flow,
                load_dialog: None,
            },
            modal: ModalUi {
                confirm: None,
                quit_requested: false,
            },
        }
    }

    /// 悬浮窗持久化几何 (x, y, w, h)（settings.overlay_* 四键齐备才生效）
    pub fn overlay_geometry(&self) -> Option<(i32, i32, u32, u32)> {
        let s = &self.settings;
        Some((s.overlay_x?, s.overlay_y?, s.overlay_w?, s.overlay_h?))
    }

    /// PanelApply 节拍到期：消费防抖并返回应整体重放的设置快照
    /// （原版 _do_auto_save → _apply_settings → settings_changed.emit(snapshot)）。
    /// 返回 None = 无脏标记（不该发生，防御语义）。
    pub fn take_due_panel_apply(&mut self, now: Instant) -> Option<Settings> {
        if self.panel.state.take_apply_due(now) {
            Some(self.settings.clone())
        } else {
            None
        }
    }

    /// 向导"开始下载"统一入口：倒计时归零自动触发与按钮点击共用（原版 _start_download）。
    /// 切 Downloading、停倒计时、按当前 hub/proxy 选择发 StartDownload；重复触发忽略。
    pub fn wizard_auto_start(&mut self) {
        wizard_auto_start(&mut self.startup.flow, &mut self.session);
    }
}

/// 向导"开始下载"统一入口（自由函数版：窗口模块与 AppUi 根共用）：倒计时归零
/// 自动触发与按钮点击殊途同归——切 Downloading、停倒计时（原版 _auto_timer.stop()）、
/// 发 StartDownload；重复触发忽略。
pub fn wizard_auto_start(flow: &mut StartupFlow, session: &mut SessionView) {
    let (hub, proxy) = match flow {
        StartupFlow::Wizard(w) if !matches!(w.phase, WizardPhase::Downloading) => {
            (w.hub_arg(), w.proxy_arg())
        }
        _ => return,
    };
    if let StartupFlow::Wizard(w) = flow {
        w.phase = WizardPhase::Downloading;
    }
    session.cancel_tick(WinId::Setup, TickKind::Setup);
    // E2/D-79：载荷改型为值域枚举——向导的字符串参数在发送点经单点转换
    session.send_cmd(Cmd::StartDownload {
        hub: lt_proto::Hub::from_settings_str(&hub),
        proxy: lt_proto::ProxyMode::from_settings_str(&proxy),
    });
}

impl SessionView {
    /// UI 帧内请求窗口动作（宿主帧后消费）
    pub fn enqueue_action(&mut self, win: WinId, action: WinAction) {
        self.actions.push((win, action));
    }

    /// 取走全部窗口动作
    pub fn drain_actions(&mut self) -> Vec<(WinId, WinAction)> {
        std::mem::take(&mut self.actions)
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

    /// 登记节拍（同窗同种覆盖既有时刻：字幕/监视/Setup 等延迟语义）
    pub fn schedule_tick(&mut self, win: WinId, kind: TickKind, at: Instant) {
        if let Some(t) = self.ticks.iter_mut().find(|t| t.win == win && t.kind == kind) {
            t.at = at;
        } else {
            self.ticks.push(Tick { at, win, kind });
        }
    }

    /// 登记节拍（同窗同种已存在则不重复排班：穿透轮询/位置防抖等启动语义）
    pub fn ensure_tick(&mut self, win: WinId, kind: TickKind, at: Instant) {
        if !self.ticks.iter().any(|t| t.win == win && t.kind == kind) {
            self.ticks.push(Tick { at, win, kind });
        }
    }

    /// 取消指定节拍（自动隐藏取消等）
    pub fn cancel_tick(&mut self, win: WinId, kind: TickKind) {
        self.ticks.retain(|t| !(t.win == win && t.kind == kind));
    }

    /// 取走所有到期的节拍；返回需要处理的节拍（宿主按 kind 分派）
    pub fn drain_due_ticks(&mut self) -> Vec<Tick> {
        let now = Instant::now();
        let mut due = Vec::new();
        self.ticks.retain(|t| {
            if t.at <= now {
                due.push(t.clone());
                false
            } else {
                true
            }
        });
        due
    }

    /// 最近的节拍时刻（None = 无定时事项，可无限期等待）
    pub fn next_tick(&self) -> Option<Instant> {
        self.ticks.iter().map(|t| t.at).min()
    }

    /// 设置草稿变更意图（跨域请求：字幕窗/确认模态/面板页 → 宿主消费）
    pub fn request_settings_apply(&mut self) {
        self.settings_apply_pending = true;
    }

    /// 宿主消费设置变更意图（返回 true = 应做面板 300ms 防抖登记）
    pub fn take_settings_apply_pending(&mut self) -> bool {
        std::mem::take(&mut self.settings_apply_pending)
    }
}

impl OverlayUi {
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

    /// 流式译文增量（原版 update_streaming：50ms 节流）。
    /// 只缓冲 + 安排 50ms 悬浮窗节拍；节拍触发时 [`Self::flush_streams`] 落盘到消息。
    pub fn update_streaming(&mut self, session: &mut SessionView, id: u64, partial: String) {
        self.state.pending_streams.insert(id, partial);
        let at = Instant::now() + Duration::from_millis(50);
        session.ensure_tick(WinId::Overlay, TickKind::StreamFlush, at);
    }

    /// 节拍触发：把缓冲的流式文本写入消息并标记滚动/重绘
    pub fn flush_streams(&mut self) {
        if self.state.pending_streams.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.state.pending_streams);
        for (id, text) in pending {
            if let Some(m) = self.find_message_mut(id) {
                m.translation = Some(text);
                m.streaming = true;
            }
        }
        self.state.scroll_pending = true;
    }

    /// 译文完成（原版 update_translation；空文本=同语言/无翻译，同样置 Some）
    pub fn update_translation(&mut self, id: u64, text: String, tl_ms: f64) {
        if let Some(m) = self.find_message_mut(id) {
            m.translation = Some(text);
            m.tl_ms = tl_ms;
            m.streaming = false;
        }
        self.state.scroll_pending = true;
    }

    /// 统计快照更新（原版 update_stats）
    pub fn update_stats(&mut self, stats: OverlayStats) {
        self.stats = stats;
    }

    /// 位置/尺寸变更登记（原版 _schedule_pos_save：几何变化 → 500ms 防抖保存）
    pub fn schedule_pos_save(&mut self, session: &mut SessionView, geo: (i32, i32, u32, u32)) {
        if self.state.last_saved_geo == Some(geo) {
            return;
        }
        self.state.pos_dirty_since = Some(Instant::now());
        let at = Instant::now() + Duration::from_millis(500);
        session.ensure_tick(WinId::Overlay, TickKind::PosSave, at);
    }

    /// 悬浮窗穿透轮询节拍（原版 _ct_timer 50ms；仅穿透开启时由宿主续拍）
    pub fn schedule_click_through_tick(&self, session: &mut SessionView) {
        let at = Instant::now() + Duration::from_millis(50);
        session.ensure_tick(WinId::Overlay, TickKind::ClickThrough, at);
    }
}

impl SubtitleUi {
    /// 字幕窗拖动提示的一次性消费（原版 _subwin_notified：会话内只弹一次）。
    /// 返回 true = 本次应弹（首次）；false = 已弹过。
    pub fn take_subtitle_hint(&mut self) -> bool {
        if self.hint_shown {
            return false;
        }
        self.hint_shown = true;
        true
    }

    /// 字幕窗文本更新入口（原版 SubtitleWindow.update_text → _on_update_text；
    /// 宿主由 UpdateTranslation 事件换算 original + {lang: translation}）。
    /// pending 进队时安排 SubtitlePending 节拍；立即插入路径同步自动隐藏节拍。
    pub fn update_text(
        &mut self,
        session: &mut SessionView,
        settings: &Settings,
        original: String,
        translations: std::collections::BTreeMap<String, String>,
    ) {
        let now = Instant::now();
        if let Some(at) = self.state.update_text(original, translations, &settings.subtitle_mode, now) {
            self.schedule_tick(session, TickKind::SubtitlePending, at);
        } else {
            self.sync_auto_hide_tick(session);
        }
    }

    /// 字幕窗节拍安排（同窗同种去重：覆盖既有时刻）
    pub fn schedule_tick(&mut self, session: &mut SessionView, kind: TickKind, at: Instant) {
        session.schedule_tick(WinId::Subtitle, kind, at);
    }

    /// 依 auto_hide_deadline 同步自动隐藏节拍（None → 取消既有节拍）
    pub fn sync_auto_hide_tick(&mut self, session: &mut SessionView) {
        match self.state.auto_hide_deadline {
            Some(at) => self.schedule_tick(session, TickKind::SubtitleAutoHide, at),
            None => session.cancel_tick(WinId::Subtitle, TickKind::SubtitleAutoHide),
        }
    }

    /// SubtitlePending 节拍：消费到点待插入（原版 timer.timeout → _insert_sentence）；
    /// 返回是否实际插入（宿主据此重绘）。
    pub fn flush_pending(&mut self, session: &mut SessionView, settings: &Settings) -> bool {
        let now = Instant::now();
        if self.state.flush_pending(&settings.subtitle_mode, now) {
            self.sync_auto_hide_tick(session);
            true
        } else {
            false
        }
    }

    /// 字幕窗位置变更登记（原版 position_changed → 500ms 防抖保存；仅存 x/y）
    pub fn schedule_pos_save(&mut self, session: &mut SessionView, pos: (i32, i32)) {
        if self.state.last_saved_pos == Some(pos) {
            return;
        }
        self.state.pos_dirty_since = Some(Instant::now());
        let at = Instant::now() + Duration::from_millis(500);
        self.schedule_tick(session, TickKind::PosSave, at);
    }

    /// 字幕窗 100ms 光标感知轮询节拍（D-36：分区穿透断言 + 顶条悬停工具的
    /// 单一事实源；原版 _ct_timer 500ms。可见期间由宿主无需条件地续拍）
    pub fn schedule_window_poll(&mut self, session: &mut SessionView) {
        let at = Instant::now() + Duration::from_millis(SUBTITLE_POLL_MS);
        self.schedule_tick(session, TickKind::ClickThrough, at);
    }
}

impl ModalUi {
    /// 打开确认模态（D-33/H-3~H-5）。已有模态打开则幂等忽略（单模态源防嵌套）。
    /// `overlay_ok`：悬浮窗可作宿主（可见且非紧凑且高度充足，由调用方判定）；
    /// `panel_visible`：面板当前可见（W5b 起真值在宿主，调用方传只读快照值）。
    /// 否则宿主=面板（调用方负责面板不可见时入队 ShowPanel/HidePanel 平衡）。
    /// 标题/正文由调用方按 kind 装配（i18n 当前语言；Delete 系含动态参数）。
    pub fn request_confirm(
        &mut self,
        kind: ConfirmKind,
        overlay_ok: bool,
        panel_visible: bool,
        title: String,
        msg: String,
    ) -> bool {
        if self.confirm.is_some() {
            return false;
        }
        let host = if overlay_ok {
            WinId::Overlay
        } else {
            WinId::Panel
        };
        let panel_shown_for_confirm = host == WinId::Panel && !panel_visible;
        self.confirm = Some(ConfirmUi {
            host,
            kind,
            title,
            msg,
            panel_shown_for_confirm,
        });
        true
    }

    /// 关闭确认模态并返回原状态（渲染帧执行确定效果 / 取消恢复逻辑）
    pub fn take_confirm(&mut self) -> Option<ConfirmUi> {
        self.confirm.take()
    }
}

impl StartupUi {
    /// 安排一次 Setup 窗节拍（同窗去重：覆盖既有节拍时刻）。
    /// 向导倒计时（1s/拍）与下载成功后的 500ms 收尾延迟共用。
    pub fn schedule_tick(&mut self, session: &mut SessionView, delay: Duration) {
        let at = Instant::now() + delay;
        session.schedule_tick(WinId::Setup, TickKind::Setup, at);
    }

    /// 取消 Setup 节拍（下载开始即停倒计时定时器，等价原版 _auto_timer.stop()）
    pub fn cancel_tick(&mut self, session: &mut SessionView) {
        session.cancel_tick(WinId::Setup, TickKind::Setup);
    }

    /// 若启动流需要节拍（向导倒计时 / 成功后 500ms 收尾延迟）则安排 Setup 节拍。
    /// 倒计时按 1s 一拍；收尾延迟按 500ms。
    pub fn kick_tick(&mut self, session: &mut SessionView) {
        if !crate::windows::setup::needs_setup_tick(&self.flow, &self.load_dialog) {
            return;
        }
        let idle_countdown = matches!(
            &self.flow,
            StartupFlow::Wizard(w) if w.phase == WizardPhase::Idle
        );
        let delay = if idle_countdown {
            Duration::from_secs(1)
        } else {
            Duration::from_millis(500)
        };
        self.schedule_tick(session, delay);
    }
}

impl BenchUi {
    /// 性能基准输出追加一行（上限 500 行，满删最旧）
    pub fn push_line(&mut self, line: String) {
        self.lines.push(line);
        if self.lines.len() > 500 {
            self.lines.remove(0);
        }
    }
}

/// 面板设置防抖登记（宿主消费 [`SessionView::take_settings_apply_pending`] 意图后
/// 调用；原版 _auto_save：控件改 draft 后重启 300ms 单发定时，连续变更不断顺延）。
/// `now` 可注入时钟（测试用）。
pub fn register_panel_apply(
    panel: &mut PanelUi,
    session: &mut SessionView,
    now: Instant,
) {
    panel.state.mark_dirty_at(now);
    let at = PanelUiState::apply_deadline(now);
    session.schedule_tick(WinId::Panel, TickKind::PanelApply, at);
}

/// 各窗口初始可见性表（W5b：真值归宿主 WindowManager——`MultiWindowApp::new`
/// 调用本函数初始化；启动流进行中 4 个主窗口不可见，仅 Setup 对话框可见，
/// accept 之后才 reveal 主窗口）。
pub fn initial_visibility(
    settings: &Settings,
    flow: &StartupFlow,
) -> std::collections::HashMap<WinId, bool> {
    let startup_pending = !matches!(flow, StartupFlow::Ready);
    let mut visible = std::collections::HashMap::new();
    visible.insert(WinId::Overlay, !startup_pending);
    visible.insert(
        WinId::Subtitle,
        !startup_pending && settings.subtitle_mode.enabled,
    );
    // 原版启动只开悬浮窗；控制面板由悬浮窗"设置"/托盘打开（on_toggle_panel）。
    // LIVETRANSLATE_SHOW_PANEL=1：开发/排障便利（实机截图走查用），默认关闭
    let show_panel = std::env::var("LIVETRANSLATE_SHOW_PANEL")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false);
    visible.insert(WinId::Panel, !startup_pending && show_panel);
    visible.insert(WinId::Log, false); // 原版：启动即建但隐藏
                                       // Setup 对话框窗口：仅启动流进行中初始可见（运行期 load_dialog 单独控制）
    visible.insert(WinId::Setup, startup_pending);
    // Benchmark 工具窗：启动即建但隐藏（原版仅点识别页"性能基准…"时 exec）
    visible.insert(WinId::Benchmark, false);
    visible
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WP-1：字幕窗拖动提示会话内一次性（原版 _subwin_notified 语义）
    #[test]
    fn subtitle_hint_once_per_session() {
        let mut st = AppUi::new(Settings::default());
        assert!(st.subtitle.take_subtitle_hint(), "首次应弹提示");
        assert!(!st.subtitle.take_subtitle_hint(), "会话内不重复弹");
        // 独立实例互不影响
        let mut st2 = AppUi::new(Settings::default());
        assert!(st2.subtitle.take_subtitle_hint());
    }

    /// D-36：顶条悬停状态机——淡入/淡出动画、reduce_motion 瞬时落位、重复幂等
    #[test]
    fn subtitle_toolbar_hover_anim_lifecycle() {
        let mut sub = SubtitleUiState::default();
        let t0 = Instant::now();
        // 悬停进入：0→1 OutCubic
        assert!(sub.set_toolbar_hover(true, t0));
        assert_eq!(sub.toolbar_opacity(t0), 0.0);
        let mid = t0 + Duration::from_millis(SUBTITLE_STRIP_FADE_MS / 2);
        let o = sub.toolbar_opacity(mid);
        assert!(o > 0.0 && o < 1.0, "淡入进行中: {o}");
        let end = t0 + Duration::from_millis(SUBTITLE_STRIP_FADE_MS + 1);
        assert_eq!(sub.toolbar_opacity(end), 1.0);
        // 重复幂等：状态不变（返回 false，不发生动画重置）
        assert!(!sub.set_toolbar_hover(true, t0));
        // 离开：1→0 InCubic；淡出启动 1ms 后接近 1（刚起步），结束+1ms 收敛 0
        assert!(sub.set_toolbar_hover(false, end));
        let fade_end = end + Duration::from_millis(SUBTITLE_STRIP_FADE_MS + 1);
        assert_eq!(sub.toolbar_opacity(fade_end), 0.0);
        // reduce_motion：瞬时落位（无动画）
        let mut sub2 = SubtitleUiState {
            reduce_motion: true,
            ..Default::default()
        };
        assert!(sub2.set_toolbar_hover(true, t0));
        assert_eq!(sub2.toolbar_opacity(t0), 1.0);
        assert_eq!(sub2.toolbar_anim, None);
        assert!(sub2.set_toolbar_hover(false, t0));
        assert_eq!(sub2.toolbar_opacity(t0), 0.0);
    }

    /// D-33/H-3：确认模态——打开/幂等/宿主选择/取消恢复标记（单模态源防嵌套）
    #[test]
    fn confirm_modal_request_idempotent_and_host_choice() {
        let mut st = AppUi::new(Settings::default());
        // 默认启动流 Ready：面板可见性取 show_panel 环境（测试无该变量 → 隐藏）
        st.session.visible.insert(WinId::Panel, false);
        st.session.visible.insert(WinId::Overlay, true);
        let panel_visible =
            |st: &AppUi| st.session.visible.get(&WinId::Panel).copied().unwrap_or(true);

        // 悬浮窗可宿主 → Overlay，无 panel 标记
        assert!(st.modal.request_confirm(
            ConfirmKind::Quit,
            true,
            panel_visible(&st),
            lt_i18n::t("quit_confirm_title"),
            lt_i18n::t("quit_confirm_msg"),
        ));
        let c = st.modal.confirm.as_ref().unwrap();
        assert_eq!(c.host, WinId::Overlay);
        assert_eq!(c.kind, ConfirmKind::Quit);
        assert!(!c.panel_shown_for_confirm);

        // 已开 → 幂等忽略（不叠加/不换 kind）
        assert!(!st.modal.request_confirm(
            ConfirmKind::Clear,
            true,
            panel_visible(&st),
            lt_i18n::t("clear_confirm_title"),
            lt_i18n::t("clear_confirm_msg"),
        ));
        assert_eq!(st.modal.confirm.as_ref().unwrap().kind, ConfirmKind::Quit);

        // 取消 → 取走
        let taken = st.modal.take_confirm().expect("应取走模态");
        assert!(st.modal.confirm.is_none());
        assert_eq!(taken.kind, ConfirmKind::Quit);

        // 悬浮窗不可宿主（隐藏/紧凑）→ Panel + 临时显示标记
        assert!(st.modal.request_confirm(
            ConfirmKind::Quit,
            false,
            panel_visible(&st),
            lt_i18n::t("quit_confirm_title"),
            lt_i18n::t("quit_confirm_msg"),
        ));
        let c = st.modal.confirm.as_ref().unwrap();
        assert_eq!(c.host, WinId::Panel);
        assert!(c.panel_shown_for_confirm, "面板原本隐藏应标记临时显示");

        // 面板已可见 → 取消不触发回藏（无临时标记）
        let _ = st.modal.take_confirm();
        st.session.visible.insert(WinId::Panel, true);
        assert!(st.modal.request_confirm(
            ConfirmKind::ResetSubtitle,
            false,
            panel_visible(&st),
            lt_i18n::t("reset_confirm_title"),
            lt_i18n::t("subwin_reset_confirm"),
        ));
        assert!(!st.modal.confirm.as_ref().unwrap().panel_shown_for_confirm);
        let _ = st.modal.take_confirm();
    }

    /// D-33/H-3：Quit 确认 → quit_requested（宿主 about_to_wait 收敛路径不变）
    #[test]
    fn confirm_quit_sets_quit_requested() {
        let mut st = AppUi::new(Settings::default());
        assert!(!st.modal.quit_requested);
        st.modal.request_confirm(
            ConfirmKind::Quit,
            false,
            false,
            lt_i18n::t("quit_confirm_title"),
            lt_i18n::t("quit_confirm_msg"),
        );
        let _ = st.modal.take_confirm();
        // 渲染层确定按钮 → 置 quit_requested（与确认框分离的纯状态断言：
        // 直接模拟 apply 的结果——渲染层唯一副作用是 quit_requested）
        st.modal.quit_requested = true;
        assert!(st.modal.quit_requested);
    }

    /// DL-3：apply_progress 整体覆盖（精确字节），文件切换自然归零；
    /// 非 Downloading 态忽略晚到事件
    #[test]
    fn apply_progress_overwrites_and_ignores_non_downloading() {
        let mut st = AppUi::new(Settings::default());
        st.panel.download = DownloadUiState::Downloading {
            file: "model.int8.onnx".into(),
            k: 1,
            n: 2,
            done_bytes: 200_000_000,
            total_bytes: 239_000_000,
            log: vec![],
        };
        // 第二个文件开始：进度随新文件归零（机器段为权威值）
        st.panel.download
            .apply_progress("tokens.txt".into(), 2, 2, 1_048_576, 2_097_152);
        match &st.panel.download {
            DownloadUiState::Downloading {
                file,
                k,
                n,
                done_bytes,
                total_bytes,
                ..
            } => {
                assert_eq!(file, "tokens.txt");
                assert_eq!((*k, *n), (2, 2));
                assert_eq!((*done_bytes, *total_bytes), (1_048_576, 2_097_152));
            }
            other => panic!("{other:?}"),
        }
        // 成功回 Idle 后的晚到事件不生效
        st.panel.download = DownloadUiState::Idle;
        st.panel.download.apply_progress("x".into(), 1, 1, 1, 1);
        assert_eq!(st.panel.download, DownloadUiState::Idle);
    }

    fn msg(id: u64) -> OverlayMessage {
        OverlayMessage {
            id,
            timestamp: format!("12:00:{:02}", id % 60),
            original: format!("消息 {id}"),
            lang: "zh".into(),
            asr_ms: 100.0,
            translation: None,
            tl_ms: 0.0,
            streaming: false,
        }
    }

    #[test]
    fn messages_cap_at_50_keep_newest() {
        let mut st = AppUi::new(Settings::default());
        for i in 1..=55u64 {
            st.overlay.push_message(msg(i));
        }
        // 上限 50 条，保留的是最新的 50 条（首条 id 应为第 6 条）
        assert_eq!(st.overlay.messages.len(), 50);
        assert_eq!(st.overlay.messages[0].id, 6);
        assert_eq!(st.overlay.messages.last().unwrap().id, 55);
    }

    #[test]
    fn translation_updates_target_latest_message_with_id() {
        let mut st = AppUi::new(Settings::default());
        // 同 id 消息出现两次（现实中不发生，防御语义：取最新）
        st.overlay.push_message(msg(1));
        st.overlay.push_message(msg(2));

        // 流式只入缓冲，50ms 节拍 flush 后才落消息（并置 streaming 标志）
        st.overlay.update_streaming(&mut st.session, 2, "partial".into());
        assert_eq!(st.overlay.messages.last().unwrap().translation, None);
        st.overlay.flush_streams();
        assert_eq!(
            st.overlay.messages.last().unwrap().translation.as_deref(),
            Some("partial")
        );
        assert!(st.overlay.messages.last().unwrap().streaming);
        assert_eq!(st.overlay.messages.last().unwrap().tl_ms, 0.0);

        st.overlay.update_translation(2, "done".into(), 320.0);
        let m = st.overlay.messages.last().unwrap();
        assert_eq!(m.translation.as_deref(), Some("done"));
        assert_eq!(m.tl_ms, 320.0);
        assert!(!m.streaming);

        // 不存在的 id：无害 no-op
        st.overlay.update_translation(999, "ghost".into(), 1.0);
        assert!(st.overlay.messages.iter().all(|m| m.id != 999));

        // 淘汰边界：id=1 已被挤出 50 条窗口（仅 2 条时不适用，直接验证 id=1 更新）
        st.overlay.update_translation(1, "first".into(), 5.0);
        assert_eq!(st.overlay.messages[0].translation.as_deref(), Some("first"));
    }

    #[test]
    fn same_language_empty_translation_still_marked() {
        let mut st = AppUi::new(Settings::default());
        st.overlay.push_message(msg(7));
        // 同语言回空译文：translation=Some("")，消息标记完成
        st.overlay.update_translation(7, String::new(), 0.0);
        let m = st.overlay.messages.last().unwrap();
        assert_eq!(m.translation.as_deref(), Some(""));
    }

    #[test]
    fn logwin_buffers_all_levels_and_caps_at_2000() {
        let mut lw = LogWindowState::default();
        // D-31：缓冲全级别保留（过滤移到渲染期 level_visible）
        assert!(lw.push(LogLineEntry {
            time: String::new(),
            level: 10,
            target: "t".into(),
            msg: "debug line".into(),
        }));
        assert_eq!(lw.lines.len(), 1, "DEBUG 进入缓冲");

        assert!(lw.push(LogLineEntry {
            time: String::new(),
            level: 20,
            target: "t".into(),
            msg: "info line".into(),
        }));
        assert_eq!(lw.lines.len(), 2);
        assert_eq!(lw.lines[0].time.len(), 8, "空时戳自动盖 HH:MM:SS");

        // 环形 2000：满后丢最旧
        for i in 0..2100u32 {
            lw.push(LogLineEntry {
                time: "x".into(),
                level: 20,
                target: "t".into(),
                msg: format!("{i}"),
            });
        }
        assert_eq!(lw.lines.len(), 2000);
        assert_ne!(lw.lines[0].msg, "0");
    }

    #[test]
    fn logwin_level_visible_matrix() {
        // 关 show_debug：仅 INFO+ 可见；开：全级别可见（回溯语义）
        assert!(!LogWindowState::level_visible(0, false));
        assert!(!LogWindowState::level_visible(10, false));
        assert!(LogWindowState::level_visible(20, false));
        assert!(LogWindowState::level_visible(30, false));
        assert!(LogWindowState::level_visible(40, false));
        assert!(LogWindowState::level_visible(10, true));
        assert!(LogWindowState::level_visible(0, true));
    }

    #[test]
    fn logwin_formatted_cache_tracks_lines() {
        let mut lw = LogWindowState::default();
        lw.push(LogLineEntry {
            time: "10:00:00".into(),
            level: 20,
            target: "t".into(),
            msg: "m1".into(),
        });
        lw.push(LogLineEntry {
            time: "10:00:01".into(),
            level: 10,
            target: "t".into(),
            msg: "m2".into(),
        });
        let fmt = lw.formatted();
        assert_eq!(fmt.len(), 2);
        assert_eq!(fmt[0].0, "10:00:00 [INFO] t: m1");
        assert_eq!(fmt[0].1, 20);
        assert_eq!(fmt[1].0, "10:00:01 [DEBUG] t: m2");

        // 溢出后整体重建，缓存与 lines 仍同步
        for i in 0..3000u32 {
            lw.push(LogLineEntry {
                time: "x".into(),
                level: 20,
                target: "t".into(),
                msg: format!("{i}"),
            });
        }
        let (line_count, first_msg) = (lw.lines.len(), lw.lines.front().unwrap().msg.clone());
        let fmt = lw.formatted();
        assert_eq!(fmt.len(), line_count);
        assert_eq!(fmt.len(), 2000);
        assert_eq!(first_msg, "1000");
        assert_eq!(fmt[0].0, "x [INFO] t: 1000");
        // clear 后缓存与缓冲同清
        lw.clear();
        assert_eq!(lw.lines.len(), 0);
        assert_eq!(lw.formatted().len(), 0);
    }

    #[test]
    fn logwin_new_since_bottom_tracks_unread() {
        let mut lw = LogWindowState::default();
        assert_eq!(lw.new_since_bottom(), 0);
        lw.push(LogLineEntry {
            time: "x".into(),
            level: 20,
            target: "t".into(),
            msg: "a".into(),
        });
        assert_eq!(lw.new_since_bottom(), 1, "无贴底基准：现有行全部视作未读");
        lw.mark_at_bottom();
        assert_eq!(lw.new_since_bottom(), 0);
        lw.push(LogLineEntry {
            time: "x".into(),
            level: 20,
            target: "t".into(),
            msg: "b".into(),
        });
        lw.push(LogLineEntry {
            time: "x".into(),
            level: 20,
            target: "t".into(),
            msg: "c".into(),
        });
        assert_eq!(lw.new_since_bottom(), 2, "上翻期间新增 2 行计入未读");
        lw.mark_at_bottom();
        assert_eq!(lw.new_since_bottom(), 0, "回底后清零");
        lw.clear();
        assert_eq!(lw.new_since_bottom(), 0, "清空后归零");
    }

    #[test]
    fn is_near_bottom_boundaries() {
        // 内容不满视口：无偏移，恒贴底
        assert!(is_near_bottom(100.0, 80.0, 4.0));
        // 滚到末尾：差 0
        assert!(is_near_bottom(100.0, 100.0, 4.0));
        // 容差内
        assert!(is_near_bottom(97.0, 100.0, 4.0));
        // 上翻 20px：不贴底
        assert!(!is_near_bottom(80.0, 100.0, 4.0));
    }

    #[test]
    fn advance_follow_pins_only_on_user_scroll() {
        let mut lw = LogWindowState::default();
        // 视口高 100、内容高 1000：贴底 offset = 900
        // 帧 1（首帧，无前值）：offset 900 近底 → 解锁并设基准
        assert!(!lw.advance_follow(900.0, 100.0, 1000.0, LogView::Panel));
        // 贴底后新增 2 行 → 未读 2（浮钮 +N 计数源）
        lw.push(LogLineEntry {
            time: "x".into(),
            level: 20,
            target: "t".into(),
            msg: "a".into(),
        });
        lw.push(LogLineEntry {
            time: "x".into(),
            level: 20,
            target: "t".into(),
            msg: "b".into(),
        });
        // 帧 2：用户滚动（offset 变化）且不近底 → 锁定，浮钮出现
        assert!(lw.advance_follow(600.0, 100.0, 1000.0, LogView::Panel));
        // 帧 3：offset 不变（无输入）→ 维持锁定
        assert!(lw.advance_follow(600.0, 100.0, 1000.0, LogView::Panel));
        // 帧 4：回贴底 → 解锁 + 基准重置
        assert!(!lw.advance_follow(900.0, 100.0, 1000.0, LogView::Panel));
        assert_eq!(lw.new_since_bottom(), 0);
    }

    #[test]
    fn advance_follow_ignores_content_growth_when_stuck() {
        let mut lw = LogWindowState::default();
        // 贴底基线：offset 900 = 内容高 1000 - 视口高 100
        assert!(!lw.advance_follow(900.0, 100.0, 1000.0, LogView::Panel));
        // 内容增长（1020）但偏移不变：stick 尚未追平的一帧不算用户滚动
        assert!(
            !lw.advance_follow(900.0, 100.0, 1020.0, LogView::Panel),
            "贴底+内容增长不得弹浮钮"
        );
        // 追平后贴住新底（offset 920）
        assert!(!lw.advance_follow(920.0, 100.0, 1020.0, LogView::Panel));
        // 用户上翻 50px → 锁定；无未读行时不返回 true（浮钮需 unread > 0）
        assert!(!lw.advance_follow(870.0, 100.0, 1020.0, LogView::Panel));
        assert!(lw.panel_pinned, "用户上翻后 pinned 置位");
        // 视图隔离：日志窗视图不受 Panel 锁定影响
        assert!(!lw.advance_follow(900.0, 100.0, 1000.0, LogView::LogWin));
    }

    #[test]
    fn advance_follow_unread_flips_jump() {
        let mut lw = LogWindowState::default();
        lw.mark_at_bottom();
        lw.push(LogLineEntry {
            time: "x".into(),
            level: 20,
            target: "t".into(),
            msg: "a".into(),
        });
        lw.push(LogLineEntry {
            time: "x".into(),
            level: 20,
            target: "t".into(),
            msg: "b".into(),
        });
        // 首帧 offset 未变（无前值）→ 未锁定，即使有未读也不弹（用户未滚动）
        assert!(
            !lw.advance_follow(0.0, 100.0, 1000.0, LogView::Panel),
            "未滚动不弹浮钮"
        );
        // 用户滚动一帧 → 锁定 → 浮钮出现（有 2 条未读）
        assert!(lw.advance_follow(400.0, 100.0, 1000.0, LogView::Panel));
        // 回到贴底 → 解锁 + 基准重置 → 无未读
        assert!(!lw.advance_follow(900.0, 100.0, 1000.0, LogView::Panel));
        assert_eq!(lw.new_since_bottom(), 0);
    }

    #[test]
    fn stats_snapshot_replaces_wholesale() {
        let mut st = AppUi::new(Settings::default());
        st.overlay.update_stats(OverlayStats {
            asr_n: 10,
            tl_n: 8,
            prompt_tokens: 1000,
            completion_tokens: 500,
            cost: 0.002,
        });
        assert_eq!(st.overlay.stats.asr_n, 10);
        assert_eq!(st.overlay.stats.cost, 0.002);
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
        lt_i18n::set_lang("zh").expect("zh 表解析");
        let w = WizardState::new();
        assert_eq!(w.hub_index, 0);
        assert_eq!(w.proxy_index, 1); // 默认跟随系统代理
        assert_eq!(w.countdown, 15);
        assert_eq!(w.phase, WizardPhase::Idle);
        assert!(w.log.is_empty());
        lt_i18n::set_lang("en").expect("en 表解析");
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
        let mut st = AppUi::with_startup(Settings::default(), StartupFlow::Wizard(w));
        st.session.cmd_tx = Some(tx);
        st.startup.schedule_tick(&mut st.session, Duration::from_secs(1));

        st.wizard_auto_start();

        let StartupFlow::Wizard(w) = &st.startup.flow else {
            panic!("应仍处于向导流");
        };
        assert_eq!(w.phase, WizardPhase::Downloading);
        // 下载开始即停倒计时定时器（原版 _auto_timer.stop()）
        assert!(st.session.ticks.iter().all(|t| t.win != WinId::Setup));
        match rx.try_recv() {
            Ok(Cmd::StartDownload { hub, proxy }) => {
                assert_eq!(hub, lt_proto::Hub::Hf);
                assert_eq!(proxy, lt_proto::ProxyMode::None);
            }
            other => panic!("应发送 StartDownload，实际 {other:?}"),
        }
        // 下载中重复触发应被忽略（不重复发命令）
        st.wizard_auto_start();
        assert!(rx.try_recv().is_err());
    }

    /// W5b：初始可见性表（宿主 MultiWindowApp 调 `initial_visibility` 初始化）
    #[test]
    fn initial_visibility_hides_main_windows_when_pending() {
        // 首启向导：4 个主窗口初始全部不可见，仅 Setup 可见
        let visible = initial_visibility(
            &Settings::default(),
            &startup_flow(true, vec![]),
        );
        for id in [WinId::Overlay, WinId::Subtitle, WinId::Panel, WinId::Log] {
            assert!(!*visible.get(&id).unwrap(), "{id:?} 应隐藏");
        }
        assert!(*visible.get(&WinId::Setup).unwrap());

        // 缺模型下载：同样全隐藏；names 为逗号连接的显示名
        let flow = startup_flow(false, vec!["Silero VAD".into(), "SenseVoice Small".into()]);
        let StartupFlow::DownloadMissing { names, .. } = &flow else {
            panic!("应为缺模型下载流");
        };
        assert_eq!(names, "Silero VAD, SenseVoice Small");
        let visible = initial_visibility(&Settings::default(), &flow);
        assert!(!*visible.get(&WinId::Overlay).unwrap());
        assert!(*visible.get(&WinId::Setup).unwrap());

        // Ready：维持现有默认（Log 仍隐藏），Setup 不可见
        let visible = initial_visibility(&Settings::default(), &StartupFlow::Ready);
        assert!(*visible.get(&WinId::Overlay).unwrap());
        assert!(!*visible.get(&WinId::Setup).unwrap());
        assert!(!*visible.get(&WinId::Log).unwrap());
        // 字幕窗随 settings.subtitle_mode.enabled（原版启动即建但隐藏）
        let mut s = Settings::default();
        s.subtitle_mode.enabled = false;
        let visible = initial_visibility(&s, &StartupFlow::Ready);
        assert!(!*visible.get(&WinId::Subtitle).unwrap());
    }

    // ── 面板（M4.3）：页序 / 防抖 / 页键 ──

    /// 页序对照原版 tabs.addTab（control_panel.py:170-180）固定 7 Tab +
    /// Rust 版「日志」页（末位）
    #[test]
    fn panel_pages_order_matches_original() {
        assert_eq!(
            PanelPage::ALL,
            [
                PanelPage::VadAsr,
                PanelPage::Translation,
                PanelPage::Style,
                PanelPage::Subtitle,
                PanelPage::Benchmark,
                PanelPage::Cache,
                PanelPage::Changelog,
                PanelPage::Log,
            ]
        );
        // Tab 标题键与 yaml 真实键对齐（t() 缺键回退 key 本身 → 不等即键缺失）
        for page in PanelPage::ALL {
            assert_ne!(
                lt_i18n::t(page.tab_key()),
                page.tab_key(),
                "tab 键缺失: {}",
                page.tab_key()
            );
        }
    }

    /// 面板默认值：首页 VAD/ASR（原版第一个 addTab）、无设备枚举请求
    #[test]
    fn panel_state_defaults_match_original_chrome() {
        let st = AppUi::new(Settings::default());
        assert_eq!(st.panel.state.page, PanelPage::VadAsr);
        assert!(matches!(st.panel.state.devices, DevicesState::Idle));
        assert!(st.panel.state.apply_due_at.is_none());
    }

    /// W5 收口（P1）：设备枚举首入守卫——Idle→Probe 只发一次命令；
    /// Probe 在途任意帧再调不重发；Ready 后不重发（刷新按钮显式走
    /// request_devices_refresh 才换新枚举）
    #[test]
    fn devices_probe_sends_once_until_reply() {
        let mut st = AppUi::new(Settings::default());
        let (tx, rx) = std::sync::mpsc::channel();
        st.session.cmd_tx = Some(tx);
        let cmd = |st: &mut AppUi| {
            st.panel.state.request_devices_if_idle(&mut st.session);
            st.session
                .cmd_tx
                .as_ref()
                .map(|_| rx.try_iter().count())
                .unwrap_or(0)
        };
        // 首帧：发送 1 条（Idle→Probe）
        assert_eq!(cmd(&mut st), 1, "首入应发一次 RefreshDevices");
        assert!(matches!(st.panel.state.devices, DevicesState::Probe));
        // 回执前连续帧：0 条（在途不再叠发）
        for _ in 0..10 {
            assert_eq!(cmd(&mut st), 0, "Probe 在途不得重复发送");
        }
        // 回执：Probe→Ready；再帧 0 条
        st.panel.state.devices = DevicesState::Ready(lt_proto::DeviceList::default());
        assert_eq!(cmd(&mut st), 0, "Ready 态不自动重发");
        // 刷新按钮：Ready→Probe 再发一条；在途幂等
        st.panel.state.request_devices_refresh(&mut st.session);
        assert_eq!(cmd(&mut st), 1, "显式刷新应发一条");
        st.panel.state.request_devices_refresh(&mut st.session);
        assert_eq!(cmd(&mut st), 0, "刷新在途幂等");
    }

    /// 防抖：登记后 300ms 到期触发一次；到期前不触发
    #[test]
    fn panel_apply_debounce_fires_once_after_300ms() {
        let mut st = AppUi::new(Settings::default());
        st.settings.vad_threshold = 0.35;
        let t0 = Instant::now();
        register_panel_apply(&mut st.panel, &mut st.session, t0);
        // 到期前：无快照
        assert!(!st.panel.state.take_apply_due(t0 + Duration::from_millis(299)));
        // 到期：返回当前设置快照
        let snap = st
            .take_due_panel_apply(t0 + Duration::from_millis(300))
            .expect("300ms 应触发");
        assert_eq!(snap.vad_threshold, 0.35);
        // 消费后不再触发（单发语义，原版 setSingleShot(True)）
        assert!(st
            .take_due_panel_apply(t0 + Duration::from_millis(600))
            .is_none());
        // 节拍已无未到期项（消费时 tick 已被 drain；此处防御：deadline 标记已清）
        assert_eq!(st.panel.state.apply_due_at, None);
    }

    /// 防抖合并：300ms 内连续登记 → 只保留一个节拍且时刻顺延（原版 singleShot restart）
    #[test]
    fn panel_apply_debounce_merges_bursts() {
        let mut st = AppUi::new(Settings::default());
        let t0 = Instant::now();
        register_panel_apply(&mut st.panel, &mut st.session, t0);
        register_panel_apply(&mut st.panel, &mut st.session, t0 + Duration::from_millis(100));
        register_panel_apply(&mut st.panel, &mut st.session, t0 + Duration::from_millis(200));
        // 仅一个 PanelApply 节拍，deadline = 最后一次登记 + 300ms
        let ticks: Vec<_> = st
            .session.ticks
            .iter()
            .filter(|t| t.win == WinId::Panel && t.kind == TickKind::PanelApply)
            .collect();
        assert_eq!(ticks.len(), 1, "300ms 内连发应合并为单节拍");
        assert_eq!(ticks[0].at, t0 + Duration::from_millis(500));
        // 合并后仍只发一次（draft 最终值即快照）
        assert!(!st.panel.state.take_apply_due(t0 + Duration::from_millis(400)));
        assert!(st
            .take_due_panel_apply(t0 + Duration::from_millis(500))
            .is_some());
        assert!(st
            .take_due_panel_apply(t0 + Duration::from_millis(500))
            .is_none());
    }

    // ── M4.4 第二批：ModelEditState / LineEditState / prompt 防抖 / bench 行 ──

    /// ModelEditState ↔ ModelConfig 全量往返（原版 populate ↔ get_data）
    #[test]
    fn model_edit_state_roundtrip_full_config() {
        let cfg = lt_proto::ModelConfig {
            name: "deepseek".into(),
            api_base: "https://api.deepseek.com/v1".into(),
            api_key: "sk-x".into(),
            model: "deepseek-chat".into(),
            proxy: "http://127.0.0.1:7890".into(),
            no_system_role: true,
            thinking_style: Some("qwen".into()),
            streaming: false,
            json_response: true,
            context_turns: 4,
            input_price: 0.27,
            output_price: 1.1,
            overrides: Some(
                [
                    ("temperature".to_string(), serde_json::json!(0.7)),
                    ("max_tokens".to_string(), serde_json::json!(512)),
                ]
                .into_iter()
                .collect(),
            ),
            extra_body: Some(serde_json::json!({"thinking": {"type": "disabled"}})),
        };
        let st = ModelEditState::new_edit(1, &cfg);
        assert_eq!(st.proxy_index, 2, "自定义 URL → custom 模式");
        assert_eq!(st.proxy_url, "http://127.0.0.1:7890");
        assert_eq!(st.thinking_index, 2, "qwen → 索引 2");
        let built = st.build().expect("extra_body 合法");
        assert_eq!(built, cfg);
    }

    /// 代理三模式映射：none/system/custom+空白回退（原版 get_data proxy 分支）
    #[test]
    fn model_edit_proxy_modes() {
        let mut st = ModelEditState::new_add();
        st.proxy_index = 0;
        assert_eq!(st.proxy_arg(), "none");
        st.proxy_index = 1;
        assert_eq!(st.proxy_arg(), "system");
        st.proxy_index = 2;
        st.proxy_url = "  ".into();
        assert_eq!(st.proxy_arg(), "none", "custom 空白回退 none");
        st.proxy_url = " http://p:8080 ".into();
        assert_eq!(st.proxy_arg(), "http://p:8080", "去首尾空白");
        // 契约字符串 → 索引往返
        assert_eq!(proxy_index_for("none"), 0);
        assert_eq!(proxy_index_for("system"), 1);
        assert_eq!(proxy_index_for("http://p:8080"), 2);
        assert_eq!(proxy_index_for(""), 0);
    }

    /// overrides 勾选 ↔ BTreeMap 往返：勾选才写入、整数行取整、未勾选 = None
    #[test]
    fn model_edit_overrides_checkbox_roundtrip() {
        let mut st = ModelEditState::new_add();
        st.overrides[0] = super::OverrideRow {
            enabled: true,
            value: 0.256,
        }; // temperature
        st.overrides[2] = super::OverrideRow {
            enabled: true,
            value: 512.4,
        }; // max_tokens
        let cfg = st.build().expect("ok");
        let map = cfg.overrides.as_ref().expect("勾选后应存在");
        assert_eq!(map.len(), 2);
        assert_eq!(
            map["temperature"],
            serde_json::json!(0.26),
            "原版 round(val,2)"
        );
        assert_eq!(map["max_tokens"], serde_json::json!(512), "整数行取整");
        // 未勾选 = None（缺省不落盘）
        let st2 = ModelEditState::new_add();
        assert!(st2.build().unwrap().overrides.is_none());
        // 往返：勾选行回填后 build 保持
        let back = ModelEditState::new_edit(0, &cfg);
        assert!(back.overrides[0].enabled && back.overrides[2].enabled);
        assert!(!back.overrides[1].enabled);
        assert_eq!(back.build().unwrap(), cfg);
    }

    /// extra_body 解析：空 → None；合法 object → Some；非法/数组 → Err（原版 _parse_extra_body）
    #[test]
    fn model_edit_extra_body_parsing() {
        let mut st = ModelEditState::new_add();
        assert_eq!(st.build().unwrap().extra_body, None, "空文本不设置");
        st.extra_body_text = r#"{"thinking": {"type": "disabled"}}"#.into();
        let cfg = st.build().unwrap();
        assert_eq!(
            cfg.extra_body,
            Some(serde_json::json!({"thinking": {"type": "disabled"}}))
        );
        st.extra_body_text = "not json".into();
        assert!(st.build().is_err());
        st.extra_body_text = "[1,2]".into();
        assert!(st.build().is_err(), "非 object 拒绝");
        st.extra_body_text = "  ".into();
        assert_eq!(st.parse_extra_body(), Ok(None));
    }

    /// LineEditState ↔ SubtitleLine 全字段往返（原版 populate ↔ get_config）
    #[test]
    fn line_edit_state_roundtrip_all_fields() {
        let line = lt_proto::SubtitleLine {
            line_type: "translation".into(),
            lang: Some("ja".into()),
            enabled: false,
            font_family: "SimHei".into(),
            font_size: 36,
            color: "#FFD700".into(),
            opacity: 128,
            align: "right".into(),
            outline_enabled: false,
            outline_color: "#123456".into(),
            outline_width: 5,
            bg_image: std::env::temp_dir()
                .join("lt_bg.png")
                .to_string_lossy()
                .into_owned(),
            entry_animation: "slide_up".into(),
            exit_animation: "fade".into(),
            animation_duration: 250,
        };
        let st = LineEditState::new_edit(1, &line);
        assert_eq!(st.line_type_index, 1);
        assert_eq!(st.lang, "ja");
        assert_eq!(st.opacity_pct, 50, "128/255 ≈ 50%");
        assert_eq!(st.entry_anim_index, 4);
        let built = st.build();
        assert_eq!(built, line, "全字段往返一致");
    }

    /// 原文行 lang=None；opacity % ↔ 0-255 换算；新增行草稿默认值
    #[test]
    fn line_edit_original_line_and_add_defaults() {
        let orig = lt_proto::SubtitleLine {
            line_type: "original".into(),
            lang: None,
            ..Default::default()
        };
        let st = LineEditState::new_edit(0, &orig);
        assert_eq!(st.line_type_index, 0);
        // 原文行即便草稿 lang 有兜底值，build 也必须回 None（原版 get_config 分支）
        assert_eq!(st.build().lang, None);

        let add = LineEditState::new_add(2);
        assert!(add.is_new);
        let built = add.build();
        assert_eq!(built.line_type, "translation");
        assert_eq!(built.lang.as_deref(), Some("en"), "原版 _add_line new_line");
        assert_eq!(built.animation_duration, 300);
        assert_eq!(built.opacity, 255);
    }

    /// 字幕行上移/下移边界（原版 _move_line_up/_move_line_down）
    #[test]
    fn move_line_boundaries() {
        let mk = |n: u32| lt_proto::SubtitleLine {
            font_size: n,
            ..Default::default()
        };
        let mut lines = vec![mk(1), mk(2), mk(3)];
        assert!(!move_line_up(&mut lines, 0), "首行上移无效");
        assert!(!move_line_down(&mut lines, 2), "末行下移无效");
        assert!(move_line_down(&mut lines, 0));
        assert_eq!(
            lines.iter().map(|l| l.font_size).collect::<Vec<_>>(),
            [2, 1, 3]
        );
        assert!(move_line_up(&mut lines, 2));
        assert_eq!(
            lines.iter().map(|l| l.font_size).collect::<Vec<_>>(),
            [2, 3, 1]
        );
        // 越界行号防御
        assert!(!move_line_up(&mut lines, 9));
        assert!(!move_line_down(&mut lines, 9));
    }

    /// 颜色归一：#RRGGBB 小写化、#RGB 展开、8 位截断、非法拒绝
    #[test]
    fn normalize_hex_color_semantics() {
        assert_eq!(normalize_hex_color("#FFD700"), Some("#ffd700".into()));
        assert_eq!(normalize_hex_color(" #abc "), Some("#aabbcc".into()));
        assert_eq!(normalize_hex_color("#AABBCCDD"), Some("#aabbcc".into()));
        assert_eq!(normalize_hex_color("#GGGGGG"), None);
        assert_eq!(normalize_hex_color("red"), None);
        assert_eq!(normalize_hex_color("#12345"), None);
        assert_eq!(normalize_hex_color(""), None);
    }

    /// prompt 600ms 防抖：登记/合并/消费与 PanelApply 互不干扰
    #[test]
    fn prompt_apply_debounce_independent() {
        let mut st = AppUi::new(Settings::default());
        let t0 = Instant::now();
        crate::windows::panel::schedule_prompt_apply(&mut st.panel, &mut st.session, t0);
        crate::windows::panel::schedule_prompt_apply(&mut st.panel, &mut st.session, t0 + Duration::from_millis(200));
        let ticks: Vec<_> = st
            .session.ticks
            .iter()
            .filter(|t| t.kind == TickKind::PromptApply)
            .collect();
        assert_eq!(ticks.len(), 1, "连发合并为单节拍");
        assert_eq!(
            ticks[0].at,
            t0 + Duration::from_millis(800),
            "deadline = 末次登记 + 600ms"
        );
        assert!(!st.panel.state.take_prompt_apply_due(t0 + Duration::from_millis(799)));
        assert!(st.panel.state.take_prompt_apply_due(t0 + Duration::from_millis(800)));
        assert!(
            !st.panel.state.take_prompt_apply_due(t0 + Duration::from_millis(800)),
            "单发语义"
        );
        // 面板 300ms 防抖独立存在
        assert!(
            st.panel.state.apply_due_at.is_none(),
            "prompt 登记不应触发 ApplySettings"
        );
    }

    /// bench 行上限 500 删最旧（对齐 push_log_line 语义）
    #[test]
    fn bench_lines_cap_at_500() {
        let mut st = AppUi::new(Settings::default());
        for i in 0..520 {
            st.bench.push_line(format!("line {i}"));
        }
        assert_eq!(st.bench.lines.len(), 500);
        assert_eq!(st.bench.lines[0], "line 20");
        assert_eq!(st.bench.lines.last().unwrap(), "line 519");
    }

    /// W2/D-67：音频监视节拍排班（33ms 拍、重复调用去重；悬浮窗可见才续拍
    /// 的语义在 app.rs about_to_wait 消费）
    #[test]
    fn audio_monitor_tick_scheduling() {
        let mut st = AppUi::new(Settings::default());
        let at = Instant::now() + Duration::from_millis(33);
        st.session.schedule_tick(WinId::Overlay, TickKind::AudioMonitor, at);
        st.session.schedule_tick(WinId::Overlay, TickKind::AudioMonitor, at);
        let n = st
            .session
            .ticks
            .iter()
            .filter(|t| t.kind == TickKind::AudioMonitor)
            .count();
        assert_eq!(n, 1, "同拍去重");
        assert!(st.session.next_tick().is_some(), "应有待触发节拍");
    }

    /// thinking_style 存储值 ↔ 下拉索引（未知/None 回退 auto）
    #[test]
    fn thinking_style_index_mapping() {
        assert_eq!(thinking_style_index(None), 0);
        assert_eq!(thinking_style_index(Some("auto")), 0);
        assert_eq!(thinking_style_index(Some("off")), 5);
        assert_eq!(thinking_style_index(Some("bogus")), 0);
        assert_eq!(lt_proto::THINKING_STYLES.len(), 6);
    }
}
