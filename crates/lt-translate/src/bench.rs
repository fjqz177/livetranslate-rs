//! 多模型流式基准（对照原版 benchmark.py）：
//! 每模型 5 轮固定句对，测 TTFT 与总耗时；流式失败回退非流式（TTFT=总耗时）；
//! 结果按平均 TTFT 排名输出（格式与原版逐行一致）。

use std::time::Instant;

use serde_json::json;

use crate::translator::make_openai_client;

pub const BENCH_SENTENCES: &[(&str, [&str; 5])] = &[
    (
        "ja",
        [
            "こんにちは、今日はいい天気ですね。",
            "この映画はとても面白かったです。",
            "明日の会議は何時からですか？",
            "日本の桜は本当に美しいですね。",
            "新しいレストランに行ってみましょう。",
        ],
    ),
    (
        "en",
        [
            "Hello, the weather is nice today.",
            "That movie was really interesting.",
            "What time does tomorrow's meeting start?",
            "The cherry blossoms in Japan are truly beautiful.",
            "Let's try going to the new restaurant.",
        ],
    ),
    (
        "zh",
        [
            "你好，今天天气真不错。",
            "那部电影真的很有意思。",
            "明天的会议几点开始？",
            "日本的樱花真的很美丽。",
            "我们去试试那家新餐厅吧。",
        ],
    ),
    (
        "ko",
        [
            "안녕하세요, 오늘 날씨가 좋네요.",
            "그 영화 정말 재미있었어요.",
            "내일 회의는 몇 시부터인가요?",
            "일본의 벚꽃은 정말 아름답네요.",
            "새로운 레스토랑에 가볼까요?",
        ],
    ),
    (
        "fr",
        [
            "Bonjour, il fait beau aujourd'hui.",
            "Ce film était vraiment intéressant.",
            "À quelle heure commence la réunion demain?",
            "Les cerisiers en fleurs au Japon sont magnifiques.",
            "Allons essayer le nouveau restaurant.",
        ],
    ),
    (
        "de",
        [
            "Hallo, heute ist schönes Wetter.",
            "Der Film war wirklich interessant.",
            "Um wie viel Uhr beginnt das Meeting morgen?",
            "Die Kirschblüten in Japan sind wunderschön.",
            "Lass uns das neue Restaurant ausprobieren.",
        ],
    ),
];

/// 参与基准的模型（字段对齐原版 model dict 的基准所需子集）
#[derive(Debug, Clone)]
pub struct BenchModel {
    pub name: String,
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    pub proxy: String,
    pub no_system_role: bool,
}

/// 单轮结果：(ttft_ms, total_ms, 结果文本前 60 字符由输出层截取)
pub type BenchRound = (f64, f64, String);

#[derive(Debug, Clone, Default)]
pub struct BenchResult {
    pub name: String,
    pub avg_ttft: f64,
    pub std_ttft: f64,
    pub avg_total: f64,
    pub std_total: f64,
    pub rounds: Vec<BenchRound>,
    pub error: Option<String>,
}

fn sentences_for(source_lang: &str) -> &'static [&'static str; 5] {
    BENCH_SENTENCES
        .iter()
        .find(|(k, _)| *k == source_lang)
        .map_or_else(
            || {
                BENCH_SENTENCES
                    .iter()
                    .find(|(k, _)| *k == "en")
                    .map(|(_, v)| v)
                    .expect("en 句组常量必存在")
            },
            |(_, v)| v,
        )
}

