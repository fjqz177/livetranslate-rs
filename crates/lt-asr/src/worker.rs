//! worker 子进程入口逻辑（原版 asr_worker.py::worker_main 1:1）。
//!
//! 生命周期：stdin 收帧 → 装载期错误发 error(recoverable=false) 后退出 →
//! 循环处理 transcribe / set_language / set_input_padding / shutdown。
//! 单命令执行错误发 error(recoverable=true) 继续；EOF/ shutdown 退出。
//! stdout/stderr 由父进程重定向到日志（E-07：绝不刷 GUI 控制台）。

use crate::engine::AsrEngine;
use crate::frame::{FrameReader, FrameWriter, ReadyInfo, ReqKind, Request, Response};
use std::io::{Read, Write};
use std::path::PathBuf;

/// 引擎工厂：由装配层注入（M2.3 SenseVoice / M5 whisper / 测试 echo）
pub type EngineFactory<E> = fn(&WorkerConfig) -> anyhow::Result<E>;

/// worker 启动配置（父进程经参数/环境传入；Rust 版走 stdin 首帧前固定通道，
/// 这里与原版一致：由 spawn 方把配置作为 argv JSON 传入）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkerConfig {
    pub engine: String,
    pub language: String,
    #[serde(default)]
    pub pad_seconds: Option<f32>,
    /// 引擎自定义参数（W6 类型化：装配层构造、worker 工厂解释；替代
    /// serde_json::Value——无 schema 字符串载荷是契约走私同类病灶）
    pub options: WorkerOptions,
}

/// 引擎参数（W6 类型化，替代 `serde_json::Value`）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WorkerOptions {
    /// 模型快照目录（funasr sensevoice/nano 与 qwen3 共用；语言/段长
    /// 经 WorkerConfig 平级字段传递，不混入引擎参数）
    ModelDir(PathBuf),
    /// whisper：本地 GGML 模型文件（builtin 档缓存解析或自定义本地路径）
    ModelPath(PathBuf),
    /// 测试假 worker 行为注入（fake_asr_worker 专用；全字段默认关/0）
    Echo(EchoOptions),
}

/// fake worker 注入参数（AH-9：行为注入覆盖崩溃/失败/挂死/间隙死亡场景）
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EchoOptions {
    #[serde(default)]
    pub crash_on_ready: bool,
    #[serde(default)]
    pub crash_on_transcribe: bool,
    #[serde(default)]
    pub crash_on_set_language: bool,
    #[serde(default)]
    pub fail_load: bool,
    #[serde(default)]
    pub fail_transcribe: bool,
    #[serde(default)]
    pub fail_set_language: bool,
    #[serde(default)]
    pub hang_ms: u64,
    #[serde(default)]
    pub exit_after_ms: u64,
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
