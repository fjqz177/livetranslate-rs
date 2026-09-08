//! AppShell：组合委托 [`lt_ui::MultiWindowApp`] 的 ApplicationHandler。
//!
//! 在用户事件转交 UI 之前做进程级编排（M2.5）：
//! - `DownloadSucceeded` → 启动管道（向导/缺模型流程完成后；原版向导
//!   accept 后经 `_deferred_init` + 500ms `on_start` 的等价时机）；
//! - 退出时统一 `Pipeline::stop` + 设置保存。

use crate::pipeline::{transcript_shared, Pipeline};
use lt_proto::{Cmd, Settings, UiEvent, UiMsg};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};

pub struct AppShell {
    pub ui: lt_ui::MultiWindowApp,
    proxy: EventLoopProxy<UiMsg>,
    pipeline: Option<Pipeline>,
    started: bool,
}

impl AppShell {
    /// `start_settings` 为 Some（启动即就绪）时立即启动管道。
    pub fn new(
        ui: lt_ui::MultiWindowApp,
        proxy: EventLoopProxy<UiMsg>,
        start_settings: Option<Settings>,
    ) -> Self {
        let mut shell = Self {
            ui,
            proxy,
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
        match Pipeline::start(&settings, self.proxy.clone()) {
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
            // 挂起机制：UI 线程只存值，ASR 线程在下一次 transcribe 前应用；
            // AH-3：同步 ASR 线程内段过滤镜像，防过滤仍按启动快照旧值判定
            Cmd::SetAsrLanguage(lang) => {
                if let Some(p) = &self.pipeline {
                    p.set_pending_language(&lang);
                    p.sync_asr_language(&lang);
                }
                self.ui.app_state.settings.asr_language = lang.clone();
                tracing::info!("源语言: {lang}");
                self.persist_settings();
            }
            // padding 挂起热应用（原版 _set_asr_padding）+ 镜像同步（AH-3：
            // 引擎切换装配读取，防切换后静默回退旧值）；settings 字段已由面板写入
            Cmd::SetPadding { engine, secs } => {
                if let Some(p) = &self.pipeline {
                    p.set_pending_padding(&engine, secs);
                    p.sync_padding(&engine, secs);
                }
                tracing::info!("padding 挂起: {engine} {secs}s（下一段识别生效）");
                self.persist_settings();
            }
            // 增量识别热应用（原版 _incremental_asr_cb → _incremental_enabled/
            // _interim_interval）；settings 字段已由面板写入
            Cmd::IncrementalAsr { enabled, interval } => {
                if let Some(p) = &self.pipeline {
                    p.set_interim(enabled, interval);
                }
                tracing::info!("增量识别: {enabled}（间隔 {interval}s）");
                self.persist_settings();
            }
            Cmd::SetTargetLanguage(lang) => {
                if let Some(p) = self.pipeline.as_mut() {
                    p.set_translator_target_language(&lang);
                }
                self.ui.app_state.settings.target_language = lang.clone();
                tracing::info!("目标语言: {lang}");
                self.persist_settings();
            }
            Cmd::SetTimeout(secs) => {
                if let Some(p) = &self.pipeline {
                    p.set_translator_timeout(secs);
                }
                self.ui.app_state.settings.timeout = secs;
                self.persist_settings();
            }
            Cmd::SwitchTranslator(config) => {
                if let Some(p) = self.pipeline.as_mut() {
                    p.switch_translator(&config, &self.ui.app_state.settings);
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
                self.persist_settings();
            }
            Cmd::TestTranslator(config) => {
                if let Some(p) = self.pipeline.as_ref() {
                    p.test_translator(&config);
                }
            }
            Cmd::PersistSettings(settings) => {
                self.ui.app_state.settings = *settings;
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
                self.persist_settings();
            }
            // 面板"应用"全量重放（原版 settings_changed → main 逐项应用）：
            // VAD 参数热更新 + 翻译目标语言/超时 + 转录开关 + 落盘
            Cmd::ApplySettings(s) => {
                let s = *s;
                if let Some(p) = &self.pipeline {
                    // AH-8/D-28：VAD 生效值按当前引擎钳制（qwen3 ≤15s）
                    p.update_vad_settings(crate::pipeline::clamp_vad_for_engine(
                        &s.asr_engine,
                        lt_pipeline::VadSettings {
                            mode: s.vad_mode.clone(),
                            threshold: s.vad_threshold as f64,
                            energy_threshold: s.energy_threshold as f64,
                            min_speech_duration: s.min_speech_duration as f64,
                            max_speech_duration: s.max_speech_duration as f64,
                            silence_mode: s.silence_mode.clone(),
                            silence_duration: s.silence_duration as f64,
                        },
                    ));
                    p.set_translator_target_language(&s.target_language);
                    p.set_translator_timeout(s.timeout);
                    // 增量识别重放（原版 settings_changed 同步 _incremental_enabled/
                    // _interim_interval，main.py:321-323）
                    p.set_interim(s.incremental_asr, s.interim_interval);
                }
                transcript_shared().set_enabled(s.auto_save_transcript);
                self.ui.app_state.settings = s;
                self.persist_settings();
                tracing::info!("设置已应用");
            }
            // 其余命令后续波次接线
            other => tracing::debug!("命令待后续接线: {other:?}"),
        }
    }

    /// 设置落盘（主线程直写；原子写由 settings_io 保证）
    fn persist_settings(&mut self) {
        if let Err(e) = lt_models::settings_io::save(&self.ui.app_state.settings) {
            tracing::error!("设置保存失败: {e:#}");
        }
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
        // 编排先于 UI：下载成功 → 管道启动（UI 转场由 MultiWindowApp 处理）
        if let UiMsg::Event(UiEvent::DownloadSucceeded { settings }) = &event {
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
        // UI 回流的管道域命令（backend 不持有 Pipeline，经 proxy 回环至此）
        if let UiMsg::Cmd(cmd) = &event {
            self.handle_cmd(cmd.clone());
        }
        self.ui.user_event(event_loop, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.ui.about_to_wait(event_loop);
    }
}
