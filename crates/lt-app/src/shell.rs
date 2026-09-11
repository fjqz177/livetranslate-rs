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

use lt_orchestrator::{DownloadManager, Msg, Pipeline, Policy, SettingsBus, Supervisor};
use lt_proto::{AppCommand, Cmd, Settings, ThreadRole, UiEvent, UiMsg};

use crate::shell_helpers::{
    pick_file_path, probe_audio_devices, to_bench_model, BenchActiveGuard, FilePickKind,
    ProbeExitGuard,
};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;

/// 命令 → 设置草稿的纯写入面（E1-4，写入点唯一化）：所有携带设置语义的
/// 命令，其草稿写入只发生在本函数——旧实现里 `SetPadding`/`IncrementalAsr`
/// 依赖 UI 发送前预写草稿、shell 只发布（同一命令枚举两种暗规则，照抄
/// 即埋雷）。收敛后 UI 只发命令；`handle_cmd` 分派前先过本函数。纯函数：
/// 只改 Settings，不触管道/总线/落盘（那些留在 handle_cmd 各臂）。
/// pad 的 engine 值域 = `funasr`/`whisper` 两族（与识别页滑杆一一对应，
/// 非 ASR_ENGINES 全集——nano 无 padding 语义，UI 侧不产生该载荷）。
fn apply_settings_side_effects(s: &mut Settings, cmd: &Cmd) {
    match cmd {
        Cmd::SetAsrLanguage(lang) => s.asr_language = lang.clone(),
        Cmd::SetPadding { engine, secs } => match engine.as_str() {
            "funasr" => s.sensevoice_pad_seconds = *secs,
            "whisper" => s.whisper_pad_seconds = *secs,
            other => tracing::warn!("SetPadding 未知 engine 值域: {other}（忽略写入）"),
        },
        Cmd::IncrementalAsr {
            enabled, interval, ..
        } => {
            s.incremental_asr = *enabled;
            s.interim_interval = *interval;
        }
        Cmd::SetTargetLanguage(lang) => s.target_language = lang.clone(),
        // E6/D-81：SetTimeout 契约变体已删（零生产者）——timeout 走 ApplySettings 重放
        Cmd::SwitchTranslator(config) => {
            // 记为当前激活模型（按名匹配；越界/未知名不动 active_model）
            if let Some(idx) = s.models.iter().position(|m| m.name == config.name) {
                s.active_model = idx;
            }
        }
        Cmd::SwitchEngine {
            engine,
            funasr_model,
            whisper_model_size,
            language,
            ..
        } => {
            s.asr_engine = engine.clone();
            s.funasr_model = funasr_model.clone();
            s.whisper_model_size = whisper_model_size.clone();
            s.asr_language = language.clone();
        }
        Cmd::SetAudioDevice(choice) => s.audio_device = choice.clone().into(),
        Cmd::SetMicDevice(choice) => s.mic_device = choice.clone().into(),
        Cmd::ApplySettings(new) | Cmd::PersistSettings(new) => *s = (**new).clone(),
        _ => {}
    }
}

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
    /// 编排域监督器（app 侧实例，W5：设备探测/基准/文件对话框一次性线程
    /// 的出生点——Policy::Never 死亡仅上报；动脉桥/日志桥同源）
    sup: std::sync::Arc<Supervisor>,
    /// 在途基准取消标志（W5f：Cmd::CancelBench 置位；RunBench 新会话重置）
    bench_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 基准在途标志（W5 收口 P3：RunBench 防重入——UI 侧 bench.running 之外的
    /// 纵深防御；基准线程退出（含 panic unwind）经 [`BenchActiveGuard`] 复位）
    bench_active: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 在途连接测试的取消标志（D-85：`Cmd::CancelTranslatorTest` 置位；
    /// 每次启动新探测换成新实例——被取代的旧探测自行察觉）
    probe_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 在途连接测试的号（D-85：0 = 无在途；取消/退出守卫按号比对）
    probe_id: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// 编排域文案注入句柄（D-85：探测的降级说明经它取；构造一次全生命周期复用）
    msg: Msg,
    pipeline: Option<Pipeline>,
    started: bool,
}

