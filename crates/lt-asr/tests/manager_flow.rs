//! AsrManager 语义测试（真子进程；原版 _run_asr/_recover_asr_worker 行为对照）。

use lt_asr::{AsrClientError, AsrManager, AsrManagerError, AsrPendingHandle, AsrWorkerClient, Spawner, WorkerConfig};
use std::path::PathBuf;

fn fake_spawn() -> Spawner {
    let path = PathBuf::from(env!("CARGO_BIN_EXE_fake_asr_worker"));
    Box::new(move |cfg: &WorkerConfig| AsrWorkerClient::spawn_program(&path, cfg.clone()))
}

/// engine=="bad" 时注入加载失败，其余起真假 worker（切换回滚测试用）
fn switch_fail_spawn() -> Spawner {
    let path = PathBuf::from(env!("CARGO_BIN_EXE_fake_asr_worker"));
    Box::new(move |cfg: &WorkerConfig| {
        if cfg.engine == "bad" {
            return Err(AsrClientError::Status("注入的新引擎加载失败".into()));
        }
        AsrWorkerClient::spawn_program(&path, cfg.clone())
    })
}

/// 恒定失败（首次启动无回滚测试用）
fn failing_spawn() -> Spawner {
    Box::new(|_cfg: &WorkerConfig| Err(AsrClientError::Status("注入的启动失败".into())))
}

/// 指定引擎名的配置（engine_family / 回滚测试用）
fn cfg_engine(engine: &str, name: &str, options: serde_json::Value) -> WorkerConfig {
    WorkerConfig {
        engine: engine.into(),
        display_name: name.into(),
        language: "auto".into(),
        pad_seconds: Some(0.5),
        options,
    }
}

fn cfg(name: &str, options: serde_json::Value) -> WorkerConfig {
    cfg_engine("echo", name, options)
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

// ── pending（原版 _asr_pending_* / _apply_pending_asr_settings） ──

#[test]
fn pending_language_applied_and_committed() {
    let handle = AsrPendingHandle::default();
    let mut m = AsrManager::with_spawner_and_pending(fake_spawn(), handle.clone());
    m.ensure_started(&cfg("Echo", serde_json::json!({}))).expect("start");
    assert_eq!(m.config().unwrap().language, "auto");

    // UI 线程挂起 → ASR 线程 transcribe 前应用并提交（写回 restart config）
    handle.set_language("zh");
    let res = m.transcribe(&audio(), false).expect("transcribe");
    assert_eq!(res.text, "echo len=1600");
    assert_eq!(m.config().unwrap().language, "zh");

    // 挂起已清除且 restart config 已更新：再次挂起别的值同样走"应用→提交"
    handle.set_language("en");
    m.transcribe(&audio(), false).expect("transcribe 2");
    assert_eq!(m.config().unwrap().language, "en");
    m.shutdown();
}

#[test]
fn pending_padding_wrong_family_ignored() {
    let handle = AsrPendingHandle::default();
    let mut m = AsrManager::with_spawner_and_pending(fake_spawn(), handle.clone());
    // sensevoice → funasr 家族：挂起 whisper 家族的 padding 不应影响它
    m.ensure_started(&cfg_engine("sensevoice", "SenseVoice", serde_json::json!({})))
        .expect("start");
    let before = m.config().unwrap().pad_seconds;

    handle.set_padding("whisper", 1.0);
    m.transcribe(&audio(), false).expect("transcribe");
    assert_eq!(m.config().unwrap().pad_seconds, before);
    m.shutdown();
}

/// worker 在 set_language 送达途中死亡：命令未送达 → 挂起保持、不提交，
/// 换正常 worker 后挂起值在其首次 transcribe 前被重新应用（原版
/// "worker-death exceptions propagate with the pending intact"）
#[test]
fn pending_kept_when_worker_dies_during_apply() {
    let handle = AsrPendingHandle::default();
    let mut m = AsrManager::with_spawner_and_pending(fake_spawn(), handle.clone());
    let crash = cfg("CrashLang", serde_json::json!({"fake": {"crash_on_set_language": true}}));
    m.ensure_started(&crash).expect("start");
    handle.set_language("zh");

    // 两次尝试均因 worker 崩溃失败（重启在配额内，非 unavailable）
    for i in 1..=2 {
        let err = m.transcribe(&audio(), false).unwrap_err();
        assert!(!err.unavailable(), "尝试 {i}: {err}");
    }
    // 命令未送达 → 不写回 restart config
    assert_eq!(m.config().unwrap().language, "auto");

    // 挂起保持：切到正常 worker 后首次 transcribe 前被应用并提交
    m.ensure_started(&cfg("Echo", serde_json::json!({}))).expect("switch");
    m.transcribe(&audio(), false).expect("transcribe");
    assert_eq!(m.config().unwrap().language, "zh");
    m.shutdown();
}

// ── 引擎切换失败回滚（原版 _switch_asr_engine._load） ──

#[test]
fn engine_switch_failure_rolls_back() {
    let mut m = AsrManager::with_spawner(switch_fail_spawn());
    let good = cfg_engine("echo", "Echo", serde_json::json!({}));
    m.ensure_started(&good).expect("start");

    // 切到 bad 引擎：加载失败 → 用旧配置恢复旧 worker，manager 仍可用
    let bad = cfg_engine("bad", "Bad", serde_json::json!({}));
    let err = m.ensure_started(&bad).expect_err("切换应失败");
    assert!(!err.unavailable(), "回滚成功后应仍可用（Failed 而非 Unavailable）: {err}");
    assert!(m.is_ready());
    assert_eq!(m.config().unwrap().engine, "echo");
    assert!(m.transcribe(&audio(), false).is_ok());
    m.shutdown();
}

#[test]
fn first_start_failure_has_no_rollback() {
    let mut m = AsrManager::with_spawner(failing_spawn());
    let err = m.ensure_started(&cfg("Echo", serde_json::json!({}))).expect_err("首次启动应失败");
    // 首次启动本就没有旧 worker：仅 Failed，不标记不可用
    assert!(!err.unavailable(), "首次失败应仅 Failed: {err}");
    assert!(!m.is_unavailable());
}
