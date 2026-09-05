//! M2.1/2.2 完成标准：echo 假 worker 全命令往返 + 崩溃/超时/回收语义。

use lt_asr::client::{AsrClientError, AsrWorkerClient, Status};
use lt_asr::worker::WorkerConfig;
use std::time::Duration;

fn fake_config(name: &str, options: serde_json::Value) -> WorkerConfig {
    WorkerConfig {
        engine: "echo".into(),
        display_name: name.into(),
        language: "auto".into(),
        pad_seconds: Some(0.5),
        options,
    }
}

fn fake_worker_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_fake_asr_worker"))
}

fn sample(seconds: f64) -> Vec<f32> {
    vec![0.1f32; (16000.0 * seconds) as usize]
}

#[test]
fn full_command_roundtrip() {
    let mut c = AsrWorkerClient::spawn_program(&fake_worker_path(), fake_config("Echo", serde_json::json!({})))
        .expect("spawn");
    assert_eq!(c.status(), Status::Starting);

    let ready = c.wait_ready().expect("ready");
    assert_eq!(ready.engine, "echo");
    assert_eq!(c.status(), Status::Ready);

    let res = c.transcribe(&sample(1.0), false).expect("transcribe");
    assert_eq!(res.text, "echo len=16000");
    assert_eq!(res.language, "zh");

    c.set_language("en").expect("set_language");
    c.set_input_padding(0.3).expect("set_input_padding");
    // 状态仍就绪（ack 不改变状态）
    assert_eq!(c.status(), Status::Ready);

    c.shutdown();
    assert_eq!(c.status(), Status::Stopped);
}

#[test]
fn crash_during_transcribe_reports_exited() {
    let mut c = AsrWorkerClient::spawn_program(
        &fake_worker_path(),
        fake_config("Crash", serde_json::json!({"fake": {"crash_on_transcribe": true}})),
    )
    .expect("spawn");
    c.wait_ready().expect("ready");
    let err = c.transcribe(&sample(0.5), false).unwrap_err();
    assert!(matches!(err, AsrClientError::Exited(_)), "实际: {err:?}");
    assert_eq!(c.status(), Status::Exited);
}

#[test]
fn worker_exit_before_ready_is_error() {
    let mut c = AsrWorkerClient::spawn_program(
        &fake_worker_path(),
        fake_config("Die", serde_json::json!({"fake": {"crash_on_ready": true}})),
    )
    .expect("spawn");
    let err = c.wait_ready().unwrap_err();
    assert!(matches!(err, AsrClientError::Exited(_)), "实际: {err:?}");
}

#[test]
fn transcribe_timeout_terminates_worker() {
    let mut c = AsrWorkerClient::spawn_program(
        &fake_worker_path(),
        fake_config("Hang", serde_json::json!({"fake": {"hang_ms": 5000}})),
    )
    .expect("spawn");
    c.wait_ready().expect("ready");
    c.set_request_timeout(Duration::from_millis(500)); // 挂起 5s 必超时
    let t0 = std::time::Instant::now();
    let err = c.transcribe(&sample(0.1), false).unwrap_err();
    match err {
        AsrClientError::Timeout(t) => assert!((t - 0.5).abs() < 1e-6),
        other => panic!("期望 Timeout，实际 {other:?}"),
    }
    assert!(t0.elapsed() < Duration::from_secs(3), "超时后必须立即回收");
    // 超时路径已 terminate；状态 Failed
    assert_eq!(c.status(), Status::Failed);
}
