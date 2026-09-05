//! lt-asr：AsrEngine trait + worker 子进程协议 + 客户端状态机 + 各引擎实现。

pub mod client;
pub mod engine;
pub mod frame;
#[cfg(windows)]
pub mod job;
pub mod manager;
pub mod sensevoice;
pub mod worker;

pub use client::{AsrClientError, AsrWorkerClient, Status};
pub use manager::{AsrManager, AsrManagerError, Spawner};
pub use engine::AsrEngine;
pub use frame::{ErrorInfo, ReadyInfo, ReqKind, Request, RespKind, Response};
pub use worker::WorkerConfig;
