//! TranscriptWriter（对照原版 transcript_writer.py 1:1）：
//! 每会话三个文件（original / translation / all），识别即追加（行缓冲），
//! 内存消息轮换不丢内容。msg_id 挂起机制保证 all 文件的"原文→译文"配对。

use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// 挂起条目：msg_id → (时间戳, 原文)，译文到达时配对写入 all
struct Pending {
    timestamp: String,
    original: String,
}

struct Inner {
    enabled: bool,
    opened: bool,
    files: HashMap<&'static str, Option<std::fs::File>>,
    paths: HashMap<&'static str, PathBuf>,
    pending: HashMap<u64, Pending>,
}

/// 会话转录写盘器。`&self` 并发安全（内部锁；ASR 线程与翻译 worker 共享）。
pub struct TranscriptWriter {
    base_dir: PathBuf,
    inner: Mutex<Inner>,
}

const KINDS: [&str; 3] = ["original", "translation", "all"];

impl TranscriptWriter {
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self {
            base_dir: base_dir.as_ref().to_path_buf(),
            inner: Mutex::new(Inner {
                enabled: true,
                opened: false,
                files: HashMap::new(),
                paths: HashMap::new(),
                pending: HashMap::new(),
            }),
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        let mut g = self.inner.lock();
        if enabled == g.enabled {
            if enabled && !g.opened {
                open_session(&self.base_dir, &mut g);
            }
            return;
        }
        g.enabled = enabled;
        if enabled && !g.opened {
            open_session(&self.base_dir, &mut g);
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.lock().enabled
    }

    /// 当前会话文件路径（kind → path；未开会话为空）
    pub fn session_paths(&self) -> HashMap<&'static str, PathBuf> {
        self.inner.lock().paths.clone()
    }

    /// 识别原文落盘（original 立即写；all 待译文配对）
    pub fn write_original(&self, msg_id: u64, timestamp: &str, original: &str) {
        if original.is_empty() {
            return;
        }
        let mut g = self.inner.lock();
        if !g.enabled {
            return;
        }
        if !g.opened {
            open_session(&self.base_dir, &mut g);
        }
        g.pending.insert(
            msg_id,
            Pending {
                timestamp: timestamp.to_string(),
                original: original.to_string(),
            },
        );
        write_kind(&mut g, "original", &format!("[{timestamp}] {original}\n"));
    }

    /// 译文落盘（translation + all 配对；无挂起原文时退化为独立行）
    pub fn write_translation(&self, msg_id: u64, translation: &str) {
        if translation.is_empty() {
            return;
        }
        let mut g = self.inner.lock();
        if !g.enabled {
            return;
        }
        if !g.opened {
            open_session(&self.base_dir, &mut g);
        }
        match g.pending.remove(&msg_id) {
            None => {
                let ts = chrono::Local::now().format("%H:%M:%S");
                write_kind(&mut g, "translation", &format!("[{ts}] {translation}\n"));
                write_kind(&mut g, "all", &format!("[{ts}] -> {translation}\n\n"));
            }
            Some(p) => {
                write_kind(
                    &mut g,
                    "translation",
                    &format!("[{}] {translation}\n", p.timestamp),
                );
                write_kind(
                    &mut g,
                    "all",
                    &format!("[{}] {}\n  -> {translation}\n\n", p.timestamp, p.original),
                );
            }
        }
    }

    /// 标记无译文完成（同语言/翻译失败）：all 只写原文块
    pub fn finalize_no_translation(&self, msg_id: u64) {
        let mut g = self.inner.lock();
        match g.pending.remove(&msg_id) {
            _ if !g.enabled => {}
            None => {}
            Some(p) => {
                if !g.opened {
                    open_session(&self.base_dir, &mut g);
                }
                write_kind(
                    &mut g,
                    "all",
                    &format!("[{}] {}\n\n", p.timestamp, p.original),
                );
            }
        }
    }

    pub fn close(&self) {
        let mut g = self.inner.lock();
        for fp in g.files.values_mut() {
            if let Some(f) = fp {
                let _ = f.flush();
            }
        }
        g.files.clear();
        g.pending.clear();
        g.opened = false;
    }
}

