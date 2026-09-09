//! AsrManager 语义测试（真子进程；原版 _run_asr/_recover_asr_worker 行为对照）。

use lt_asr::{
    AsrClientError, AsrEffectiveSettings, AsrManager, AsrManagerError, AsrWorkerClient, Spawner,
    WorkerConfig,
};
use std::path::PathBuf;

/// 生效快照（W4 总线视图）：language="auto" + 双 pad 0.5（=默认设置透视）
fn eff() -> AsrEffectiveSettings {
    AsrEffectiveSettings {
        language: "auto".into(),
        sensevoice_pad: 0.5,
        whisper_pad: 0.5,
        transcribe_base_secs: 60.0,
        transcribe_per_audio_secs: 0.0,
    }
}

fn eff_with(language: &str, sensevoice_pad: f32, whisper_pad: f32) -> AsrEffectiveSettings {
    AsrEffectiveSettings {
        language: language.into(),
        sensevoice_pad,
        whisper_pad,
        transcribe_base_secs: 60.0,
        transcribe_per_audio_secs: 0.0,
    }
}

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

/// 第 0（初始）与第 3+（复活）次成功、第 1/2 次恒失败
/// （AH-2 H3：recover 重建失败 → 空窗 → 有限重建失败 → unavailable → 复活，全链）
fn flaky_spawn() -> Spawner {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    let path = PathBuf::from(env!("CARGO_BIN_EXE_fake_asr_worker"));
    let calls = Arc::new(AtomicU32::new(0));
    Box::new(move |cfg: &WorkerConfig| {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        // 0=ensure_started 成功；1=recover 重启失败；2=空窗重建失败；3=复活成功
        if n == 1 || n == 2 {
            return Err(AsrClientError::Status("注入的后续启动失败".into()));
        }
        AsrWorkerClient::spawn_program(&path, cfg.clone())
    })
}

/// 指定引擎名的配置（engine_family / 回滚测试用）
fn cfg_engine(engine: &str, _name: &str) -> WorkerConfig {
    WorkerConfig {
        engine: engine.into(),
        language: "auto".into(),
        pad_seconds: Some(0.5),
        options: lt_asr::worker::WorkerOptions::Echo(lt_asr::worker::EchoOptions::default()),
    }
}

fn cfg(name: &str) -> WorkerConfig {
    cfg_engine("echo", name)
}

/// 带行为注入的配置（json! 语义迁移：注入参数类型化为 EchoOptions）
fn cfg_echo(name: &str, echo: lt_asr::worker::EchoOptions) -> WorkerConfig {
    let mut c = cfg_engine("echo", name);
    if let lt_asr::worker::WorkerOptions::Echo(e) = &mut c.options {
        *e = echo;
    }
    c
}

fn audio() -> Vec<f32> {
    vec![0.1f32; 1600]
}

#[test]
fn success_path_resets_and_ready() {
    let mut m = AsrManager::with_spawner(fake_spawn());
    let c = cfg("Echo");
    m.ensure_started(&c).expect("start");
    assert!(m.is_ready());
    let res = m.transcribe(&audio(), false, &eff()).expect("transcribe");
    assert_eq!(res.text, "echo len=1600");
    assert!(!m.is_unavailable());
    m.shutdown();
}

#[test]
fn crash_restarts_then_exhausts_to_unavailable() {
    let mut m = AsrManager::with_spawner(fake_spawn());
    let c = cfg_echo("Crash", lt_asr::worker::EchoOptions { crash_on_transcribe: true, ..Default::default() });
    m.ensure_started(&c).expect("start");

    // 每次识别都崩 worker → 恢复重启；配额 3 次耗尽后不可用
    for call in 1..=4 {
        match m.transcribe(&audio(), false, &eff()) {
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
        m.transcribe(&audio(), false, &eff()),
        Err(AsrManagerError::Unavailable(_))
    ));
}

#[test]
fn engine_switch_revives_unavailable() {
    let mut m = AsrManager::with_spawner(fake_spawn());
    let bad = cfg_echo("Crash", lt_asr::worker::EchoOptions { crash_on_transcribe: true, ..Default::default() });
    m.ensure_started(&bad).expect("start");
    for _ in 0..4 {
        let _ = m.transcribe(&audio(), false, &eff());
    }
    assert!(m.is_unavailable());

    // 换配置（引擎切换语义）→ 复活
    let good = cfg("Echo");
    m.ensure_started(&good).expect("切换后应复活");
    assert!(!m.is_unavailable());
    assert!(m.transcribe(&audio(), false, &eff()).is_ok());
}

