//! lt-audio：音频采集 / VAD / 分句 / 增量 ASR / 转写落盘。

pub mod audio;
pub mod interim;
pub mod transcript;
pub mod vad;

pub use audio::capture::{CaptureLoop, InterimControl};
pub use audio::{
    mix_with_mic, pad_bucket, pad_bucket_len, resample_linear, rms, to_mono, AudioBackend,
    BoundedDropQueue, CHUNK_DURATION, CHUNK_SAMPLES, TARGET_RATE,
};
pub use vad::{
    ensure_ort_dylib, make_confidence_source, ConfidenceSource, DisabledVad, EnergyVad, SileroVad,
    VadProcessor, VadSettings,
};

/// 段来源（对齐原版 _enqueue_asr 的 seg_type；段队列元素的首个字段）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentSource {
    /// VAD 收段（含 max 切分与超时静音推进；原版 "vad_flush"，携段音频）
    VadFlush,
    /// 增量触发标记（原版 "interim"）：空音频，capture 线程只做触发判定塞标记，
    /// peek/识别/裁剪全在 ASR 线程锁 VAD 后进行（原版拓扑，勿在 capture 线程跑 ASR）
    Interim,
}
