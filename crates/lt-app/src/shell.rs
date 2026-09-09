//! AppShell：组合委托 [`lt_ui::MultiWindowApp`] 的 ApplicationHandler。
//!
//! 在用户事件转交 UI 之前做进程级编排（M2.5）：
//! - `DownloadSucceeded` → 启动管道（向导/缺模型流程完成后；原版向导
//!   accept 后经 `_deferred_init` + 500ms `on_start` 的等价时机）；
//! - 退出时统一 `Pipeline::stop` + 设置保存。
//!
//! W4（架构 2.0 §3.2.3/INV7）：本文件持有设置总线——**唯一写者**（winit 主
//! 线程）：凡修改 `ui.app_state.settings` 的命令处理完毕即 `publish`（发布
//! 幂等重算全部派生视图，ApplySettings 重放与专用快捷命令殊途同归）；
//! 读者（capture/ASR/翻译池/下载）任意线程 `load()` 无锁。
//!
//! W4（INV2）：lt-backend 线程退役——UI→mpsc→shell 两跳（旧五跳），
//! cmd mpsc 在 `about_to_wait` 直排（UI 与 shell 同在 winit 线程，无竞序）；
//! 下载编排由本文件持有的 [`DownloadManager`] 接管（会话线程跑下载）。

use lt_orchestrator::{DownloadManager, Msg, Pipeline, SettingsBus};
use lt_proto::{AppCommand, Cmd, Settings, UiEvent, UiMsg};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;

pub struct AppShell {
    pub ui: lt_ui::MultiWindowApp,
    /// 事件动脉（全后台→UI 事件出口；W4 起 UiMsg::Cmd 回环已不存在，
    /// 控制面 = cmd_rx 直排）
    artery: lt_orchestrator::EventSink,
    /// 音频监视快照格（W2：capture 写侧句柄——传给 Pipeline，UI 读侧句柄
    /// 已在 MultiWindowApp）
    monitor_cell: std::sync::Arc<arc_swap::ArcSwap<lt_proto::MonitorSample>>,
    /// 设置总线（W4：唯一写者=本线程；发布后 capture/ASR/翻译/下载读格）
    bus: std::sync::Arc<SettingsBus>,
    /// UI 命令通道（W4 起本线程直排——UI 与 shell 同在 winit 线程）
    cmd_rx: std::sync::mpsc::Receiver<Cmd>,
    /// 事件循环代理（`Cmd::Stop` 经 AppCommand::Quit 走 UI 退出确认流）
    proxy: winit::event_loop::EventLoopProxy<UiMsg>,
    /// 下载编排（会话线程化；设置读总线——经 `bus.load().raw`）
    download: DownloadManager,
    pipeline: Option<Pipeline>,
    started: bool,
}

impl AppShell {
    /// `start_settings` 为 Some（启动即就绪）时立即启动管道。
    pub fn new(
        ui: lt_ui::MultiWindowApp,
        artery: lt_orchestrator::EventSink,
        monitor_cell: std::sync::Arc<arc_swap::ArcSwap<lt_proto::MonitorSample>>,
        cmd_rx: std::sync::mpsc::Receiver<Cmd>,
        proxy: winit::event_loop::EventLoopProxy<UiMsg>,
        start_settings: Option<Settings>,
    ) -> Self {
        // 总线先于一切读者就绪：初值 = 当前设置（pipeline None 期间的
        // 发布同样有效——StartDownload 等经总线读）
        let bus = std::sync::Arc::new(SettingsBus::new(
            start_settings.clone().unwrap_or_default(),
        ));
        // 下载编排（W3 起在 orchestrator；first_launch 恒 false——首启直进
        // 主界面 D-19，无向导下载会话）
        let download = DownloadManager::new(artery.clone(), false);
        let mut shell = Self {
            ui,
            artery,
            monitor_cell,
            bus,
            cmd_rx,
            proxy,
            download,
            pipeline: None,
            started: false,
        };
        if let Some(s) = start_settings {
            shell.start_pipeline(s);
        }
        shell
    }

    fn start_pipeline(&mut self, settings: Settings) {
        if self.started {
            return;
        }
        self.started = true;
        // INV7：先发布后装配（管道启动读总线当前快照；重复发布幂等）
        self.bus.publish(settings);
        // i18n 文案经 Msg 注入编排域（白名单不变量：orchestrator 零 lt-i18n 依赖）
        let msg = Msg::new(|k| lt_i18n::t(k));
        match Pipeline::start(&self.bus, self.artery.clone(), &self.monitor_cell, msg) {
            Ok(p) => self.pipeline = Some(p),
            Err(e) => {
                // P0-3：装配失败必须让用户看见——面板识别页顶部红字（数据
                // 源在本字段；细节经日志文件/日志页可查），不再只进日志
                let msg = format!("{e:#}");
                tracing::error!("管道启动失败: {msg}");
                self.ui.app_state.pipeline_error = Some(msg);
            }
        }
    }