impl AppShell {
    /// `start_settings` 为 Some（启动即就绪）时立即启动管道。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ui: lt_ui::MultiWindowApp,
        artery: lt_orchestrator::EventSink,
        monitor_cell: std::sync::Arc<arc_swap::ArcSwap<lt_proto::MonitorSample>>,
        cmd_rx: std::sync::mpsc::Receiver<Cmd>,
        proxy: winit::event_loop::EventLoopProxy<UiMsg>,
        sup: std::sync::Arc<Supervisor>,
        start_settings: Option<Settings>,
    ) -> Self {
        // 总线先于一切读者就绪：初值 = 当前设置（pipeline None 期间的
        // 发布同样有效——StartDownload 等经总线读）
        let bus = std::sync::Arc::new(SettingsBus::new(start_settings.clone().unwrap_or_default()));
        // 下载编排（W3 起在 orchestrator；first_launch 恒 false——首启直进
        // 主界面 D-19，无向导下载会话）；W7：会话线程经本监督器出生
        //（INV3——Death 可见，不再裸 spawn）
        let download = DownloadManager::new(artery.clone(), false, sup.clone());
        let mut shell = Self {
            ui,
            artery,
            monitor_cell,
            bus,
            cmd_rx,
            proxy,
            download,
            sup,
            bench_cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            bench_active: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            probe_cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            probe_id: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            msg: Msg::new(lt_i18n::t, lt_i18n::get_lang),
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
        // D-85：句柄在 new 时构造一次（探测可能先于管道启动发生），此处只克隆
        let msg = self.msg.clone();
        match Pipeline::start(&self.bus, self.artery.clone(), &self.monitor_cell, msg) {
            Ok(p) => self.pipeline = Some(p),
            Err(e) => {
                // P0-3：装配失败必须让用户看见——面板识别页顶部红字（数据
                // 源在本字段；细节经日志文件/日志页可查），不再只进日志
                let msg = format!("{e:#}");
                tracing::error!("管道启动失败: {msg}");
                self.ui.app_state.session.pipeline_error = Some(msg);
            }
        }
    }

    /// 事件循环退出后的统一收尾（main 调用）
    pub fn shutdown(&mut self) {
        // D-85：先叫停在途连接测试——否则 join_all 要等它跑完一次尝试
        self.probe_cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
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
        // E1-4：设置写入点唯一——分派前先过纯写入面，各臂只剩管道动作/
        // 日志/发布/落盘（臂内不再直接改 settings）
        apply_settings_side_effects(&mut self.ui.app_state.settings, &cmd);
        match cmd {
            Cmd::Pause => {
                if let Some(p) = self.pipeline.as_mut() {
                    p.set_paused(true);
                }
                self.ui.app_state.session.running = false;
                if let Some(t) = &self.ui.tray {
                    t.set_status(lt_ui::tray::IconStatus::Pause);
                }
                tracing::info!("管道暂停");
            }
            Cmd::Resume => {
                if let Some(p) = self.pipeline.as_mut() {
                    p.set_paused(false);
                }
                self.ui.app_state.session.running = true;
                if let Some(t) = &self.ui.tray {
                    t.set_status(lt_ui::tray::IconStatus::Run);
                }
                tracing::info!("管道恢复");
            }
            // W4：语言/目标/超时/padding 全部走总线发布（INV7）——草稿写入
            // 已在 apply_settings_side_effects 完成；ASR 线程每段 load().asr_lang，
            // 与 worker 的 delta 应用由 Manager 内部比对执行
            Cmd::SetAsrLanguage(lang) => {
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
            // _interim_interval）；草稿写入已过纯写入面
            Cmd::IncrementalAsr { enabled, interval } => {
                if let Some(p) = &self.pipeline {
                    p.set_interim(enabled, interval);
                }
                tracing::info!("增量识别: {enabled}（间隔 {interval}s）");
                self.publish_settings();
                self.persist_settings();
            }
            Cmd::SetTargetLanguage(lang) => {
                tracing::info!("目标语言: {lang}");
                self.publish_settings();
                self.persist_settings();
            }
            // E6/D-81：Cmd::SetTimeout 处理臂随契约变体删除（零生产者，
            // 超时改动经 ApplySettings 整体重放）
            Cmd::SwitchTranslator(config) => {
                // D-85/F4：管道未装配时不得静默——设置改了、装置没换是"界面与实际
                // 不一致"的典型形态；翻译页红字复用既有的 TranslatorUnavailable 形态
                match self.pipeline.as_mut() {
                    Some(p) => {
                        p.switch_translator(&config);
                        tracing::info!("Switching translator: {} ({})", config.name, config.model);
                    }
                    None => {
                        tracing::warn!(
                            "管道未启动，模型切换未生效：{} ({})",
                            config.name,
                            config.model
                        );
                        self.artery.push(UiEvent::TranslatorUnavailable {
                            reason: lt_i18n::t("translator_switch_no_pipeline"),
                        });
                    }
                }
                self.publish_settings();
                self.persist_settings();
            }
            // D-85：连接测试不再经 ASR 线程——组合根起一次性监督线程跑探测
            Cmd::TestTranslator { config, probe_id } => self.start_probe(*config, probe_id),
            Cmd::CancelTranslatorTest { probe_id } => {
                let in_flight = self.probe_id.load(std::sync::atomic::Ordering::SeqCst);
                if crate::shell_helpers::probe_cancel_matches(in_flight, probe_id) {
                    self.probe_cancel
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    tracing::info!("已请求中断连接测试 #{probe_id}");
                } else {
                    tracing::debug!("忽略过期的连接测试取消请求 #{probe_id}（在途 #{in_flight}）");
                }
            }
            Cmd::PersistSettings(_settings) => {
                // E1-4：草稿整体替换已过纯写入面，本臂只剩发布 + 落盘
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
                // W4：引擎切换必须先发布（总线按新引擎重算 VAD 钳制 overlay——
                // 切离 qwen3 立即恢复全值，R18 根除"等用户下次应用"）
                self.publish_settings();
                self.persist_settings();
            }
            Cmd::SetAudioDevice(choice) => {
                if let Some(p) = self.pipeline.as_mut() {
                    p.set_audio_device(choice);
                }
                self.publish_settings();
                self.persist_settings();
            }
            Cmd::SetMicDevice(choice) => {
                if let Some(p) = self.pipeline.as_mut() {
                    p.set_mic_device(choice);
                }
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
                self.publish_settings();
                self.persist_settings();
                tracing::info!("设置已应用");
            }
            // 下载编排（W3 起在 DownloadManager；targets 读设置总线当前
            // raw——运行中切换引擎/档位后下载即所选模型，M5.1）。
            // E2/D-79：载荷已是值域枚举，直传不解析
            Cmd::StartDownload { hub, proxy } => {
                let raw = self.bus.load().raw.clone();
                self.download.start(&raw, hub, proxy);
            }
            Cmd::CancelDownload => self.download.cancel(),
            // 下载对话框失败后的关闭按钮（原版 reject → sys.exit(0)）：
            // 经 AppCommand::Quit 走 UI 既有退出确认流（drain 循环短路处理）
            Cmd::Stop => {
                tracing::info!("命令 Cmd::Stop → 请求退出");
            }
            // ── W5 下沉三路 ──
            // 设备探测（R13）：Supervisor 一次性线程（Policy::Never）——
            // UI 帧内不再阻塞 COM 枚举；结果经动脉 Devices 回执
            Cmd::RefreshDevices => self.spawn_device_probe(),
            // 性能基准（R22）：Supervisor 一次性线程跑 run_benchmark
            Cmd::RunBench {
                models,
                src,
                tgt,
                timeout,
                prompt,
            } => self.start_bench(models, src, tgt, timeout, prompt),
            Cmd::CancelBench => {
                self.bench_cancel
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                tracing::info!("取消基准（模型边界截停）");
            }
            // rfd 同步对话框移离事件循环线程（R19）：Supervisor 一次性线程弹框
            Cmd::PickExportFile {
                mode,
                default_name,
                dialog_title,
            } => self.spawn_file_dialog(
                dialog_title,
                default_name,
                FilePickKind::Export,
                Box::new(move |path| UiEvent::ExportSave { mode, path }),
            ),
            Cmd::PickBgImage { dialog_title } => self.spawn_file_dialog(
                dialog_title,
                String::new(),
                FilePickKind::BgImage,
                Box::new(move |path| UiEvent::BgImagePicked { path }),
            ),
        }
    }

    /// 设备探测一次性线程（W5/R13）：COM 枚举面自库内 init（WasapiBackend::new
    /// 自带 COM init——探测器运行在独立线程，不污染 UI 线程 COM 状态）
    fn spawn_device_probe(&self) {
        let artery = self.artery.clone();
        self.sup.spawn(
            ThreadRole::DeviceProbe,
            "lt-device-probe",
            Policy::Never,
            move || {
                let artery = artery.clone();
                Box::new(move || {
                    artery.push(UiEvent::Devices(probe_audio_devices()));
                })
            },
        );
    }

    /// 性能基准一次性线程（W5/R22）：ModelConfig → BenchModel 转换在此
    /// （随迁自 lt-ui——UI 侧不再直持 lt-translate 基准执行）；取消经
    /// `bench_cancel` 在模型边界轮询。防重入（W5 收口 P3）：标准在途时
    /// 拒绝新会话（UI 侧 bench.running 之外的纵深防御——连发命令不再叠
    /// 线程）；拒绝时旧基准的取消标志不受影响。
    /// 连接测试一次性线程（D-85）：`Cmd::TestTranslator` → 监督器出生
    /// （`Policy::Never`）→ `lt_orchestrator::probe::run_probe` → 回执经动脉回流。
    ///
    /// **取代语义（不是拒绝）**：已有在途时先置位**旧探测**的取消标志再启动新的
    /// ——旧线程退出可滞后一个轮询周期（流式 ≤150ms），拒绝会把"中断后立刻重测"
    /// 变成最长一次尝试超时的空等。旧探测的迟到回执因 `probe_id` 不符必然被 UI 丢弃。
    /// 簿记本身是 [`crate::shell_helpers::begin_probe_slot`]（纯函数，直接可测）。
    fn start_probe(&mut self, config: lt_proto::ModelConfig, probe_id: u64) {
        let cancel = crate::shell_helpers::begin_probe_slot(
            &self.probe_id,
            &mut self.probe_cancel,
            probe_id,
        );
        let artery = self.artery.clone();
        let bus = self.bus.clone();
        let msg = self.msg.clone();
        let id_cell = self.probe_id.clone();
        self.sup.spawn(
            ThreadRole::TranslatorProbe,
            "lt-probe",
            Policy::Never,
            move || {
                // factory 为 Fn：每次构造干净的运行闭包（INV5）
                let artery = artery.clone();
                let bus = bus.clone();
                let cancel = cancel.clone();
                let msg = msg.clone();
                let id_cell = id_cell.clone();
                let config = config.clone();
                Box::new(move || {
                    // 任何退出路径（含 panic unwind）按号条件复位——被取代时
                    // 不得把新探测的号抹掉
                    let _guard = ProbeExitGuard {
                        id: id_cell,
                        probe_id,
                    };
                    lt_orchestrator::probe::run_probe(
                        probe_id, &config, &bus, cancel, &msg, &artery,
                    );
                })
            },
        );
    }

    fn start_bench(
        &mut self,
        models: Vec<lt_proto::ModelConfig>,
        src: String,
        tgt: String,
        timeout: u32,
        prompt: String,
    ) {
        if self
            .bench_active
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            tracing::warn!("基准已在途，忽略重复 RunBench（模型边界截停协议不叠加）");
            return;
        }
        self.bench_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let bench_models: Vec<lt_translate::bench::BenchModel> =
            models.iter().map(to_bench_model).collect();
        let artery = self.artery.clone();
        let cancel = self.bench_cancel.clone();
        let bench_active = self.bench_active.clone();
        self.sup
            .spawn(ThreadRole::Bench, "lt-bench", Policy::Never, move || {
                // factory 为 Fn（死亡可重生）：每次构造干净的运行闭包（INV5）
                let artery = artery.clone();
                let bench_models = bench_models.clone();
                let src = src.clone();
                let tgt = tgt.clone();
                let prompt = prompt.clone();
                let cancel = cancel.clone();
                let bench_active = bench_active.clone();
                Box::new(move || {
                    // 任何退出路径（含 panic unwind——监督器上报）都复位在途标志
                    let _guard = BenchActiveGuard(bench_active);
                    lt_translate::bench::run_benchmark(
                        bench_models,
                        &src,
                        &tgt,
                        timeout,
                        &prompt,
                        cancel,
                        move |out| {
                            let ev = match out {
                                lt_translate::bench::BenchOutput::Line(l) => {
                                    UiEvent::Bench(lt_proto::BenchEvent::Line(l))
                                }
                                lt_translate::bench::BenchOutput::Finished { ok, elapsed_ms } => {
                                    UiEvent::Bench(lt_proto::BenchEvent::Finished {
                                        ok,
                                        elapsed_ms,
                                    })
                                }
                            };
                            artery.push(ev);
                        },
                    );
                })
            });
    }

    /// 文件对话框一次性线程（W5/R19）：rfd 同步 Dialog 的阻塞面被隔离到该
    /// 线程（事件循环线程零阻塞）；`make_event` 把选中路径（None=取消）装配
    /// 为回流事件
    fn spawn_file_dialog(
        &self,
        dialog_title: String,
        default_name: String,
        kind: FilePickKind,
        make_event: Box<dyn Fn(Option<String>) -> UiEvent + Send + Sync>,
    ) {
        let artery = self.artery.clone();
        let make_event: std::sync::Arc<dyn Fn(Option<String>) -> UiEvent + Send + Sync> =
            make_event.into();
        self.sup.spawn(
            ThreadRole::FileDialog,
            "lt-file-dialog",
            Policy::Never,
            move || {
                // factory 为 Fn（死亡可重生）：每次构造干净运行闭包（INV5）
                let artery = artery.clone();
                let make_event = make_event.clone();
                let dialog_title = dialog_title.clone();
                let default_name = default_name.clone();
                Box::new(move || {
                    artery.push(make_event(pick_file_path(
                        &dialog_title,
                        &default_name,
                        kind,
                    )));
                })
            },
        );
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
                let _ = self.proxy.send_event(UiMsg::AppCommand(AppCommand::Quit));
                continue;
            }
            self.handle_cmd(cmd);
        }
        self.ui.about_to_wait(event_loop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// E1-4：SetPadding 写入点唯一——纯写入面从载荷直接落草稿
    ///（旧实现该命令载荷被 shell 丢弃，生效依赖 UI 发送前预写）
    #[test]
    fn set_padding_writes_draft_from_payload() {
        let mut s = Settings::default();
        apply_settings_side_effects(
            &mut s,
            &Cmd::SetPadding {
                engine: "funasr".into(),
                secs: 1.5,
            },
        );
        assert_eq!(s.sensevoice_pad_seconds, 1.5);
        apply_settings_side_effects(
            &mut s,
            &Cmd::SetPadding {
                engine: "whisper".into(),
                secs: 2.0,
            },
        );
        assert_eq!(s.whisper_pad_seconds, 2.0);
        assert_eq!(s.sensevoice_pad_seconds, 1.5, "另一族 pad 不得被串写");
    }

    /// E1-4：IncrementalAsr 同法——enabled/interval 都从载荷落草稿
    #[test]
    fn incremental_asr_writes_draft_from_payload() {
        let mut s = Settings::default();
        apply_settings_side_effects(
            &mut s,
            &Cmd::IncrementalAsr {
                enabled: true,
                interval: 3.0,
            },
        );
        assert!(s.incremental_asr);
        assert_eq!(s.interim_interval, 3.0);
    }

    /// E1-4：未知 pad engine 值域拒绝写入（防新值域静默串写）
    #[test]
    fn set_padding_unknown_engine_is_ignored() {
        let mut s = Settings::default();
        apply_settings_side_effects(
            &mut s,
            &Cmd::SetPadding {
                engine: "qwen3".into(),
                secs: 9.0,
            },
        );
        assert_eq!(s.sensevoice_pad_seconds, 0.5, "默认值不动");
        assert_eq!(s.whisper_pad_seconds, 0.5);
    }

    /// E1-4：SwitchTranslator 按名记激活模型；未知名不动 active_model
    #[test]
    fn switch_translator_records_active_model_by_name() {
        let mut s = Settings::default();
        let second = lt_proto::ModelConfig {
            name: "second".into(),
            ..Default::default()
        };
        s.models.push(second);
        let cfg = s.models[1].clone();
        apply_settings_side_effects(&mut s, &Cmd::SwitchTranslator(Box::new(cfg)));
        assert_eq!(s.active_model, 1);
        let ghost = lt_proto::ModelConfig {
            name: "ghost".into(),
            ..Default::default()
        };
        apply_settings_side_effects(&mut s, &Cmd::SwitchTranslator(Box::new(ghost)));
        assert_eq!(s.active_model, 1, "未知名不得改激活模型");
    }
}
