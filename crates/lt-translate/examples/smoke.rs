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
    // 迭代器的最后一个值即最终译文；用量随迭代器返回（不再经共享态）
    let mut it = translator.translate_iter(text, "en", "zh", 10, 0);
    #[allow(clippy::while_let_on_iterator)]
    // while let 是刻意的：跑完还要读 verdict()/usage()（方案 §4）
    while let Some(item) = it.next() {
        match item {
            Ok(partial) => {
                // 前缀切片必须按字符边界（译文含多字节字符时按字节切会 panic）
                let fresh = partial.strip_prefix(&last).unwrap_or(&partial);
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
    let (pt, ct) = it.usage();
    let usage_note = if it.usage_known() {
        String::new()
    } else {
        "（该端点未返回用量统计）".to_string()
    };
    println!(
        "\n完成：{:.0}ms，usage pt={pt} ct={ct}{usage_note}",
        t0.elapsed().as_secs_f64() * 1000.0
    );
}
