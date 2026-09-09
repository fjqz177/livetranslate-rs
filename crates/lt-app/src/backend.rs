//! 后台命令线程（M2.5；W3 起只做**命令路由**——下载编排域已迁
//! `lt_orchestrator::DownloadManager`，本文件收敛为：命令收口 + settings
//! 镜像（StartDownload 现场重算缺失清单的数据源）+ 回流事件循环）。
//!
//! 原版对应物：SetupWizardDialog/ModelDownloadDialog 的 `_download_worker`
//! 后台线程 + `_LogCapture`。Rust 版：UI 只发 [`Cmd::StartDownload`]，本线程
//! 起**下载会话线程**（DL-4：下载期间命令线程保持响应，PersistSettings/
//! SwitchEngine 照常消化，设置镜像不再过期），会话内跑 [`Downloader`]
//! （阻塞式专用线程 + 取消令牌），把 Downloader 事件类型化转发为
//! `UiEvent::Download`/`LogLine[download]`；成功发 `DownloadSucceeded`
//! （向导=13 键默认块，原版 `_check_done`），取消发 `DownloadCancelled`
//! （D-23）。落盘权归 UI 侧单写者（DEC-4）：本线程不直接写 settings.json。
//!
//! settings 镜像：拦截 PersistSettings/ApplySettings/SwitchEngine 先更新
//! 本地镜像再照旧转发 UI 循环——StartDownload 据此现场重算缺失清单，运行中
//! 切换的引擎/档位即时生效（启动快照会下错模型）。
//!
//! W2（架构 2.0 §3.3）：四条字符串旁路之一在此收口——`format_event` 的
//! `\t` 机器段协议废除：进度改推类型化 `DownloadEvent`，下载期 tracing
//! 广播行不再经会话泵二次转发（常驻日志桥已把它送日志窗），对话框人读行
//! 由下载域以 `LogLine{target:"download"}` 直接发（保序、与进度事件同源）。

use lt_orchestrator::DownloadManager;
use lt_proto::{AppCommand, Cmd, UiMsg};
use std::sync::mpsc::Receiver;
use winit::event_loop::EventLoopProxy;

/// 启动后台命令线程（进程生命期常驻；通道关闭即退出）。
/// W2 起动脉经参数注入（生产端唯一出口；本线程不再直发 proxy 事件）。
pub fn spawn(
    cmd_rx: Receiver<Cmd>,
    proxy: EventLoopProxy<UiMsg>,
    artery: lt_orchestrator::EventSink,
    first_launch: bool,
    settings: lt_proto::Settings,
) {
    std::thread::Builder::new()
        .name("lt-backend".into())
        .spawn(move || {
            let mut settings = settings;
            let mut download = DownloadManager::new(artery, first_launch);
            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    // 下载编排全在 DownloadManager（非阻塞：会话线程跑下载）
                    Cmd::StartDownload { hub, proxy } => {
                        download.start(&settings, &hub, &proxy);
                    }
                    Cmd::CancelDownload => download.cancel(),
                    // 镜像更新后照旧转发（设置真值在 UI 循环/AppState）
                    Cmd::PersistSettings(s) => {
                        settings = *s;
                        let _ = proxy.send_event(UiMsg::Cmd(Cmd::PersistSettings(Box::new(
                            settings.clone(),
                        ))));
                    }
                    Cmd::ApplySettings(s) => {
                        settings = *s;
                        let _ = proxy
                            .send_event(UiMsg::Cmd(Cmd::ApplySettings(Box::new(settings.clone()))));
                    }
                    Cmd::SwitchEngine {
                        engine,
                        funasr_model,
                        whisper_model_size,
                        hub,
                        language,
                    } => {
                        settings.asr_engine = engine.clone();
                        settings.funasr_model = funasr_model.clone();
                        settings.whisper_model_size = whisper_model_size.clone();
                        settings.hub = hub.clone();
                        settings.asr_language = language.clone();
                        let _ = proxy.send_event(UiMsg::Cmd(Cmd::SwitchEngine {
                            engine,
                            funasr_model,
                            whisper_model_size,
                            hub,
                            language,
                        }));
                    }
                    // 下载对话框失败后的关闭按钮（原版 reject → sys.exit(0)）
                    Cmd::Stop => quit(&proxy),
                    // 管道类命令原样回流事件循环，由 AppShell 分发（持有 Pipeline）
                    other => {
                        let _ = proxy.send_event(UiMsg::Cmd(other));
                    }
                }
            }
        })
        .ok();
}

/// 借托盘退出菜单项触发应用退出（MultiWindowApp::on_command 的 Quit 分支）
fn quit(proxy: &EventLoopProxy<UiMsg>) {
    let _ = proxy.send_event(UiMsg::AppCommand(AppCommand::Quit));
}
