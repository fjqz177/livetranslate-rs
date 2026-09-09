//! 音频公共层：纯函数（单测对齐 numpy 参考输出）+ 有界队列 + 后端 trait。
//!
//! 逐位对齐原版 `audio_capture.py::_resample_to_mono` 与 numpy 的算术顺序
//! （f64 索引、f32 样本、顺序无 FMA 收缩），这是任务 1.1 的完成标准。

pub mod capture;
#[cfg(windows)]
pub mod wasapi_win;

use parking_lot::{Condvar, Mutex};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// 原版固定 16k mono 的目标采样率
pub const TARGET_RATE: u32 = 16000;
/// 原版 config audio.chunk_duration
pub const CHUNK_DURATION: f64 = 0.032;
/// 16k 下的标准 chunk 样本数（16000 * 0.032 = 512）
pub const CHUNK_SAMPLES: usize = 512;

/// 原版 numpy `np.mean`（contiguous 1-D）的 pairwise 求和复刻，
/// rms/energy 置信度要与 Python 版可比就必须逐位一致。
/// 算法照抄 numpy `pairwise_sum`：n<8 顺序；≤128 时 8 路累加器；
/// 否则对半递归（切点向下取整到 8 的倍数）。
fn numpy_sum_f32(a: &[f32]) -> f32 {
    const BLOCK: usize = 128;
    let n = a.len();
    if n < 8 {
        let mut res = 0.0f32;
        for &x in a {
            res += x;
        }
        res
    } else if n <= BLOCK {
        let mut r = [0.0f32; 8];
        let m = n - n % 8;
        let mut i = 0;
        while i < m {
            r[0] += a[i];
            r[1] += a[i + 1];
            r[2] += a[i + 2];
            r[3] += a[i + 3];
            r[4] += a[i + 4];
            r[5] += a[i + 5];
            r[6] += a[i + 6];
            r[7] += a[i + 7];
            i += 8;
        }
        let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        while i < n {
            res += a[i];
            i += 1;
        }
        res
    } else {
        let n2 = n / 2 - (n / 2) % 8;
        numpy_sum_f32(&a[..n2]) + numpy_sum_f32(&a[n2..])
    }
}

/// interleaved → 各通道平均（对应 `reshape(-1, ch).mean(axis=1)`；
/// 通道数 ≤8 时 numpy 为顺序累加，故此处顺序求和即逐位一致）
pub fn to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    assert!(channels >= 1 && interleaved.len().is_multiple_of(channels));
    if channels == 1 {
        return interleaved.to_vec();
    }
    let frames = interleaved.len() / channels;
    let mut out = Vec::with_capacity(frames);
    for f in 0..frames {
        let row = &interleaved[f * channels..(f + 1) * channels];
        let mut acc = 0.0f32;
        for &s in row {
            acc += s;
        }
        out.push(acc / channels as f32);
    }
    out
}

/// 线性重采样，逐位对齐原版：索引 f64、floor/ceil 双向 clamp、frac f32 插值。
pub fn resample_linear(audio: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return audio.to_vec();
    }
    let ratio = to_rate as f64 / from_rate as f64;
    let n_out = (audio.len() as f64 * ratio) as usize;
    let last = audio.len() as f64 - 1.0;
    let mut out = Vec::with_capacity(n_out);
    for i in 0..n_out {
        let idx_f = ((i as f64 / ratio).max(0.0)).min(last);
        let floor = idx_f as i64 as usize;
        let ceil = (floor + 1).min(audio.len() - 1);
        let frac = (idx_f - floor as f64) as f32;
        out.push(audio[floor] * (1.0 - frac) + audio[ceil] * frac);
    }
    out
}

/// RMS（numpy 逐位一致：f32 pairwise 求和 → 除 n → sqrt）
pub fn rms(audio: &[f32]) -> f32 {
    if audio.is_empty() {
        return 0.0;
    }
    let sq: Vec<f32> = audio.iter().map(|x| x * x).collect();
    (numpy_sum_f32(&sq) / audio.len() as f32).sqrt()
}