#[test]
fn three_consecutive_recoverable_errors_mark_unavailable() {
    let mut m = AsrManager::with_spawner(fake_spawn());
    let c = cfg_echo("Flaky", lt_asr::worker::EchoOptions { fail_transcribe: true, ..Default::default() });
    m.ensure_started(&c).expect("start");
    for n in 1..=3 {
        match m.transcribe(&audio(), false, &eff()) {
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
    let a = cfg("A");
    m.ensure_started(&a).expect("start A");

    let b = cfg("B");
    m.ensure_started(&b).expect("start B");
    // 旧实例已被关闭替换：新实例可用且配置为新签名
    assert!(m.transcribe(&audio(), false, &eff()).is_ok());
}

// ── 生效设置（W4 总线快照；原版 _asr_pending_* / _apply_pending_asr_settings） ──

#[test]
fn effective_language_applied_and_committed() {
    let mut m = AsrManager::with_spawner(fake_spawn());
    m.ensure_started(&cfg("Echo"))
        .expect("start");
    assert_eq!(m.config().unwrap().language, "auto");

    // 总线快照语言 zh → transcribe 前应用并提交（写回 restart config）
    let res = m
        .transcribe(&audio(), false, &eff_with("zh", 0.5, 0.5))
        .expect("transcribe");
    assert_eq!(res.text, "echo len=1600");
    assert_eq!(m.config().unwrap().language, "zh");

    // 快照换成 en：同样走"比对→应用→提交"；快照不变（同值）不重复下发
    m.transcribe(&audio(), false, &eff_with("en", 0.5, 0.5))
        .expect("transcribe 2");
    assert_eq!(m.config().unwrap().language, "en");
    m.shutdown();
}

#[test]
fn effective_padding_wrong_family_ignored() {
    let mut m = AsrManager::with_spawner(fake_spawn());
    // sensevoice → funasr 家族：快照换个 whisper 家族 padding 不应影响它
    m.ensure_started(&cfg_engine("sensevoice", "SenseVoice"))
        .expect("start");
    assert_eq!(m.config().unwrap().pad_seconds, Some(0.5));

    m.transcribe(&audio(), false, &eff_with("auto", 0.5, 3.0))
        .expect("transcribe");
    assert_eq!(m.config().unwrap().pad_seconds, Some(0.5), "whisper pad 不生效");

    // funasr 家族生效值变了 → 应用并提交
    m.transcribe(&audio(), false, &eff_with("auto", 1.5, 3.0))
        .expect("transcribe 2");
    assert_eq!(m.config().unwrap().pad_seconds, Some(1.5));
    m.shutdown();
}

/// worker 在 set_language 送达途中死亡：命令未送达 → 不提交（config 不变），
/// 换正常 worker 后同快照再传 → 比对仍不等 → 重新应用（原版
/// "worker-death exceptions propagate with the pending intact" —— W4 形态：
/// 挂起态即总线快照，本层无易失状态）
#[test]
fn effective_retries_when_worker_dies_during_apply() {
    let mut m = AsrManager::with_spawner(fake_spawn());
    let crash = cfg_echo("CrashLang", lt_asr::worker::EchoOptions {
        crash_on_set_language: true,
        ..Default::default()
    });
    m.ensure_started(&crash).expect("start");

    // 两次尝试均因 worker 崩溃失败（重启在配额内，非 unavailable）
    for i in 1..=2 {
        let err = m
            .transcribe(&audio(), false, &eff_with("zh", 0.5, 0.5))
            .unwrap_err();
        assert!(!err.unavailable(), "尝试 {i}: {err}");
    }
    // 命令未送达 → 不写回 restart config
    assert_eq!(m.config().unwrap().language, "auto");

    // 切到正常 worker（配置替换 config≠快照）→ 首次 transcribe 前应用并提交
    m.ensure_started(&cfg("Echo"))
        .expect("switch");
    m.transcribe(&audio(), false, &eff_with("zh", 0.5, 0.5))
        .expect("transcribe");
    assert_eq!(m.config().unwrap().language, "zh");
    m.shutdown();
}

// ── 引擎切换失败回滚（原版 _switch_asr_engine._load） ──

#[test]
fn engine_switch_failure_rolls_back() {
    let mut m = AsrManager::with_spawner(switch_fail_spawn());
    let good = cfg_engine("echo", "Echo");
    m.ensure_started(&good).expect("start");

    // 切到 bad 引擎：加载失败 → 用旧配置恢复旧 worker，manager 仍可用
    let bad = cfg_engine("bad", "Bad");
    let err = m.ensure_started(&bad).expect_err("切换应失败");
    assert!(
        !err.unavailable(),
        "回滚成功后应仍可用（Failed 而非 Unavailable）: {err}"
    );
    assert!(m.is_ready());
    assert_eq!(m.config().unwrap().engine, "echo");
    assert!(m.transcribe(&audio(), false, &eff()).is_ok());
    m.shutdown();
}

#[test]
fn first_start_failure_has_no_rollback() {
    let mut m = AsrManager::with_spawner(failing_spawn());
    let err = m
        .ensure_started(&cfg("Echo"))
        .expect_err("首次启动应失败");
    // 首次启动本就没有旧 worker：仅 Failed，不标记不可用
    assert!(!err.unavailable(), "首次失败应仅 Failed: {err}");
    assert!(!m.is_unavailable());
}

// ── AH-2/D-26：恢复语义统一（间隙死亡 / 空窗重建 / 装载失败帧 / 送达即提交） ──

/// worker 在**两次请求之间**死亡（exit_after_ms 后台线程退出进程）：
/// 下一次 transcribe 预检发现 → 自动恢复重启（原实现折成 Status"worker 未就绪"
/// 按用法错误上抛，永不重启 → 永久僵尸态）
#[test]
fn crash_between_requests_restarts_on_next_transcribe() {
    let mut m = AsrManager::with_spawner(fake_spawn());
    let c = cfg_echo("Echo", lt_asr::worker::EchoOptions { exit_after_ms: 800, ..Default::default() });
    m.ensure_started(&c).expect("start");
    assert!(m.transcribe(&audio(), false, &eff()).is_ok(), "首次识别应成功");
    std::thread::sleep(std::time::Duration::from_millis(1200));
    // 间隙死亡后的第一次识别：自动恢复（本段丢弃），而非 Failed 死循环
    let err = m.transcribe(&audio(), false, &eff()).unwrap_err();
    assert!(matches!(err, AsrManagerError::Restarted(_)), "实际: {err}");
    assert!(!err.unavailable());
    // 重启后的 worker 正常工作（注：同配置仍带 exit_after_ms，须立即识别）
    assert!(m.transcribe(&audio(), false, &eff()).is_ok());
    m.shutdown();
}

/// recover 重启失败留下的 client=None 空窗：下一次识别有限重建一次，
/// 再失败即标记 unavailable（有界，不每段重试）；换配置可复活
#[test]
fn failed_recover_gap_rebuilds_then_marks_unavailable() {
    let mut m = AsrManager::with_spawner(flaky_spawn());
    let c = cfg_echo("Echo", lt_asr::worker::EchoOptions { exit_after_ms: 800, ..Default::default() });
    m.ensure_started(&c).expect("start");
    assert!(m.transcribe(&audio(), false, &eff()).is_ok());
    std::thread::sleep(std::time::Duration::from_millis(1200));
    // 间隙死亡 → recover → 重启 spawn 失败（注入第 1 次）→ Failed 且 client=None
    let err = m.transcribe(&audio(), false, &eff()).unwrap_err();
    assert!(matches!(err, AsrManagerError::Failed(_)), "实际: {err}");
    assert!(!m.is_unavailable(), "配额未耗尽不应标记不可用");
    // 空窗的下一次识别：有限重建（注入第 2 次失败）→ 标记 unavailable
    let err = m.transcribe(&audio(), false, &eff()).unwrap_err();
    assert!(err.unavailable(), "实际: {err}");
    assert!(m.is_unavailable());
    // 换配置（引擎切换语义）→ 复活（注入第 3 次起成功）
    m.ensure_started(&cfg("Echo2"))
        .expect("复活");
    assert!(m.transcribe(&audio(), false, &eff()).is_ok());
    m.shutdown();
}

/// 装载失败走真实 error(recoverable=false) 帧路径（AH-9a 前 fake 只会 exit
/// 不发帧，此路径零覆盖）：首次启动仅 Failed，不标不可用、无回滚可言
#[test]
fn load_failure_frame_marks_first_start_failed() {
    let mut m = AsrManager::with_spawner(fake_spawn());
    let c = cfg_echo("Echo", lt_asr::worker::EchoOptions { fail_load: true, ..Default::default() });
    let err = m.ensure_started(&c).unwrap_err();
    assert!(!err.unavailable(), "首次装载失败应仅 Failed: {err}");
    assert!(!m.is_unavailable());
}

/// worker 回 set_language 可恢复错误（qwen3 非 auto 语言的日常形态）：
/// 原版"送达即提交"——warn 后仍写回 restart config，识别继续
#[test]
fn set_language_recoverable_fail_still_commits_effective() {
    let mut m = AsrManager::with_spawner(fake_spawn());
    let c = cfg_echo(
        "Echo",
        lt_asr::worker::EchoOptions { fail_set_language: true, ..Default::default() },
    );
    m.ensure_started(&c).expect("start");
    let res = m
        .transcribe(&audio(), false, &eff_with("zh", 0.5, 0.5))
        .expect("transcribe");
    assert_eq!(res.text, "echo len=1600");
    assert_eq!(m.config().unwrap().language, "zh", "生效值应已提交");
    // config 已提交：后续同快照不再重复下发
    assert!(m
        .transcribe(&audio(), false, &eff_with("zh", 0.5, 0.5))
        .is_ok());
    m.shutdown();
}
