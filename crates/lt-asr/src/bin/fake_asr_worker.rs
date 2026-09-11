//! 测试用假 ASR worker：复用真实 [`lt_asr::worker::run`] 主循环 + echo 引擎工厂
//! （AH-9a，docs/archive/asr-hardening.md H17：此前本文件独立复刻主循环，真实循环零测试
//! 覆盖，且两份循环行为会漂移——如 SetLanguage 错误处理一个是 panic 一个是 error 帧）。
//!
//! 通过 WorkerConfig.options 注入行为：
//! `{"fake": {"crash_on_ready": true, "crash_on_transcribe": true,
//!            "crash_on_set_language": true, "fail_load": true,
//!            "fail_transcribe": true, "fail_set_language": true,
//!            "hang_ms": 5000, "exit_after_ms": 800}}`
//!
//! - `fail_load`：工厂返回 Err → 走真实"装载失败 → error(recoverable=false) 帧"路径；
//! - `exit_after_ms`：后台线程延时退出进程——模拟 worker 在**两次请求之间**死亡
//!   （引擎只在请求内拿到控制权，间隙死亡必须由旁观线程制造，H2 回归用例依赖）。

use lt_asr::engine::AsrEngine;
use lt_asr::worker::{EchoOptions, WorkerConfig, WorkerOptions};
use lt_proto::{AsrResult, EngineError};

/// echo 引擎：返回固定文本 + 样本长度
struct EchoEngine {
    hang_ms: u64,
    crash_on_transcribe: bool,
    crash_on_set_language: bool,
    fail_transcribe: bool,
    fail_set_language: bool,
}

impl EchoEngine {
    fn from_options(options: &EchoOptions) -> Self {
        Self {
            hang_ms: options.hang_ms,
            crash_on_transcribe: options.crash_on_transcribe,
            crash_on_set_language: options.crash_on_set_language,
            fail_transcribe: options.fail_transcribe,
            fail_set_language: options.fail_set_language,
        }
    }
}

impl AsrEngine for EchoEngine {
    fn transcribe(&mut self, audio: &[f32], _word_ts: bool) -> Result<AsrResult, EngineError> {
        if self.crash_on_transcribe {
            eprintln!("fake worker: 模拟崩溃");
            std::process::abort();
        }
        if self.fail_transcribe {
            return Err(EngineError::Runtime {
                message: "注入的可恢复失败".into(),
                recoverable: true,
            });
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
        if self.crash_on_set_language {
            eprintln!("fake worker: set_language 时模拟崩溃");
            std::process::abort();
        }
        if self.fail_set_language {
            // 与真实引擎一致的失败形态：error(recoverable=true) 帧，进程继续
            return Err(EngineError::Runtime {
                message: "注入的 set_language 可恢复失败".into(),
                recoverable: true,
            });
        }
        Ok(())
    }
    fn set_input_padding(&mut self, _pad: f32) -> Result<(), EngineError> {
        Ok(())
    }
}

/// 引擎工厂（EngineFactory = fn 指针）：装载失败走真实 error(recoverable=false) 路径
fn echo_factory(cfg: &WorkerConfig) -> anyhow::Result<EchoEngine> {
    let WorkerOptions::Echo(echo) = &cfg.options else {
        anyhow::bail!("fake worker 应带 WorkerOptions::Echo");
    };
    if echo.fail_load {
        anyhow::bail!("注入的装载失败（fail_load）");
    }
    Ok(EchoEngine::from_options(echo))
}

fn main() {
    // R28/D-76：argv 仅旗标，配置经 stdin 首行（与生产 worker 一致）
    if !std::env::args().any(|a| a == "--asr-worker") {
        eprintln!("缺少 --asr-worker 参数");
        std::process::exit(2);
    }
    let mut stdin = std::io::BufReader::new(std::io::stdin().lock());
    let mut cfg_line = String::new();
    std::io::BufRead::read_line(&mut stdin, &mut cfg_line).expect("读取配置行");
    let config: WorkerConfig = serde_json::from_str(cfg_line.trim()).expect("配置解析");

    let WorkerOptions::Echo(echo) = &config.options else {
        eprintln!("fake worker: 应带 WorkerOptions::Echo");
        std::process::exit(2);
    };
    if echo.crash_on_ready {
        // 不发 ready 直接消失（测试 ready 超时/退出路径）
        eprintln!("fake worker: ready 前退出");
        std::process::exit(3);
    }
    let exit_after_ms = echo.exit_after_ms;
    if exit_after_ms > 0 {
        std::thread::Builder::new()
            .name("fake-exit".into())
            .spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(exit_after_ms));
                eprintln!("fake worker: exit_after_ms 到点，进程退出（模拟请求间隙死亡）");
                std::process::exit(7);
            })
            .expect("后台退出线程");
    }

    if let Err(e) = lt_asr::worker::run(stdin, std::io::stdout().lock(), config, echo_factory) {
        eprintln!("fake worker: 主循环异常退出: {e:#}");
        std::process::exit(1);
    }
}