/// 样本标准差（statistics.stdev：n-1 分母）；单样本为 0
fn stdev(xs: &[f64]) -> f64 {
    if xs.len() < 2 {
        return 0.0;
    }
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    var.sqrt()
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// 单模型全轮次测试；成功返回逐轮 (ttft, total, text)，失败返回错误信息。
/// timeout_s 语义同原版 httpx per-op 超时：每次流式读/整个非流式响应 ≤ timeout_s。
fn test_model(
    m: &BenchModel,
    sentences: &[&str],
    timeout_s: u32,
    prompt: &str,
) -> Result<Vec<BenchRound>, String> {
    let client = make_openai_client(&m.api_base, &m.api_key, &m.proxy).map_err(|e| e.to_string())?;
    let read_timeout = std::time::Duration::from_secs(timeout_s as u64);
    let mut rounds = Vec::new();
    for text in sentences {
        let messages = if m.no_system_role {
            json!([{ "role": "user", "content": format!("{prompt}\n{text}") }])
        } else {
            json!([
                {"role": "system", "content": prompt},
                {"role": "user", "content": text},
            ])
        };
        let streaming_body = json!({
            "model": m.model,
            "messages": messages,
            "max_tokens": 256,
            "temperature": 0.3,
            "stream": true,
        });
        let t0 = Instant::now();
        let streamed = crate::runtime().block_on(async {
            tokio::time::timeout(
                read_timeout,
                client.chat().create_stream_byot::<serde_json::Value, async_openai::types::chat::CreateChatCompletionStreamResponse>(
                    streaming_body.clone(),
                ),
            )
            .await
        });
        let outcome: Result<BenchRound, String> = match streamed {
            Ok(Ok(mut s)) => {
                use futures::StreamExt;
                let mut ttft: Option<f64> = None;
                let mut chunks = Vec::new();
                loop {
                    let next = crate::runtime().block_on(async {
                        tokio::time::timeout(read_timeout, s.next()).await
                    });
                    match next {
                        Ok(Some(Ok(chunk))) => {
                            if ttft.is_none() {
                                ttft = Some(t0.elapsed().as_secs_f64() * 1000.0);
                            }
                            if let Some(delta) = chunk
                                .choices
                                .first()
                                .and_then(|c| c.delta.content.as_deref())
                                .filter(|d| !d.is_empty())
                            {
                                chunks.push(delta.to_string());
                            }
                        }
                        Ok(Some(Err(e))) => break Err(e.to_string()),
                        Ok(None) => {
                            let total_ms = t0.elapsed().as_secs_f64() * 1000.0;
                            break Ok((ttft.unwrap_or(total_ms), total_ms, chunks.join("").trim().to_string()));
                        }
                        Err(_) => break Err(format!("timed out after {timeout_s}s")),
                    }
                }
            }
            // 流式被拒/超时 → 非流式兜底（TTFT=总耗时，原版语义）
            Ok(Err(_)) | Err(_) => {
                let plain = json!({
                    "model": m.model,
                    "messages": messages,
                    "max_tokens": 256,
                    "temperature": 0.3,
                    "stream": false,
                });
                crate::runtime()
                    .block_on(async {
                        tokio::time::timeout(
                            read_timeout,
                            client.chat().create_byot::<serde_json::Value, async_openai::types::chat::CreateChatCompletionResponse>(plain),
                        )
                        .await
                    })
                    .map_err(|_| format!("timed out after {timeout_s}s"))
                    .and_then(|r| r.map_err(|e| e.to_string()))
                    .map(|resp| {
                        let total_ms = t0.elapsed().as_secs_f64() * 1000.0;
                        let text = resp
                            .choices
                            .first()
                            .and_then(|c| c.message.content.clone())
                            .unwrap_or_default()
                            .trim()
                            .to_string();
                        (total_ms, total_ms, text)
                    })
            }
        };
        match outcome {
            Ok(round) => rounds.push(round),
            Err(e) => {
                // 截断规则与原版一致（str(e) 首行、120 字符）
                let first_line = e.lines().next().unwrap_or("");
                return Err(first_line.chars().take(120).collect());
            }
        }
    }
    Ok(rounds)
}

/// 阻塞版基准：每模型一个线程并行测试，返回全部结果（供测试/自定义 UI 复用）。
pub fn run_benchmark_blocking(
    models: &[BenchModel],
    source_lang: &str,
    timeout_s: u32,
    prompt: &str,
) -> Vec<BenchResult> {
    let sentences: Vec<&str> = sentences_for(source_lang).to_vec();
    let handles: Vec<_> = models
        .iter()
        .map(|m| {
            let m = m.clone();
            let sentences = sentences.clone();
            let prompt = prompt.to_string();
            std::thread::spawn(move || {
                let mut r = BenchResult { name: m.name.clone(), ..Default::default() };
                match test_model(&m, &sentences, timeout_s, &prompt) {
                    Ok(rounds) => {
                        let (ttfts, totals): (Vec<f64>, Vec<f64>) = rounds
                            .iter()
                            .map(|(t, tot, _)| (*t, *tot))
                            .unzip();
                        r.rounds = rounds;
                        r.avg_ttft = mean(&ttfts);
                        r.std_ttft = stdev(&ttfts);
                        r.avg_total = mean(&totals);
                        r.std_total = stdev(&totals);
                    }
                    Err(e) => r.error = Some(e),
                }
                r
            })
        })
        .collect();
    handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
}

/// 后台线程版基准（对照原版 run_benchmark）：逐行回调输出，最后输出 "__DONE__"。
#[allow(clippy::too_many_arguments)]
pub fn run_benchmark<F>(
    models: Vec<BenchModel>,
    source_lang: &str,
    target_lang: &str,
    timeout_s: u32,
    prompt: &str,
    on_line: F,
) -> std::thread::JoinHandle<()>
where
    F: Fn(&str) + Send + Sync + 'static,
{
    let source_lang = source_lang.to_string();
    let target_lang = target_lang.to_string();
    let prompt = prompt.to_string();
    let rounds = sentences_for(&source_lang).len();
    std::thread::spawn(move || {
        on_line(&format!(
            "Testing {} model(s) x {rounds} rounds  |  timeout={timeout_s}s  |  {} -> {}\n{}",
            models.len(),
            source_lang,
            target_lang,
            "=".repeat(60),
        ));

        // 每模型一个线程（原版 ThreadPoolExecutor(max_workers=len(models))），
        // 测完即输出该模型明细行（提交顺序）
        let handles: Vec<_> = models
            .iter()
            .cloned()
            .map(|m| {
                let sentences: Vec<&str> = sentences_for(&source_lang).to_vec();
                let prompt = prompt.clone();
                std::thread::spawn(move || {
                    let mut lines = vec![format!("Model: {}", m.name), format!("  {}", "─".repeat(50))];
                    match test_model(&m, &sentences, timeout_s, &prompt) {
                        Ok(rounds) => {
                            for (i, (ttft, total, text)) in rounds.iter().enumerate() {
                                let preview: String = text.chars().take(60).collect();
                                lines.push(format!(
                                    "  Round {}: {:7.0}ms (TTFT {:6.0}ms) | {preview}",
                                    i + 1,
                                    total,
                                    ttft
                                ));
                            }
                            let (ttfts, totals): (Vec<f64>, Vec<f64>) =
                                rounds.iter().map(|(t, tot, _)| (*t, *tot)).unzip();
                            lines.push(format!(
                                "  Avg: {:.0}ms \u{b1} {:.0}ms  (TTFT: {:.0}ms \u{b1} {:.0}ms)",
                                mean(&totals),
                                stdev(&totals),
                                mean(&ttfts),
                                stdev(&ttfts),
                            ));
                            let mut r = BenchResult { name: m.name.clone(), ..Default::default() };
                            r.avg_ttft = mean(&ttfts);
                            r.std_ttft = stdev(&ttfts);
                            r.avg_total = mean(&totals);
                            r.std_total = stdev(&totals);
                            (lines, Some(r))
                        }
                        Err(e) => {
                            lines.push(format!("  FAILED: {e}"));
                            let mut r = BenchResult { name: m.name.clone(), ..Default::default() };
                            r.error = Some(e);
                            (lines, Some(r))
                        }
                    }
                })
            })
            .collect();
        let mut results = Vec::new();
        for h in handles {
            if let Ok((lines, r)) = h.join() {
                for l in &lines {
                    on_line(l);
                }
                if let Some(r) = r {
                    results.push(r);
                }
            }
        }

        results.sort_by(|a, b| a.avg_ttft.partial_cmp(&b.avg_ttft).unwrap_or(std::cmp::Ordering::Equal));
        on_line(&format!("\n{}", "=".repeat(60)));
        on_line("Ranking by Avg TTFT:");
        for (i, r) in (1..).zip(results.iter().filter(|r| r.error.is_none())) {
            on_line(&format!(
                "  #{i}  TTFT {:6.0}ms \u{b1} {:4.0}ms  Total {:6.0}ms \u{b1} {:4.0}ms  {}",
                r.avg_ttft, r.std_ttft, r.avg_total, r.std_total, r.name
            ));
        }
        for r in results.iter().filter(|r| r.error.is_some()) {
            on_line(&format!("  FAIL  {}: {}", r.name, r.error.clone().unwrap_or_default()));
        }
        on_line("__DONE__");
    })
}
