//! 测试用假 ASR worker（echo 引擎 + 可注入故障）。
//!
//! 通过 WorkerConfig.options 注入行为：
//! `{"fake": {"crash_on_ready": true, "crash_on_transcribe": true, "hang_ms": 5000}}`

use lt_asr::engine::AsrEngine;
use lt_asr::frame::{FrameReader, FrameWriter, ReqKind, Request, Response, ReadyInfo};
use lt_asr::worker::WorkerConfig;
use lt_proto::{AsrResult, EngineError};

/// echo 引擎：返回固定文本 + 样本长度
struct EchoEngine {
    hang_ms: u64,
    crash_on_transcribe: bool,
    fail_transcribe: bool,
}

impl AsrEngine for EchoEngine {
    fn transcribe(&mut self, audio: &[f32], _word_ts: bool) -> Result<AsrResult, EngineError> {
        if self.crash_on_transcribe {
            eprintln!("fake worker: 模拟崩溃");
            std::process::abort();
        }
        if self.fail_transcribe {
            return Err(EngineError::Runtime { message: "注入的可恢复失败".into(), recoverable: true });
        }
        if self.hang_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(self.hang_ms));
        }
        Ok(AsrResult {
            text: format!("echo len={}", audio.len()),
            language: "zh".into(),
            language_name: "zh".into(),
            words: None,
        })
    }
    fn set_language(&mut self, _lang: &str) -> Result<(), EngineError> {
        Ok(())
    }
    fn set_input_padding(&mut self, _pad: f32) -> Result<(), EngineError> {
        Ok(())
    }
}

fn opt_bool(options: &serde_json::Value, key: &str) -> bool {
    options.get("fake").and_then(|f| f.get(key)).and_then(|v| v.as_bool()).unwrap_or(false)
}
fn opt_u64(options: &serde_json::Value, key: &str) -> u64 {
    options.get("fake").and_then(|f| f.get(key)).and_then(|v| v.as_u64()).unwrap_or(0)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(cfg_pos) = args.iter().position(|a| a == "--asr-worker") else {
        eprintln!("缺少 --asr-worker 参数");
        std::process::exit(2);
    };
    let config: WorkerConfig = serde_json::from_str(&args[cfg_pos + 1]).expect("配置解析");
    let options = config.options.clone();

    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let mut writer = FrameWriter::new(&mut stdout);

    if opt_bool(&options, "crash_on_ready") {
        // 不发 ready 直接消失（测试 ready 超时/退出路径）
        eprintln!("fake worker: ready 前退出");
        std::process::exit(3);
    }
    writer
        .write_response(&Response::ready(ReadyInfo {
            engine: config.engine.clone(),
            display_name: config.display_name.clone(),
        }))
        .expect("写 ready");

    let engine = EchoEngine {
        hang_ms: opt_u64(&options, "hang_ms"),
        crash_on_transcribe: opt_bool(&options, "crash_on_transcribe"),
        fail_transcribe: opt_bool(&options, "fail_transcribe"),
    };
    let mut engine = Some(engine);
    let mut reader = FrameReader::new(&mut stdin);
    while let Some(inc) = reader.read_request().expect("读请求") {
        let Request { id, kind } = inc.request;
        match kind {
            ReqKind::Shutdown => {
                writer.write_response(&Response::shutdown(&id)).unwrap();
                break;
            }
            ReqKind::Transcribe { word_timestamps } => {
                match engine.as_mut().unwrap().transcribe(&inc.audio, word_timestamps) {
                    Ok(r) => writer.write_response(&Response::result(&id, r)).unwrap(),
                    Err(e) => writer
                        .write_response(&Response::error(Some(&id), e.to_string(), e.recoverable()))
                        .unwrap(),
                }
            }
            ReqKind::SetLanguage { language } => {
                engine.as_mut().unwrap().set_language(&language).unwrap();
                writer.write_response(&Response::ack(&id)).unwrap();
            }
            ReqKind::SetInputPadding { pad_seconds } => {
                engine.as_mut().unwrap().set_input_padding(pad_seconds).unwrap();
                writer.write_response(&Response::ack(&id)).unwrap();
            }
        }
    }
}
