//! 真端点流式冒烟（M3.3 实机验收；对照原版 LM Studio/DeepSeek 手测路径）。
//!
//! 用法（本地 LM Studio 示例）：
//! ```text
//! LIVETRANSLATE_SMOKE_BASE=http://127.0.0.1:1234/v1 \
//! LIVETRANSLATE_SMOKE_KEY=lm-studio \
//! LIVETRANSLATE_SMOKE_MODEL=<已加载的模型 id> \
//! cargo run -p lt-translate --example smoke
//! ```
//! 预期：译文逐字流出（每收到一个增量打印一段），最后打印用量与耗时。

use std::time::Instant;

use lt_translate::{Translator, TranslatorParams};

fn env_or_panic(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("缺少环境变量 {key}"))
}

fn main() {
    // W4：目标语言/超时逐调用传入（示例固定 zh/10s；生产侧来自设置总线）
    let translator = Translator::new(TranslatorParams {
        api_base: env_or_panic("LIVETRANSLATE_SMOKE_BASE"),
        api_key: env_or_panic("LIVETRANSLATE_SMOKE_KEY"),
        model: env_or_panic("LIVETRANSLATE_SMOKE_MODEL"),
        ..TranslatorParams::default()
    })
    .expect("client 构建失败");

    let text = "The quick brown fox jumps over the lazy dog.";
    let t0 = Instant::now();
    print!("译文: ");
    let mut last = String::new();
    for item in translator.translate_iter(text, "en", "zh", 10) {
        match item {
            Ok(partial) => {
                let fresh = &partial[last.len()..];
                print!("{fresh}");
                use std::io::Write;
                std::io::stdout().flush().ok();
                last = partial;
            }
            Err(e) => {
                eprintln!("\n错误: {e:?}");
                std::process::exit(1);
            }
        }
    }
    let (pt, ct) = translator.last_usage();
    println!(
        "\n完成：{:.0}ms，usage pt={pt} ct={ct}",
        t0.elapsed().as_secs_f64() * 1000.0
    );
}