    /// 事件循环退出后的统一收尾（main 调用）
    pub fn shutdown(&mut self) {
        if let Some(p) = self.pipeline.as_mut() {
            p.stop();
        }
        if let Err(e) = lt_models::settings_io::save(&self.ui.app_state.settings) {
            tracing::error!("设置保存失败: {e}");
        }
    }

    /// W4/INV7：设置发布（winit 主线程唯一写者）——重算全部派生视图，
    /// 各读线程下一访问点即见（无镜像同步边，无"记得补同步"）
    fn publish_settings(&mut self) {
        let s = self.ui.app_state.settings.clone();
        self.bus.publish(s);
    }

    /// 管道域命令分发（对照原版 App 的 overlay/托盘信号处理段）
    fn handle_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Pause => {
                if let Some(p) = self.pipeline.as_mut() {
                    p.set_paused(true);
                }
                self.ui.app_state.running = false;
                if let Some(t) = &self.ui.tray {
                    t.set_status(lt_ui::tray::IconStatus::Pause);
                }
                tracing::info!("管道暂停");
            }
            Cmd::Resume => {
                if let Some(p) = self.pipeline.as_mut() {
                    p.set_paused(false);
                }
                self.ui.app_state.running = true;
                if let Some(t) = &self.ui.tray {
                    t.set_status(lt_ui::tray::IconStatus::Run);
                }
                tracing::info!("管道恢复");
            }
            // W4：语言/目标/超时/padding 全部走总线发布（INV7）——旧挂起机制
            // （pending + 镜像同步命令）退役：ASR 线程每段 load().asr_lang，
            // 与 worker 的 delta 应用由 Manager 内部比对执行
            Cmd::SetAsrLanguage(lang) => {
                self.ui.app_state.settings.asr_language = lang.clone();
                tracing::info!("源语言: {lang}");
                self.publish_settings();
                self.persist_settings();
            }
            Cmd::SetPadding { engine, secs } => {
                tracing::info!("padding: {engine} {secs}s（下一段识别生效）");
                self.publish_settings();
                self.persist_settings();
            }
            // 增量识别热应用（原版 _incremental_asr_cb → _incremental_enabled/
            // _interim_interval）；settings 字段已由面板写入
            Cmd::IncrementalAsr { enabled, interval } => {
                if let Some(p) = &self.pipeline {
                    p.set_interim(enabled, interval);
                }
                tracing::info!("增量识别: {enabled}（间隔 {interval}s）");
                self.publish_settings();
                self.persist_settings();
            }
            Cmd::SetTargetLanguage(lang) => {
                self.ui.app_state.settings.target_language = lang.clone();
                tracing::info!("目标语言: {lang}");
                self.publish_settings();
                self.persist_settings();
            }
            Cmd::SetTimeout(secs) => {
                self.ui.app_state.settings.timeout = secs;
                self.publish_settings();
                self.persist_settings();
            }
            Cmd::SwitchTranslator(config) => {
                if let Some(p) = self.pipeline.as_mut() {
                    p.switch_translator(&config);
                }
                // 记为当前激活模型并持久化
                if let Some(idx) = self
                    .ui
                    .app_state
                    .settings
                    .models
                    .iter()
                    .position(|m| m.name == config.name)
                {
                    self.ui.app_state.settings.active_model = idx;
                }
                tracing::info!("Switching translator: {} ({})", config.name, config.model);
                self.publish_settings();
                self.persist_settings();
            }
            Cmd::TestTranslator(config) => {
                if let Some(p) = self.pipeline.as_ref() {
                    p.test_translator(&config);
                }
            }
            Cmd::PersistSettings(settings) => {
                self.ui.app_state.settings = *settings;
                self.publish_settings();
                if let Err(e) = lt_models::settings_io::save(&self.ui.app_state.settings) {
                    tracing::error!("设置保存失败: {e:#}");
                }
            }
            Cmd::SwitchEngine {
                engine,
                funasr_model,
                whisper_model_size,
                hub: _,
                language,
            } => {
                if let Some(p) = self.pipeline.as_ref() {
                    p.switch_engine(&engine, &funasr_model, &whisper_model_size, &language);
                }
                let s = &mut self.ui.app_state.settings;
                s.asr_engine = engine;
                s.funasr_model = funasr_model;
                s.whisper_model_size = whisper_model_size;
                s.asr_language = language;
                // W4：引擎切换必须先发布（总线按新引擎重算 VAD 钳制 overlay——
                // 切离 qwen3 立即恢复全值，R18 根除"等用户下次应用"）
                self.publish_settings();
                self.persist_settings();
            }
            Cmd::SetAudioDevice(choice) => {
                let dev = match &choice {
                    lt_proto::AudioDeviceChoice::SystemDefault => None,
                    lt_proto::AudioDeviceChoice::Named(n) => Some(n.clone()),
                    lt_proto::AudioDeviceChoice::Disabled => Some("__disabled__".into()),
                };
                if let Some(p) = self.pipeline.as_mut() {
                    p.set_audio_device(choice);
                }
                self.ui.app_state.settings.audio_device = dev;
                self.publish_settings();
                self.persist_settings();
            }
            Cmd::SetMicDevice(choice) => {
                let dev = match &choice {
                    lt_proto::MicDeviceChoice::Off => None,
                    lt_proto::MicDeviceChoice::Default => Some("__default__".into()),
                    lt_proto::MicDeviceChoice::Named(n) => Some(n.clone()),
                };
                if let Some(p) = self.pipeline.as_mut() {
                    p.set_mic_device(choice);
                }
                self.ui.app_state.settings.mic_device = dev;
                self.publish_settings();
                self.persist_settings();
            }
            // 面板"应用"全量重放（原版 settings_changed → main 逐项应用）：
            // W4 起 VAD/目标语言/超时不再逐项进 Pipeline——publish 统一派生
            //（VAD 钳制已含在总线 overlay 中，本处只留真实动作：增量/转录开关）
            Cmd::ApplySettings(s) => {
                let s = *s;
                if let Some(p) = &self.pipeline {
                    // 增量识别重放（原版 settings_changed 同步 _incremental_enabled/
                    // _interim_interval，main.py:321-323）
                    p.set_interim(s.incremental_asr, s.interim_interval);
                    // 转录写盘开关（W3 起经 Pipeline 方法——句柄真源收敛为
                    // Pipeline 字段，不再经旧 transcript_shared 全局单例）
                    p.set_transcript_enabled(s.auto_save_transcript);
                }
                self.ui.app_state.settings = s;
                self.publish_settings();
                self.persist_settings();
                tracing::info!("设置已应用");
            }
            // 下载编排（W3 起在 DownloadManager；targets 读设置总线当前
            // raw——运行中切换引擎/档位后下载即所选模型，M5.1）
            Cmd::StartDownload { hub, proxy } => {
                let raw = self.bus.load().raw.clone();
                self.download.start(&raw, &hub, &proxy);
            }
            Cmd::CancelDownload => self.download.cancel(),
            // 下载对话框失败后的关闭按钮（原版 reject → sys.exit(0)）：
            // 经 AppCommand::Quit 走 UI 既有退出确认流（drain 循环短路处理）
            Cmd::Stop => {
                tracing::info!("命令 Cmd::Stop → 请求退出");
            }
        }
    }

    /// 设置落盘（主线程直写；原子写由 settings_io 保证）
    fn persist_settings(&mut self) {
        if let Err(e) = lt_models::settings_io::save(&self.ui.app_state.settings) {
            tracing::error!("设置保存失败: {e:#}");
        }
    }

    /// 单条业务事件分发（用户事件 → 编排 → UI；与 Events 批量共用）
    fn dispatch_event(&mut self, event_loop: &ActiveEventLoop, event: UiEvent) {
        // 编排先于 UI：下载成功 → 管道启动（UI 转场由 MultiWindowApp 处理）
        if let UiEvent::DownloadSucceeded { settings } = &event {
            if !self.started {
                self.start_pipeline((**settings).clone());
            } else {
                // 运行时下载完成（原版 _download_whisper accept 后 _auto_save →
                // 引擎切换）：以当前设置重发引擎切换，worker 用刚下载的模型装配，
                // 同时解除未缓存切换时的 AsrUnavailable 待命态
                let s = self.ui.app_state.settings.clone();
                self.handle_cmd(Cmd::SwitchEngine {
                    engine: s.asr_engine,
                    funasr_model: s.funasr_model,
                    whisper_model_size: s.whisper_model_size,
                    hub: s.hub,
                    language: s.asr_language,
                });
            }
        }
        self.ui.user_event(event_loop, UiMsg::Event(event));
    }
}

