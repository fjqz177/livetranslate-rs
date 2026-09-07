//! 下载器集成测试：本地 mock HTTP 服务器覆盖续传/回退/重试全场景（无外部网络依赖）。

use lt_models::download::{DownloadEvent, Downloader, Hub, ProxyMode};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::channel;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// 请求记录（Range 头；None = 无 Range）
#[derive(Default)]
struct Log {
    ranges: std::sync::Mutex<Vec<Option<String>>>,
}

/// 极简单文件 HTTP 服务器：任意路径返回 `body`；
/// 行为由 Range 决定（206 续传 / 416 超限 / 200 全量）。
/// `fail_first`：前 N 次请求返回 500（测重试）。
fn serve(body: &'static [u8], fail_first: usize, log: Arc<Log>) -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = Arc::new(AtomicU32::new(0));
    let h = std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { break };
            let n = served.fetch_add(1, Ordering::SeqCst);
            let range = read_request_range(&mut stream);
            log.ranges.lock().unwrap().push(range);
            let (status, headers, payload): (&str, String, &[u8]);
            if n < fail_first as u32 {
                status = "500 Internal Server Error";
                headers = "Content-Length: 0\r\n".into();
                payload = b"";
                write_response(&mut stream, status, &headers, payload);
                continue;
            }
            let cur = log.ranges.lock().unwrap().len();
            let _ = cur;
            let range_now = log.ranges.lock().unwrap().last().cloned().flatten();
            let start = range_now
                .as_deref()
                .and_then(|r| r.strip_prefix("bytes="))
                .and_then(|r| r.split('-').next())
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);
            match start {
                0 => {
                    status = "200 OK";
                    headers = format!("Content-Length: {}\r\nConnection: close\r\n", body.len());
                    payload = body;
                }
                s if s >= body.len() => {
                    status = "416 Range Not Satisfiable";
                    headers = format!(
                        "Content-Range: bytes */{}\r\nContent-Length: 0\r\nConnection: close\r\n",
                        body.len()
                    );
                    payload = b"";
                }
                s => {
                    status = "206 Partial Content";
                    headers = format!(
                        "Content-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n",
                        s,
                        body.len() - 1,
                        body.len(),
                        body.len() - s
                    );
                    payload = &body[s..];
                }
            }
            write_response(&mut stream, status, &headers, payload);
        }
    });
    (port, h)
}

fn read_request_range(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&buf);
    head.lines()
        .find(|l| l.to_ascii_lowercase().starts_with("range:"))
        .map(|l| l.split_once(':').unwrap().1.trim().to_string())
}

