//! lt-audio：音频采集 / VAD / 短句原语 / 转写落盘。

pub mod audio;
pub mod interim;
pub mod transcript;
pub mod vad;

pub use audio::capture::{CaptureLoop, VadSource};
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
    /// VAD 收段（含 max 切分与超时静音推进；原版 "vad_flush"，携段音频）。
    /// （原 "interim" 标记变体随实时通道裁撤，D-134；枚举保留为段通道的
    /// 来源契约位）
    VadFlush,
}
