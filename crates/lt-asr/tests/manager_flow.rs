//! AsrManager 语义测试（真子进程；原版 _run_asr/_recover_asr_worker 行为对照）。

use lt_asr::{AsrManager, AsrManagerError, AsrWorkerClient, Spawner, WorkerConfig};
use std::path::PathBuf;

fn fake_spawn() -> Spawner {
    let path = PathBuf::from(env!("CARGO_BIN_EXE_fake_asr_worker"));
    Box::new(move |cfg: &WorkerConfig| AsrWorkerClient::spawn_program(&path, cfg.clone()))
}

fn cfg(name: &str, options: serde_json::Value) -> WorkerConfig {
    WorkerConfig {
        engine: "echo".into(),
        display_name: name.into(),
        language: "auto".into(),
        pad_seconds: Some(0.5),
        options,
    }
}

fn audio() -> Vec<f32> {
    vec![0.1f32; 1600]
}

#[test]
fn success_path_resets_and_ready() {
    let mut m = AsrManager::with_spawner(fake_spawn());
    let c = cfg("Echo", serde_json::json!({}));
    m.ensure_started(&c).expect("start");
    assert!(m.is_ready());
    let res = m.transcribe(&audio(), false).expect("transcribe");
    assert_eq!(res.text, "echo len=1600");
    assert!(!m.is_unavailable());
    m.shutdown();
}

#[test]
fn crash_restarts_then_exhausts_to_unavailable() {
    let mut m = AsrManager::with_spawner(fake_spawn());
    let c = cfg("Crash", serde_json::json!({"fake": {"crash_on_transcribe": true}}));
    m.ensure_started(&c).expect("start");

    // 每次识别都崩 worker → 恢复重启；配额 3 次耗尽后不可用
    for call in 1..=4 {
        match m.transcribe(&audio(), false) {
            Err(e) => {
                if call < 4 {
                    assert!(!e.unavailable(), "call{call} 应还在配额内: {e}");
                } else {
                    assert!(e.unavailable(), "call4 应耗尽配额: {e}");
                }
            }
            Ok(_) => panic!("call{call} 不应成功"),
        }
    }
    assert!(m.is_unavailable());
    // 耗尽后不再尝试 spawn
    assert!(matches!(
        m.transcribe(&audio(), false),
        Err(AsrManagerError::Unavailable(_))
    ));
}

#[test]
fn engine_switch_revives_unavailable() {
    let mut m = AsrManager::with_spawner(fake_spawn());
    let bad = cfg("Crash", serde_json::json!({"fake": {"crash_on_transcribe": true}}));
    m.ensure_started(&bad).expect("start");
    for _ in 0..4 {
        let _ = m.transcribe(&audio(), false);
    }
    assert!(m.is_unavailable());

    // 换配置（引擎切换语义）→ 复活
    let good = cfg("Echo", serde_json::json!({}));
    m.ensure_started(&good).expect("切换后应复活");
    assert!(!m.is_unavailable());
    assert!(m.transcribe(&audio(), false).is_ok());
}

#[test]
fn three_consecutive_recoverable_errors_mark_unavailable() {
    let mut m = AsrManager::with_spawner(fake_spawn());
    let c = cfg("Flaky", serde_json::json!({"fake": {"fail_transcribe": true}}));
    m.ensure_started(&c).expect("start");
    for n in 1..=3 {
        match m.transcribe(&audio(), false) {
            Err(AsrManagerError::Failed(msg)) => {
                if n == 3 {
                    // 第 3 次连续错误 → 致命 → 不可用
                    assert!(m.is_unavailable(), "n={n} msg={msg}");
                } else {
                    assert!(!m.is_unavailable());
                }
            }
            other => panic!("n={n} 期望 Failed，实际 {other:?}"),
        }
    }
}

#[test]
fn config_change_replaces_worker() {
    let mut m = AsrManager::with_spawner(fake_spawn());
    let a = cfg("A", serde_json::json!({}));
    m.ensure_started(&a).expect("start A");

    let b = cfg("B", serde_json::json!({}));
    m.ensure_started(&b).expect("start B");
    // 旧实例已被关闭替换：新实例可用且配置为新签名
    assert!(m.transcribe(&audio(), false).is_ok());
}