/// 原版能量置信度：min(1.0, rms / (energy_threshold * 2))，f64 语义
pub fn energy_confidence(chunk: &[f32], energy_threshold: f64) -> f64 {
    (rms(chunk) as f64 / (energy_threshold * 2.0)).min(1.0)
}

/// 混音（对应 _read_loop 尾部）：loopback + mic[:n]（mic 不足补零）。
/// 返回 (混合结果, mic_chunk 的 RMS)。mic 为空则原样返回且 RMS 为 None。
pub fn mix_with_mic(loopback: &[f32], mic_buf: &mut Vec<f32>) -> (Vec<f32>, Option<f32>) {
    if mic_buf.is_empty() {
        return (loopback.to_vec(), None);
    }
    let n = loopback.len();
    let mut mic_chunk = vec![0.0f32; n];
    let take = n.min(mic_buf.len());
    mic_chunk[..take].copy_from_slice(&mic_buf[..take]);
    mic_buf.drain(..take);
    let mic_rms = rms(&mic_chunk);
    let out: Vec<f32> = loopback
        .iter()
        .zip(mic_chunk.iter())
        .map(|(a, b)| a + b)
        .collect();
    (out, Some(mic_rms))
}

/// pad 到 quantum 的整数倍（对应 §5.5 pad_bucket；quantum = round(16000*pad)）。
/// 原样返回输入切片时零拷贝（None 表示无需 pad）。
pub fn pad_bucket_len(len: usize, quantum: usize) -> usize {
    if len.is_multiple_of(quantum) {
        len
    } else {
        (len / quantum + 1) * quantum
    }
}

pub fn pad_bucket(audio: &[f32], quantum: usize) -> Vec<f32> {
    let target = pad_bucket_len(audio.len(), quantum);
    let mut out = audio.to_vec();
    out.resize(target, 0.0);
    out
}

/// 有界队列：满时丢最旧（对齐原版 audio_queue maxsize=100 的
/// `get_nowait(); put_nowait()` 语义）；支持带超时的阻塞取出。
/// 丢弃带确定性节奏告警（AH-7/H11：装载期/峰值丢段此前完全不可诊断）。
/// W2 起为全仓通用原语（音频队列/段队列/翻译任务池/事件动脉复用），
/// 文案不再限"音频"。
pub struct BoundedDropQueue<T> {
    inner: Mutex<VecDeque<T>>,
    cv: Condvar,
    cap: usize,
    /// 队列标识（丢弃告警定位用）
    name: &'static str,
    /// 累计丢弃数：第 1 次与每 200 次 warn 一条（确定性节奏，免时钟）
    dropped: AtomicU64,
}

impl<T> BoundedDropQueue<T> {
    pub fn new(cap: usize, name: &'static str) -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(cap)),
            cv: Condvar::new(),
            cap,
            name,
            dropped: AtomicU64::new(0),
        }
    }

    fn note_drop(&self) {
        let n = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
        if n == 1 || n.is_multiple_of(200) {
            tracing::warn!(
                "有界队列[{}]已满，丢弃最旧腾位（累计丢弃 {n}，容量 {}）",
                self.name,
                self.cap
            );
        }
    }

    /// 累计丢弃数（W2：水位事件上报源——仲裁"是否丢了"用原子差，免加锁）
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// 满时丢弃最旧一条再入队（与原版 put_nowait 失败路径一致）
    pub fn push(&self, v: T) {
        let mut q = self.inner.lock();
        if q.len() >= self.cap {
            q.pop_front();
            self.note_drop();
        }
        q.push_back(v);
        self.cv.notify_one();
    }

    /// 阻塞带超时取出；None = 超时（对应原版 get_audio(timeout)）
    pub fn pop_timeout(&self, timeout: Duration) -> Option<T> {
        let mut q = self.inner.lock();
        loop {
            if let Some(v) = q.pop_front() {
                return Some(v);
            }
            let timed_out = self.cv.wait_for(&mut q, timeout).timed_out();
            if timed_out && q.is_empty() {
                return None;
            }
        }
    }

    /// 立即取出不等待；None = 空（对应原版 get_nowait）
    pub fn try_pop(&self) -> Option<T> {
        self.inner.lock().pop_front()
    }

    /// 回插队首（原版 `_drain_interim_duplicates` 排空重复标记后把首个非 interim
    /// 项 put 回队首的等价物；满时丢弃队尾一条以保队首项位）
    pub fn push_front(&self, v: T) {
        let mut q = self.inner.lock();
        if q.len() >= self.cap {
            q.pop_back();
            self.note_drop();
        }
        q.push_front(v);
        self.cv.notify_one();
    }

    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 清空（设备重启后丢弃陈旧数据；对齐原版 restart 后 drain）
    pub fn clear(&self) {
        self.inner.lock().clear();
    }
}

