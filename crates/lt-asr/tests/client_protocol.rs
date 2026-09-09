//! M2.1/2.2 完成标准：echo 假 worker 全命令往返 + 崩溃/超时/回收语义。

use lt_asr::client::{AsrClientError, AsrWorkerClient, Status};
use lt_asr::worker::WorkerConfig;
use std::time::Duration;

fn fake_config(name: &str, options: lt_asr::worker::EchoOptions) -> WorkerConfig {
    WorkerConfig {
        engine: "echo".into(),
        language: "auto".into(),
        pad_seconds: Some(0.5),
        options: lt_asr::worker::WorkerOptions::Echo(options),
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
    let mut c = AsrWorkerClient::spawn_program(
        &fake_worker_path(),
        fake_config("Echo", lt_asr::worker::EchoOptions::default()),
    )
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
        fake_config(
            "Crash",
            lt_asr::worker::EchoOptions { crash_on_transcribe: true, ..Default::default() },
        ),
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
        fake_config("Die", lt_asr::worker::EchoOptions { crash_on_ready: true, ..Default::default() }),
    )
    .expect("spawn");
    let err = c.wait_ready().unwrap_err();
    assert!(matches!(err, AsrClientError::Exited(_)), "实际: {err:?}");
}

#[test]
fn transcribe_timeout_terminates_worker() {
    let mut c = AsrWorkerClient::spawn_program(
        &fake_worker_path(),
        fake_config("Hang", lt_asr::worker::EchoOptions { hang_ms: 5000, ..Default::default() }),
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

// ── AH-2/D-26：恢复语义统一 ──

/// worker 在两次请求之间死亡（后台线程延时退出）→ 下一次请求预检按 Exited
/// 上抛（原实现折成 Status"worker 未就绪: Exited"，上层归为用法错误永不恢复）
#[test]
fn request_after_gap_death_reports_exited() {
    let mut c = AsrWorkerClient::spawn_program(
        &fake_worker_path(),
        fake_config("Echo", lt_asr::worker::EchoOptions { exit_after_ms: 500, ..Default::default() }),
    )
    .expect("spawn");
    c.wait_ready().expect("ready");
    assert!(c.transcribe(&sample(0.1), false).is_ok());
    std::thread::sleep(Duration::from_millis(900));
    let err = c.transcribe(&sample(0.1), false).unwrap_err();
    assert!(matches!(err, AsrClientError::Exited(_)), "实际: {err:?}");
}

/// shutdown 的 kill 兜底（H6）：worker 忙（挂 30s）时 ack 窗口（5s）过后
/// 必须杀进程返回，不得无界 child.wait()（否则 UI 线程 join 整体挂死）
#[test]
fn shutdown_kills_worker_hung_in_transcribe() {
    let mut c = AsrWorkerClient::spawn_program(
        &fake_worker_path(),
        fake_config("Hang", lt_asr::worker::EchoOptions { hang_ms: 30_000, ..Default::default() }),
    )
    .expect("spawn");
    c.wait_ready().expect("ready");
    let t0 = std::time::Instant::now();
    c.shutdown();
    assert!(
        t0.elapsed() < Duration::from_secs(8),
        "shutdown 应在 ack 窗口+ε 内返回，实际 {:?}",
        t0.elapsed()
    );
    assert_eq!(c.status(), Status::Stopped);
}