impl ApplicationHandler<UiMsg> for AppShell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.ui.resumed(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        self.ui.window_event(event_loop, window_id, event);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UiMsg) {
        match event {
            // 事件动脉批量变体（W2）：逐条分发（事件序 = 动脉 FIFO，
            // 同生产者保序，INV8——批次切分不破坏序）
            UiMsg::Events(events) => {
                for ev in events {
                    self.dispatch_event(event_loop, ev);
                }
            }
            // Event/AppCommand 原样转 UI（W4：UiMsg::Cmd 变体已删——
            // 控制面 = cmd_rx 在 about_to_wait 直排）
            msg => self.ui.user_event(event_loop, msg),
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // W4/INV2：cmd mpsc 直排（控制面五跳→两跳）——UI 与 shell 同在
        // winit 线程，try_recv 非阻塞批量排空；`Cmd::Stop` 经 proxy 发
        // AppCommand::Quit 走 UI 既有退出确认流（约 wait 里无 proxy 可用
        // 于 ActiveEventLoop，故 shell 持构造期代理）
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            if let Cmd::Stop = cmd {
                let _ = self
                    .proxy
                    .send_event(UiMsg::AppCommand(AppCommand::Quit));
                continue;
            }
            self.handle_cmd(cmd);
        }
        self.ui.about_to_wait(event_loop);
    }
}
