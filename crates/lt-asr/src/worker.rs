//! worker 子进程入口逻辑（原版 asr_worker.py::worker_main 1:1）。
//!
//! 生命周期：stdin 收帧 → 装载期错误发 error(recoverable=false) 后退出 →
//! 循环处理 transcribe / set_language / set_input_padding / shutdown。
//! 单命令执行错误发 error(recoverable=true) 继续；EOF/ shutdown 退出。
//! stdout/stderr 由父进程重定向到日志（E-07：绝不刷 GUI 控制台）。

use crate::engine::AsrEngine;
use crate::frame::{FrameReader, FrameWriter, ReqKind, Request, Response, ReadyInfo};
use std::io::{Read, Write};

/// 引擎工厂：由装配层注入（M2.3 SenseVoice / M5 whisper / 测试 echo）
pub type EngineFactory<E> = fn(&WorkerConfig) -> anyhow::Result<E>;

/// worker 启动配置（父进程经参数/环境传入；Rust 版走 stdin 首帧前固定通道，
/// 这里与原版一致：由 spawn 方把配置作为 argv JSON 传入）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkerConfig {
    pub engine: String,
    pub display_name: String,
    pub language: String,
    #[serde(default)]
    pub pad_seconds: Option<f32>,
    /// 引擎自定义参数（模型路径等，装配层解释）
    #[serde(default)]
    pub options: serde_json::Value,
}

/// worker 主循环。`stdin/stdout` 为管道；返回即退出进程。
pub fn run<E: AsrEngine + 'static>(
    mut stdin: impl Read,
    mut stdout: impl Write,
    config: WorkerConfig,
    factory: EngineFactory<E>,
) -> anyhow::Result<()> {
    let mut writer = FrameWriter::new(&mut stdout);

    // 装载期：失败 → error(不可恢复) → 退出（对齐原版 load 失败路径）
    let engine = match factory(&config) {
        Ok(e) => e,
        Err(e) => {
            let _ = writer.write_response(&Response::error(
                None,
                format!("模型加载失败: {e:#}"),
                false,
            ));
            return Ok(());
        }
    };
    writer.write_response(&Response::ready(ReadyInfo {
        engine: config.engine.clone(),
        display_name: config.display_name.clone(),
    }))?;

    let mut engine = Some(engine);
    let mut reader = FrameReader::new(&mut stdin);
    while let Some(inc) = reader.read_request()? {
        let Request { id, kind } = inc.request;
        match kind {
            ReqKind::Shutdown => {
                writer.write_response(&Response::shutdown(&id))?;
                break;
            }
            ReqKind::Transcribe { word_timestamps } => {
                let e = engine.as_mut().expect("engine live");
                match e.transcribe(&inc.audio, word_timestamps) {
                    Ok(res) => writer.write_response(&Response::result(&id, res))?,
                    Err(err) => writer.write_response(&Response::error(
                        Some(&id),
                        err.to_string(),
                        err.recoverable(),
                    ))?,
                }
            }
            ReqKind::SetLanguage { language } => {
                let e = engine.as_mut().expect("engine live");
                match e.set_language(&language) {
                    Ok(()) => writer.write_response(&Response::ack(&id))?,
                    Err(err) => writer.write_response(&Response::error(
                        Some(&id),
                        err.to_string(),
                        err.recoverable(),
                    ))?,
                }
            }
            ReqKind::SetInputPadding { pad_seconds } => {
                let e = engine.as_mut().expect("engine live");
                match e.set_input_padding(pad_seconds) {
                    Ok(()) => writer.write_response(&Response::ack(&id))?,
                    Err(err) => writer.write_response(&Response::error(
                        Some(&id),
                        err.to_string(),
                        err.recoverable(),
                    ))?,
                }
            }
        }
    }
    Ok(())
}