/// 打开会话三文件（append + 首行会话头；对照 _open_session_locked）
fn open_session(base_dir: &Path, g: &mut Inner) {
    if let Err(e) = std::fs::create_dir_all(base_dir) {
        tracing::error!(
            "Failed to create transcript dir {}: {e}",
            base_dir.display()
        );
        return;
    }
    let session_ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let header_ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    for kind in KINDS {
        let path = base_dir.join(format!("livetrans_{session_ts}_{kind}.txt"));
        // 追加模式（会话重开不覆盖）+ 即写即刷（tail -f 可用）
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(mut fp) => {
                let _ = writeln!(fp, "# Session started at {header_ts}");
                g.files.insert(kind, Some(fp));
                g.paths.insert(kind, path);
            }
            Err(e) => {
                tracing::error!("Failed to open transcript file {}: {e}", path.display());
                g.files.insert(kind, None);
            }
        }
    }
    g.opened = true;
    tracing::info!("Transcripts -> {}", base_dir.display());
}

fn write_kind(g: &mut Inner, kind: &str, text: &str) {
    if let Some(Some(fp)) = g.files.get_mut(kind) {
        if let Err(e) = fp.write_all(text.as_bytes()) {
            tracing::warn!("Transcript write failed ({kind}): {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "lt-transcript-test-{tag}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn read(base: &Path, kind: &str) -> String {
        let mut p = None;
        for e in std::fs::read_dir(base).unwrap() {
            let p_ = e.unwrap().path();
            if p_.to_string_lossy().ends_with(&format!("_{kind}.txt")) {
                p = Some(p_);
            }
        }
        std::fs::read_to_string(p.unwrap()).unwrap()
    }

    #[test]
    fn lazy_session_opens_on_first_write() {
        let dir = tmpdir("lazy");
        let tw = TranscriptWriter::new(&dir);
        assert!(!tw.is_enabled() == false);
        // 构造期不开文件（原版 _opened=False）
        assert!(tw.session_paths().is_empty());
        tw.write_original(1, "12:00:00", "你好");
        assert_eq!(tw.session_paths().len(), 3);
        let original = read(&dir, "original");
        assert!(original.contains("# Session started at"));
        assert!(original.contains("[12:00:00] 你好"));
    }

    #[test]
    fn paired_translation_writes_all_block() {
        let dir = tmpdir("paired");
        let tw = TranscriptWriter::new(&dir);
        tw.write_original(7, "08:00:01", "hello");
        tw.write_translation(7, "你好");
        let all = read(&dir, "all");
        assert!(
            all.contains("[08:00:01] hello\n  -> 你好\n\n"),
            "all: {all}"
        );
        let trans = read(&dir, "translation");
        assert!(trans.contains("[08:00:01] 你好\n"));
    }

    #[test]
    fn orphan_translation_falls_back_to_now_ts() {
        let dir = tmpdir("orphan");
        let tw = TranscriptWriter::new(&dir);
        tw.write_translation(9, "world");
        let all = read(&dir, "all");
        assert!(all.contains("-> world\n\n"));
        assert!(!all.contains("  ->"), "无原文时不应有缩进配对块: {all}");
    }

    #[test]
    fn finalize_writes_original_without_translation() {
        let dir = tmpdir("final");
        let tw = TranscriptWriter::new(&dir);
        tw.write_original(3, "09:00:00", "same lang");
        tw.finalize_no_translation(3);
        let all = read(&dir, "all");
        assert!(all.contains("[09:00:00] same lang\n\n"));
        let trans = read(&dir, "translation");
        assert!(!trans.contains("same lang"), "translation 文件不应写入");
    }

    #[test]
    fn disabled_writer_writes_nothing() {
        let dir = tmpdir("disabled");
        let tw = TranscriptWriter::new(&dir);
        tw.set_enabled(false);
        tw.write_original(1, "10:00:00", "x");
        tw.write_translation(1, "y");
        // 未开会话即无文件
        assert!(!dir.join("livetrans_original.txt").exists());
        let any: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert!(any.is_empty());
    }

    #[test]
    fn close_clears_session_and_reopens_on_next_write() {
        let dir = tmpdir("reopen");
        let tw = TranscriptWriter::new(&dir);
        tw.write_original(1, "11:00:00", "a");
        tw.close();
        // 原版 close 只清 files/pending/opened，paths 保留（陈旧值亦同）
        assert_eq!(tw.session_paths().len(), 3);
        tw.write_original(2, "11:00:01", "b");
        // append 模式：新会话头追加，两行原文都在
        let original = read(&dir, "original");
        assert_eq!(original.matches("# Session started at").count(), 2);
        assert!(original.contains("[11:00:00] a"));
        assert!(original.contains("[11:00:01] b"));
    }
}
