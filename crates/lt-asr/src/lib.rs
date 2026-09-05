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
pub use engines::whisper::WhisperEngine;
pub use manager::{AsrManager, AsrManagerError, AsrPendingHandle, Spawner};
pub use engine::AsrEngine;
pub use frame::{ErrorInfo, ReadyInfo, ReqKind, Request, RespKind, Response};
pub use worker::WorkerConfig;
