//! lt-pipeline：音频采集 / VAD / 分句 / 增量 ASR / 转写落盘。

pub mod audio;
pub mod vad;

pub use audio::{
    mix_with_mic, pad_bucket, pad_bucket_len, resample_linear, rms, to_mono,
    AudioBackend, BoundedDropQueue, CHUNK_DURATION, CHUNK_SAMPLES, TARGET_RATE,
};
pub use audio::capture::CaptureLoop;
pub use vad::{
    ensure_ort_dylib, make_confidence_source, ConfidenceSource, DisabledVad, EnergyVad,
    SileroVad, VadProcessor, VadSettings,
};

/// 段来源（对齐原版 _enqueue_asr 的 seg_type）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentSource {
    /// VAD 收段（含 max 切分与超时静音推进）
    VadFlush,
    /// 增量 ASR 完结（M6 interim 接入后启用）
    InterimFinal,
}

/// capture 线程对外事件（→ UI/ASR 队列）
#[derive(Debug)]
pub enum CaptureEvent {
    /// 监视条数据（对齐原版 update_monitor；vad 为上一 chunk 置信度）
    Monitor { rms: f32, vad: f64, mic_rms: Option<f32> },
    /// 完整语音段（送 ASR 队列）
    Segment { source: SegmentSource, audio: Vec<f32> },
}