fn write_response(stream: &mut TcpStream, status: &str, headers: &str, body: &[u8]) {
    let _ = write!(stream, "HTTP/1.1 {status}\r\n{headers}\r\n");
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn downloader_at(models: &std::path::Path, port: u16) -> Downloader {
    Downloader::new(models, ProxyMode::None).with_ms_endpoint(format!("http://127.0.0.1:{port}"))
}

fn tmpdir(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("lt_dl_test_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

const BODY: &[u8] = b"0123456789abcdef"; // 16B 假"模型"

#[test]
fn full_download_emits_events_and_writes_file() {
    let log = Arc::new(Log::default());
    let (port, _t) = serve(BODY, 0, log.clone());
    let dir = tmpdir("full");
    let (tx, rx) = channel();

    let out = downloader_at(&dir, port)
        .download_files(Hub::Ms, "iic/Test", &[("model.bin", 1)], &AtomicBool::new(false), Some(&tx))
        .unwrap();

    assert_eq!(std::fs::read(out.join("model.bin")).unwrap(), BODY);
    let events: Vec<_> = rx.try_iter().collect();
    assert!(matches!(events.last(), Some(DownloadEvent::Done { .. })));
    assert!(events.iter().any(|e| matches!(e, DownloadEvent::FileDone { file, .. } if file == "model.bin")));
    // 幂等：第二次调用跳过已存在文件
    let (tx2, rx2) = channel();
    downloader_at(&dir, port).download_files(Hub::Ms, "iic/Test", &[("model.bin", 1)], &AtomicBool::new(false), Some(&tx2)).unwrap();
    let ev2: Vec<_> = rx2.try_iter().collect();
    assert!(ev2.iter().any(|e| matches!(e, DownloadEvent::Log(m) if m.contains("跳过"))));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn resume_from_incomplete_sends_range() {
    let log = Arc::new(Log::default());
    let (port, _t) = serve(BODY, 0, log.clone());
    let dir = tmpdir("resume");
    // 预置 .incomplete：前 3 字节已下
    let snap = dir.join("modelscope").join("models").join("iic--Test").join("snapshots").join("master");
    std::fs::create_dir_all(&snap).unwrap();
    std::fs::write(snap.join("model.bin.incomplete"), &BODY[..3]).unwrap();

    let out = downloader_at(&dir, port).download_files(Hub::Ms, "iic/Test", &[("model.bin", 1)], &AtomicBool::new(false), None).unwrap();

    // 完整文件 + .incomplete 已 rename 消失
    assert_eq!(std::fs::read(out.join("model.bin")).unwrap(), BODY);
    assert!(!snap.join("model.bin.incomplete").exists());
    // 服务器收到 Range: bytes=3-，且回了 206（payload 从 3 开始）
    let ranges = log.ranges.lock().unwrap().clone();
    assert!(ranges.iter().any(|r| r.as_deref() == Some("bytes=3-")), "ranges={ranges:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stale_incomplete_416_restarts_from_zero() {
    let log = Arc::new(Log::default());
    let (port, _t) = serve(BODY, 0, log.clone());
    let dir = tmpdir("stale");
    let snap = dir.join("modelscope").join("models").join("iic--Test").join("snapshots").join("master");
    std::fs::create_dir_all(&snap).unwrap();
    // 陈旧续传：比远端还长
    std::fs::write(snap.join("model.bin.incomplete"), vec![0u8; 64]).unwrap();

    let out = downloader_at(&dir, port).download_files(Hub::Ms, "iic/Test", &[("model.bin", 1)], &AtomicBool::new(false), None).unwrap();
    assert_eq!(std::fs::read(out.join("model.bin")).unwrap(), BODY);
    let ranges = log.ranges.lock().unwrap().clone();
    // 第一次带超限 Range → 416；第二次不带 Range 从头来
    assert!(ranges[0].as_deref().map(|r| r.starts_with("bytes=")).unwrap_or(false));
    assert!(ranges[1].is_none(), "ranges={ranges:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn server_error_retries_with_backoff() {
    let log = Arc::new(Log::default());
    let (port, _t) = serve(BODY, 1, log.clone()); // 第一次 500
    let dir = tmpdir("retry");

    let out = downloader_at(&dir, port).download_files(Hub::Ms, "iic/Test", &[("model.bin", 1)], &AtomicBool::new(false), None).unwrap();
    assert_eq!(std::fs::read(out.join("model.bin")).unwrap(), BODY);
    assert_eq!(log.ranges.lock().unwrap().len(), 2, "恰好重试一次");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn progress_events_throttled_but_final_emitted() {
    let log = Arc::new(Log::default());
    let (port, _t) = serve(BODY, 0, log.clone());
    let dir = tmpdir("prog");
    let (tx, rx) = channel();
    downloader_at(&dir, port).download_files(Hub::Ms, "iic/Test", &[("small.bin", 1)], &AtomicBool::new(false), Some(&tx)).unwrap();
    let events: Vec<_> = rx.try_iter().collect();
    // 文件收尾必发一条 Progress（total=16）
    let got = events.iter().any(|e| matches!(e, DownloadEvent::Progress { done: 16, total: Some(16), .. }));
    assert!(got, "events={events:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 固定状态码服务器（测 404/503 等固定响应的快速失败路径）
fn serve_always(status: &'static str, body: &'static [u8]) -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let h = std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { break };
            let _ = read_request_range(&mut stream);
            let headers = format!("Content-Length: {}\r\nConnection: close\r\n", body.len());
            write_response(&mut stream, status, &headers, body);
        }
    });
    (port, h)
}

/// DL-2/F4 端到端：已存在但低于下限的半截文件 → 不跳过，删除后真实重下
#[test]
fn undersized_existing_file_is_redownloaded() {
    let log = Arc::new(Log::default());
    let (port, _t) = serve(BODY, 0, log.clone());
    let dir = tmpdir("undersize");
    let snap = dir.join("modelscope").join("models").join("iic--Test").join("snapshots").join("master");
    std::fs::create_dir_all(&snap).unwrap();
    // 残留半截终版文件（4B < 下限 10B）
    std::fs::write(snap.join("model.bin"), &BODY[..4]).unwrap();

    let (tx, rx) = channel();
    let out = downloader_at(&dir, port)
        .download_files(Hub::Ms, "iic/Test", &[("model.bin", 10)], &AtomicBool::new(false), Some(&tx))
        .unwrap();

    // 重下为完整 BODY，而非跳过保留半截
    assert_eq!(std::fs::read(out.join("model.bin")).unwrap(), BODY);
    let events: Vec<_> = rx.try_iter().collect();
    assert!(events.iter().any(|e| matches!(e, DownloadEvent::Log(m) if m.contains("尺寸异常"))), "events={events:?}");
    assert!(events.iter().any(|e| matches!(e, DownloadEvent::FileDone { file, .. } if file == "model.bin")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 带外部请求计数的固定响应服务器（取消 / 请求次数断言用）
fn serve_counting(body: &'static [u8], counter: Arc<AtomicU32>) -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let h = std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { break };
            counter.fetch_add(1, Ordering::SeqCst);
            let _ = read_request_range(&mut stream);
            let headers = format!("Content-Length: {}\r\nConnection: close\r\n", body.len());
            write_response(&mut stream, "200 OK", &headers, body);
        }
    });
    (port, h)
}

/// DL-4/D-23：取消令牌预置 → 不发任何请求即返回 [cancel]，.incomplete 续传现场保留
#[test]
fn cancel_before_start_makes_no_requests_and_keeps_incomplete() {
    let counter = Arc::new(AtomicU32::new(0));
    let (port, _t) = serve_counting(BODY, counter.clone());
    let dir = tmpdir("cancel");
    let snap = dir.join("modelscope").join("models").join("iic--Test").join("snapshots").join("master");
    std::fs::create_dir_all(&snap).unwrap();
    std::fs::write(snap.join("model.bin.incomplete"), &BODY[..5]).unwrap();

    let cancel = AtomicBool::new(true);
    let err = downloader_at(&dir, port)
        .download_files(Hub::Ms, "iic/Test", &[("model.bin", 1)], &cancel, None)
        .expect_err("取消应返回错误");
    assert!(err.to_string().contains("[cancel]"), "{err}");
    assert_eq!(counter.load(Ordering::Relaxed), 0, "取消后不得发任何请求");
    assert!(snap.join("model.bin.incomplete").exists(), "续传现场保留");
    assert!(!snap.join("model.bin").exists());
    // 复位令牌 → 同一 .incomplete 断点续传成功（自 5B 起续 Range）
    let log = Arc::new(Log::default());
    let (port2, _t2) = serve(BODY, 0, log.clone());
    let out = downloader_at(&dir, port2)
        .download_files(Hub::Ms, "iic/Test", &[("model.bin", 1)], &AtomicBool::new(false), None)
        .unwrap();
    assert_eq!(std::fs::read(out.join("model.bin")).unwrap(), BODY);
    let ranges = log.ranges.lock().unwrap().clone();
    assert!(ranges.iter().any(|r| r.as_deref() == Some("bytes=5-")), "ranges={ranges:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// DL-2/F5 端到端：404 是永久错误 → 只发 1 次请求立即失败（不再 1/4/16s 退避），
/// 错误串带 [http-404] 前缀（UI 分类契约）。5xx 暂时性重试路径由
/// server_error_retries_with_backoff 覆盖（分类规则见单元测试 fail_kind_*）。
#[test]
fn http_404_fails_fast_without_retries() {
    let (port, _t) = serve_always("404 Not Found", b"");
    let dir = tmpdir("fastfail");
    let t0 = std::time::Instant::now();
    let err = downloader_at(&dir, port)
        .download_files(Hub::Ms, "iic/Test", &[("model.bin", 1)], &AtomicBool::new(false), None)
        .expect_err("404 应失败");
    assert!(err.to_string().contains("[http-404]"), "{err}");
    assert!(t0.elapsed() < std::time::Duration::from_secs(3), "应快速失败，实际 {:?}", t0.elapsed());
    let _ = std::fs::remove_dir_all(&dir);
}
