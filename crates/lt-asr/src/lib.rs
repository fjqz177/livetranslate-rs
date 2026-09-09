//! lt-asr：AsrEngine trait + worker 子进程协议 + 客户端状态机 + 各引擎实现。

pub mod client;
pub mod engine;
pub mod engines;
pub mod frame;
#[cfg(windows)]
pub mod job;
pub mod manager;
pub mod sensevoice;
pub mod worker;

pub use client::{AsrClientError, AsrWorkerClient, Status};
pub use engine::AsrEngine;
pub use engines::nano::NanoEngine;
pub use engines::profile::transcribe_timeout_profile as engine_timeout_profile;
pub use engines::qwen3::Qwen3AsrEngine;
pub use engines::whisper::WhisperEngine;
pub use frame::{ErrorInfo, ReadyInfo, ReqKind, Request, RespKind, Response};
pub use manager::{AsrEffectiveSettings, AsrManager, AsrManagerError, Spawner};
pub use worker::{EchoOptions, WorkerConfig, WorkerOptions};