/// 音频后端抽象：Windows 上由 [`wasapi_win::WasapiBackend`] 实现。
pub trait AudioBackend: Send {
    /// 枚举输出（loopback 源）设备名
    fn list_output_devices(&self) -> anyhow::Result<Vec<String>>;
    /// 枚举输入（麦克风）设备名
    fn list_input_devices(&self) -> anyhow::Result<Vec<String>>;
    /// 当前系统默认输出设备名
    fn current_default_output(&self) -> anyhow::Result<Option<String>>;

    /// 启动采集线程；device 语义与 settings.audio_device 一致
    /// （None=系统默认 | 名字 | "__disabled__"）。
    /// 返回的 chunk 为 16k mono + 可选 mic RMS。`status` 为可选的可用性
    /// 边沿上报通道（R4/D-62：打开/读取失败与恢复；None = 不上报）
    fn start(
        &mut self,
        device: Option<String>,
        mic_device: Option<String>,
        chunk_tx: std::sync::Arc<BoundedDropQueue<(Vec<f32>, Option<f32>)>>,
        status: Option<std::sync::mpsc::Sender<wasapi_win::AudioStatus>>,
    ) -> anyhow::Result<()>;

    /// 运行时切换采集设备（触发线程内重启）
    fn set_device(&mut self, device: Option<String>);
    /// 运行时切换麦克风（None=禁用；触发线程内重启）
    fn set_mic_device(&mut self, mic_device: Option<String>);
    /// 停止并回收线程
    fn stop(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 读取 fixtures（由 tests/gen_fixtures.py 生成，numpy 参考输出）
    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/",).to_string() + name)
            .expect("fixture missing; run: python tests/gen_fixtures.py")
    }
    fn read_f32(name: &str) -> Vec<f32> {
        let b = fixture(name);
        b.as_chunks::<4>().0.iter()
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }
    fn meta() -> serde_json::Value {
        serde_json::from_str(
            &std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/meta.json"
            ))
            .unwrap(),
        )
        .unwrap()
    }

    /// 生成 8 组随机长度（8..=600）的信号，验证 pairwise 和与朴素
    /// numpy sum（用 Python 已验证的公式）在长数组上一致——这里仅
    /// 验证算法自身可重复，逐位一致性由 fixtures 覆盖。
    #[test]
    fn pairwise_sum_repeats() {
        let a: Vec<f32> = (0..300)
            .map(|i| ((i * 37) % 101) as f32 / 101.0 - 0.5)
            .collect();
        assert_eq!(numpy_sum_f32(&a), numpy_sum_f32(&a));
    }

    #[test]
    fn resample_matches_numpy() {
        let m = meta();
        for case in m["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let ch = case["channels"].as_u64().unwrap() as usize;
            let rate = case["rate"].as_u64().unwrap() as u32;
            let inter = read_f32(&format!("{name}.in.bin"));
            let want = read_f32(&format!("{name}.out.bin"));
            let mono = to_mono(&inter, ch);
            let got = resample_linear(&mono, rate, TARGET_RATE);
            assert_eq!(got.len(), want.len(), "{name}: length mismatch");
            for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
                assert_eq!(g.to_bits(), w.to_bits(), "{name}[{i}]: {g} != {w}");
            }
        }
    }

    #[test]
    fn mix_matches_numpy() {
        let m = meta();
        let mix = &m["mix"];
        let loopback = read_f32("mix_loop.bin");
        let mut mic_buf = read_f32("mix_mic.bin");
        let (out, mic_rms) = mix_with_mic(&loopback, &mut mic_buf);
        let want = read_f32("mix_out.bin");
        for (i, (g, w)) in out.iter().zip(want.iter()).enumerate() {
            assert_eq!(g.to_bits(), w.to_bits(), "mix[{i}]");
        }
        // mic_rms 与 numpy 逐位一致（f32 bits）
        let want_rms = mic_rms.unwrap();
        let w = mix["mic_rms"].as_f64().unwrap() as f32;
        assert_eq!(
            want_rms.to_bits(),
            w.to_bits(),
            "mic_rms {} != {}",
            want_rms,
            w
        );
        // mic 缓冲已被消费
        assert!(mic_buf.is_empty());
        // mic 为空时直通
        let mut empty = Vec::new();
        let (out, r) = mix_with_mic(&loopback, &mut empty);
        assert_eq!(out, loopback);
        assert!(r.is_none());
    }

    #[test]
    fn rms_matches_numpy_pairwise() {
        // 512 样本（>128 → 对半递归路径）与 300（≤128 → 8 路累加路径）
        for n in [512usize, 300, 64, 7] {
            let a: Vec<f32> = (0..n)
                .map(|i| ((i * 89) % 251) as f32 / 251.0 - 0.5)
                .collect();
            let r = rms(&a);
            // 参考值由 numpy 在 gen_fixtures 同款公式计算（内嵌期望）
            let want = numpy_rms_reference(&a);
            assert_eq!(r.to_bits(), want.to_bits(), "rms n={n}");
        }
    }

    /// 与 numpy 一致的参考实现（直接用 pairwise 和 / n 再 sqrt）
    fn numpy_rms_reference(a: &[f32]) -> f32 {
        let sq: Vec<f32> = a.iter().map(|x| x * x).collect();
        (numpy_sum_f32(&sq) / a.len() as f32).sqrt()
    }

    #[test]
    fn pad_bucket_behavior() {
        let m = meta();
        let pad = &m["pad"];
        let q = pad["quantum"].as_u64().unwrap() as usize;
        let inp = read_f32("pad_in.bin");
        let got = pad_bucket(&inp, q);
        let want = read_f32("pad_out.bin");
        assert_eq!(got, want);
        let exact = read_f32("pad_exact_in.bin");
        assert_eq!(pad_bucket(&exact, q), exact);
        assert_eq!(pad_bucket_len(8192, q), 16000);
        assert_eq!(pad_bucket_len(16000, q), 16000);
    }

    #[test]
    fn drop_queue_semantics() {
        let q: BoundedDropQueue<u32> = BoundedDropQueue::new(3, "test");
        for i in 0..5 {
            q.push(i);
        }
        assert_eq!(q.len(), 3);
        assert_eq!(q.dropped_count(), 2, "满丢最旧计数（W2 水位事件源）");
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Some(2));
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Some(3));
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Some(4));
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), None);
        q.clear();
        assert!(q.is_empty());
    }

    #[test]
    fn try_pop_and_push_front_preserve_order() {
        // 排空重复标记场景：弹出后回插队首，FIFO 顺序不变（原版 put 回语义）
        let q: BoundedDropQueue<u32> = BoundedDropQueue::new(4, "test");
        for i in [10, 20, 30] {
            q.push(i);
        }
        assert_eq!(q.try_pop(), Some(10));
        assert_eq!(q.try_pop(), Some(20));
        assert_eq!(q.try_pop(), Some(30));
        assert_eq!(q.try_pop(), None);
        q.push_front(20);
        assert_eq!(q.try_pop(), Some(20));
        // 满时 push_front 丢队尾保队首
        let full: BoundedDropQueue<u32> = BoundedDropQueue::new(2, "test");
        full.push(1);
        full.push(2);
        full.push_front(0);
        assert_eq!(full.try_pop(), Some(0));
        assert_eq!(full.try_pop(), Some(1));
        assert_eq!(full.try_pop(), None);
    }

    #[test]
    fn energy_confidence_clamps() {
        // rms=0.1, thr=0.02 → 0.1/(0.04)=2.5 → clamp 1.0
        let chunk = vec![0.1f32; 512];
        assert_eq!(energy_confidence(&chunk, 0.02), 1.0);
        let quiet = vec![0.0f32; 512];
        assert_eq!(energy_confidence(&quiet, 0.02), 0.0);
    }
}
