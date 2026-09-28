//! worker 子进程入口逻辑（原版 asr_worker.py::worker_main 1:1）。
//!
//! 生命周期：stdin 收帧 → 装载期错误发 error(recoverable=false) 后退出 →
//! 循环处理 transcribe / set_language / set_input_padding / shutdown。
//! 单命令执行错误发 error(recoverable=true) 继续；EOF/ shutdown 退出。
//! stdout/stderr 由父进程重定向到日志（E-07：绝不刷 GUI 控制台）。

use crate::engine::AsrEngine;
use crate::frame::{FrameReader, FrameWriter, ReadyInfo, ReqKind, Request, Response};
use std::io::{BufRead, Write};
use std::path::PathBuf;

/// 引擎工厂：由装配层注入（M2.3 SenseVoice / M5 whisper / 测试 echo）
pub type EngineFactory<E> = fn(&WorkerConfig) -> anyhow::Result<E>;

/// worker 级引擎身份（D-116 单源化）：worker 分派/家族/padding 的唯一词表。
///
/// settings 级身份是 `lt_proto::EngineKey`（三变体，funasr 合并两族）；本枚举
/// 是 worker 侧的真身份——funasr 在此拆分为 SenseVoice/Nano。serde 形态
/// （lowercase）与旧 `WorkerConfig.engine: String` 的字符串逐字节一致。
/// Echo 为测试假 worker 专用身份（先例 = `WorkerOptions::Echo`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkerEngine {
    SenseVoice,
    Nano,
    Whisper,
    Qwen3,
    /// 测试假 worker 行为注入专用（fake_asr_worker；生产入口拒绝）
    Echo,
}

impl WorkerEngine {
    /// worker 键字符串（= serde wire 形态）：ReadyInfo/日志沿用旧词表
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SenseVoice => "sensevoice",
            Self::Nano => "nano",
            Self::Whisper => "whisper",
            Self::Qwen3 => "qwen3",
            Self::Echo => "echo",
        }
    }
}

/// worker 启动配置（父进程经参数/环境传入；Rust 版走 stdin 首帧前固定通道，
/// 这里与原版一致：由 spawn 方把配置作为 argv JSON 传入）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkerConfig {
    pub engine: WorkerEngine,
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
/// R28/D-76：配置经 stdin **首行**（`\n` 结尾 JSON）传递——argv 仅旗标，
/// 任务管理器命令行不暴露模型路径。首行由**调用方**读取（fake worker 在
/// 进循环前需读注入参数做 ready 前退出决策），随后把同一 `BufRead`
/// （预读缓冲不丢字节，帧循环从剩余字节继续）与解析出的 config 传入。
pub fn run<E: AsrEngine + 'static>(
    mut stdin: impl BufRead,
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
        engine: config.engine.as_str().to_string(),
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

#[cfg(test)]
mod worker_engine_tests {
    use super::WorkerEngine;

    #[test]
    fn wire_is_lowercase_and_matches_as_str() {
        // C2/D-116：engine 改枚举后 IPC wire 必须与旧字符串逐字节一致
        //（同 exe 父子恒同构建，无跨版兼容负担，但形态要钉死防漂移）；
        // as_str 与 serde 是两处独立编码，此测试钉住二者不漂移。
        for (e, s) in [
            (WorkerEngine::SenseVoice, "sensevoice"),
            (WorkerEngine::Nano, "nano"),
            (WorkerEngine::Whisper, "whisper"),
            (WorkerEngine::Qwen3, "qwen3"),
            (WorkerEngine::Echo, "echo"),
        ] {
            assert_eq!(serde_json::to_string(&e).unwrap(), format!(r#""{s}""#));
            assert_eq!(
                serde_json::from_str::<WorkerEngine>(&format!(r#""{s}""#)).unwrap(),
                e
            );
            assert_eq!(e.as_str(), s);
        }
    }
}
