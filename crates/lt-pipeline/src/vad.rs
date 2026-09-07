//! VAD：Silero v5（ort，state 自持回喂）+ 状态机全量移植（PLAN §5.1）。
//!
//! 移植基准：`LiveTranslate/vad_processor.py`，逐参数 1:1。
//! 置信度计算与状态机解耦（[`ConfidenceSource`]），单测注入脚本序列、不跑真模型。

use crate::audio::energy_confidence;
use anyhow::Context;
use std::collections::VecDeque;
use std::path::Path;

/// 内嵌 Silero v5 模型（来源与 sha256 见 assets/SOURCES.md）
static SILERO_MODEL: &[u8] = include_bytes!("../../../assets/silero_vad.onnx");

/// 内嵌 onnxruntime.dll（R-4 预案 A：ort 走 load-dynamic，主进程独占一份 ORT；
/// worker 子进程用 sherpa 静态 ORT，互不冲突）
static ORT_DLL: &[u8] = include_bytes!("../../../assets/ort/onnxruntime.dll");

/// 解压 onnxruntime.dll 到配置目录并设置 ORT_DYLIB_PATH（幂等，进程内只需成功一次）。
/// 必须在首个 ort Session 创建前调用。
pub fn ensure_ort_dylib() -> anyhow::Result<()> {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    let mut result = Ok(());
    ONCE.call_once(|| {
        result = (|| {
            let dir = lt_models::paths::config_dir()?.join("ort");
            std::fs::create_dir_all(&dir)?;
            let target = dir.join("onnxruntime.dll");
            // 尺寸一致视为已解压（版本随 exe 发布更新时删除旧文件重新解压）
            let need = !target.is_file()
                || std::fs::metadata(&target).map(|m| m.len() as usize).unwrap_or(0) != ORT_DLL.len();
            if need {
                let tmp = target.with_extension("dll.tmp");
                std::fs::write(&tmp, ORT_DLL)?;
                std::fs::rename(&tmp, &target)?;
                tracing::info!("onnxruntime.dll 已解压至 {}", target.display());
            }
            // ort 在首个 Session 前读取该环境变量
            std::env::set_var("ORT_DYLIB_PATH", &target);
            Ok(())
        })();
    });
    result
}

/// 16k 下的 Silero 输入窗口
const WINDOW_16K: usize = 512;
/// v5 state 张量元素数（2×1×128）
const STATE_ELEMS: usize = 256;

// ─────────────────────────── 置信度源 ───────────────────────────

/// 置信度计算抽象（silero / energy / disabled / 测试脚本）
pub trait ConfidenceSource: Send {
    fn confidence(&mut self, chunk: &[f32]) -> anyhow::Result<f64>;
    /// 采集会话开始时复位（Silero 需要清零 state）
    fn reset(&mut self) {}
}

/// Box<dyn> 装置器直通（默认泛型参数用到）
impl<T: ConfidenceSource + ?Sized> ConfidenceSource for Box<T> {
    fn confidence(&mut self, chunk: &[f32]) -> anyhow::Result<f64> {
        (**self).confidence(chunk)
    }
    fn reset(&mut self) {
        (**self).reset()
    }
}

/// Silero v5（ort CPU EP）。state 必须自持并逐 chunk 回喂（经验 E-02）。
pub struct SileroVad {
    session: ort::session::Session,
    state: Vec<f32>,
    window: usize,
}

impl SileroVad {
    /// 从内嵌模型构建；`assets/silero_vad.onnx` 可被同路径文件覆盖（调试用）
    pub fn new() -> anyhow::Result<Self> {
        Self::from_bytes(SILERO_MODEL)
    }

    pub fn from_path(p: &Path) -> anyhow::Result<Self> {
        Self::from_bytes(&std::fs::read(p)?)
    }

    pub fn from_bytes(model: &[u8]) -> anyhow::Result<Self> {
        // load-dynamic：确保 dll 已解压且 ORT_DYLIB_PATH 就位（R-4 预案 A）
        super::ensure_ort_dylib().context("解压 onnxruntime.dll")?;
        let session = ort::session::Session::builder()
            .context("ort SessionBuilder")?
            .commit_from_memory(model)
            .context("加载内嵌 silero_vad.onnx")?;
        Ok(Self {
            session,
            state: vec![0.0; STATE_ELEMS],
            window: WINDOW_16K,
        })
    }

    /// 单 chunk（≤window，不足补零）置信度；逐 chunk 回喂新 state
    fn conf(&mut self, chunk: &[f32]) -> anyhow::Result<f64> {
        let mut input = vec![0.0f32; self.window];
        let n = chunk.len().min(self.window);
        input[..n].copy_from_slice(&chunk[..n]);

        let input_t = ort::value::Tensor::from_array(ndarray::Array2::from_shape_fn(
            (1, self.window),
            |(_, j)| input[j],
        ))?;
        let state_t = ort::value::Tensor::from_array(ndarray::Array3::from_shape_fn(
            (2, 1, 128),
            |(i, _, j)| self.state[i * 128 + j],
        ))?;
        let sr_t = ort::value::Tensor::from_array(ndarray::Array0::from_elem((), 16000i64))?;

        let outputs = self.session.run(ort::inputs![
            "input" => input_t,
            "state" => state_t,
            "sr" => sr_t,
        ])?;

        let (_, out_data) = outputs["output"].try_extract_tensor::<f32>()?;
        let conf = out_data[0];
        // state 输出：v5 中除 "output" 外的输出即 stateN
        let state_name = outputs
            .iter()
            .map(|(k, _)| k.to_string())
            .find(|k| k.as_str() != "output")
            .context("模型缺少 state 输出")?;
        let (_, st_data) = outputs[state_name].try_extract_tensor::<f32>()?;
        let m = st_data.len().min(STATE_ELEMS);
        self.state[..m].copy_from_slice(&st_data[..m]);
        Ok(conf as f64)
    }
}

impl ConfidenceSource for SileroVad {
    fn confidence(&mut self, chunk: &[f32]) -> anyhow::Result<f64> {
        self.conf(chunk)
    }
    fn reset(&mut self) {
        self.state = vec![0.0; STATE_ELEMS];
    }
}

/// 能量模式（原版 _energy_confidence：min(1, rms/(thr*2))，f64 语义）
pub struct EnergyVad {
    pub threshold: f64,
}

impl ConfidenceSource for EnergyVad {
    fn confidence(&mut self, chunk: &[f32]) -> anyhow::Result<f64> {
        Ok(energy_confidence(chunk, self.threshold))
    }
}

/// 禁用模式：恒 1.0
pub struct DisabledVad;

impl ConfidenceSource for DisabledVad {
    fn confidence(&mut self, _chunk: &[f32]) -> anyhow::Result<f64> {
        Ok(1.0)
    }
}

/// 按设置组装置信度源（vad_mode 三态）
pub fn make_confidence_source(
    mode: &str,
    energy_threshold: f64,
) -> Box<dyn ConfidenceSource + Send> {
    match mode {
        "energy" => Box::new(EnergyVad { threshold: energy_threshold }),
        "disabled" => Box::new(DisabledVad),
        _ => match SileroVad::new() {
            Ok(v) => Box::new(v),
            Err(e) => {
                tracing::error!("Silero VAD 加载失败，回退 energy 模式: {e:#}");
                Box::new(EnergyVad { threshold: 0.02 })
            }
        },
    }
}

// ─────────────────────────── 设置 ───────────────────────────

#[derive(Debug, Clone)]
pub struct VadSettings {
    pub mode: String,               // silero | energy | disabled
    pub threshold: f64,             // silero 阈值
    pub energy_threshold: f64,      // energy 阈值
    pub min_speech_duration: f64,   // 秒
    pub max_speech_duration: f64,   // 秒
    pub silence_mode: String,       // auto | fixed
    pub silence_duration: f64,      // 秒
}

impl Default for VadSettings {
    fn default() -> Self {
        Self {
            mode: "silero".into(),
            threshold: 0.5,
            energy_threshold: 0.02,
            min_speech_duration: 1.0,
            max_speech_duration: 15.0,
            silence_mode: "auto".into(),
            silence_duration: 0.8,
        }
    }
}

// ─────────────────────────── 状态机 ───────────────────────────

/// pause_history 容量上限（对齐 Python deque(maxlen=50)）
const PAUSE_HISTORY_MAX: usize = 50;

/// VAD 状态机（vad_processor.py 1:1；时长语义以 chunk 计数为准）
pub struct VadProcessor<C: ConfidenceSource = Box<dyn ConfidenceSource + Send>> {
    conf: C,
    sample_rate: usize,
    chunk_duration: f64,
    mode: String,
    threshold: f64,
    energy_threshold: f64,
    min_speech_samples: usize,
    max_speech_samples: usize,

    speech_buffer: Vec<Vec<f32>>,
    confidence_history: Vec<f64>,
    speech_samples: usize,
    is_speaking: bool,
    silence_counter: usize,
    was_trimmed: bool,

    pre_speech_chunks: usize, // 3
    pre_buffer: VecDeque<Vec<f32>>,

    silence_mode: String, // auto | fixed
    fixed_silence_dur: f64,
    silence_limit: usize,

    progressive_tiers: [(f64, f64); 3], // (buffer_seconds, multiplier)

    pause_history: VecDeque<f64>, // 50
    adaptive_min: f64,            // 0.3
    adaptive_max: f64,            // 2.0

    /// 缓冲代际（AH-4/D-27）：peek→识别→trim 的增量通道跨线程非原子，
    /// 识别期间 VAD 收段/切分后旧 trim 依据失效——代际在 reset 与
    /// split_at_best_pause（缓冲头部变更的两个漏斗）各 +1，trim 前校验
    pub(crate) generation: u64,

    pub last_confidence: f64,
}

impl<C: ConfidenceSource> VadProcessor<C> {
    pub fn new(
        conf: C,
        sample_rate: usize,
        threshold: f64,
        min_speech_duration: f64,
        max_speech_duration: f64,
        chunk_duration: f64,
    ) -> Self {
        let p = Self {
            conf,
            sample_rate,
            chunk_duration,
            mode: "silero".into(),
            threshold,
            energy_threshold: 0.02,
            min_speech_samples: (min_speech_duration * sample_rate as f64) as usize,
            max_speech_samples: (max_speech_duration * sample_rate as f64) as usize,
            speech_buffer: Vec::new(),
            confidence_history: Vec::new(),
            speech_samples: 0,
            is_speaking: false,
            silence_counter: 0,
            was_trimmed: false,
            pre_speech_chunks: 3,
            pre_buffer: VecDeque::new(),
            silence_mode: "auto".into(),
            fixed_silence_dur: 0.8,
            silence_limit: (Self::py_round(0.8 / chunk_duration).max(1.0)) as usize,
            progressive_tiers: [(3.0, 1.0), (6.0, 0.5), (10.0, 0.25)],
            pause_history: VecDeque::with_capacity(50),
            adaptive_min: 0.3,
            adaptive_max: 2.0,
            generation: 0,
            last_confidence: 0.0,
        };
        p
    }

    /// Python round()（银行家舍入）——seconds_to_chunks 语义对齐
    fn py_round(x: f64) -> f64 {
        let r = x.floor();
        let d = x - r;
        if d > 0.5 {
            r + 1.0
        } else if d < 0.5 {
            r
        } else if (r as i64) % 2 == 0 {
            r
        } else {
            r + 1.0
        }
    }

    fn seconds_to_chunks(&self, seconds: f64) -> usize {
        (Self::py_round(seconds / self.chunk_duration).max(1.0)) as usize
    }

    fn update_adaptive_limit(&mut self) {
        if self.pause_history.len() < 3 {
            return;
        }
        let mut pauses: Vec<f64> = self.pause_history.iter().copied().collect();
        pauses.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let idx = (pauses.len() as f64 * 0.75) as usize;
        let p75 = pauses[idx.min(pauses.len() - 1)];
        let target = self.adaptive_min.max(self.adaptive_max.min(p75 * 1.2));
        let new_limit = self.seconds_to_chunks(target);
        if new_limit != self.silence_limit {
            tracing::debug!(
                "Adaptive silence: {target:.2}s ({new_limit} chunks), P75={p75:.2}s"
            );
            self.silence_limit = new_limit;
        }
    }

    /// 引擎段长上限钳制（AH-8/D-28）：qwen3 的 `MAX_TOTAL_LEN=512` 为
    /// audio+输出共享 token 预算，超长段有静默截尾风险——生效值超上限则收敛。
    /// 返回是否发生钳制（供调用方记日志）；settings/UI 保存值不动。
    /// 注意：后续 `update_settings`（面板「应用」全量重放）会整体覆盖本值，
    /// 调用方须在应用点同步钳制。
    pub fn clamp_max_speech(&mut self, engine: &str, max_secs: f64) -> bool {
        if engine != "qwen3" {
            return false;
        }
        let cap_samples = (max_secs * self.sample_rate as f64) as usize;
        if self.max_speech_samples > cap_samples {
            self.max_speech_samples = cap_samples;
            true
        } else {
            false
        }
    }

    /// 对齐原版 update_settings：mode/threshold/energy/min/max/silence 全量热更新。
    /// 模式切换时同步替换置信度源的行为由调用方（持有源）负责；本结构只更新阈值语义。
    pub fn update_settings(&mut self, s: &VadSettings) {        self.mode = s.mode.clone();
        self.threshold = s.threshold;
        self.energy_threshold = s.energy_threshold;
        self.min_speech_samples = (s.min_speech_duration * self.sample_rate as f64) as usize;
        self.max_speech_samples = (s.max_speech_duration * self.sample_rate as f64) as usize;
        self.silence_mode = s.silence_mode.clone();
        self.fixed_silence_dur = s.silence_duration;
        if self.silence_mode == "fixed" {
            self.silence_limit = self.seconds_to_chunks(self.fixed_silence_dur);
        }
        tracing::info!(
            "VAD settings updated: mode={}, threshold={}, silence={} ({} chunks = {:.2}s)",
            self.mode, self.threshold, self.silence_mode,
            self.silence_limit, self.silence_limit as f64 * self.chunk_duration
        );
    }

    fn effective_threshold(&self) -> f64 {
        if self.mode == "silero" {
            self.threshold
        } else {
            0.5
        }
    }

    /// Progressive silence：缓冲越长，静音切分阈值越短
    pub fn effective_silence_limit(&self) -> usize {
        let buf_seconds = self.speech_samples as f64 / self.sample_rate as f64;
        let mut multiplier = 1.0;
        for (tier_sec, tier_mult) in self.progressive_tiers {
            if buf_seconds < tier_sec {
                break;
            }
            multiplier = tier_mult;
        }
        (Self::py_round(self.silence_limit as f64 * multiplier).max(1.0)) as usize
    }

    fn push_chunk(&mut self, chunk: &[f32], conf: f64) {
        self.speech_buffer.push(chunk.to_vec());
        self.confidence_history.push(conf);
        self.speech_samples += chunk.len();
    }

    /// 主入口：喂一个 chunk，产出完成的段（无则 None）
    pub fn process_chunk(&mut self, chunk: &[f32]) -> Option<Vec<f32>> {
        let confidence = self
            .conf
            .confidence(chunk)
            .unwrap_or_else(|e| {
                tracing::warn!("VAD 置信度计算失败: {e:#}");
                0.0
            });
        self.last_confidence = confidence;

        let effective_threshold = self.effective_threshold();
        let eff_silence_limit = self.effective_silence_limit();

        if confidence >= effective_threshold {
            // 记录停顿时长（自适应模式）
            if self.is_speaking && self.silence_counter > 0 {
                let pause_dur = self.silence_counter as f64 * self.chunk_duration;
                if pause_dur >= 0.1 {
                    self.push_pause(pause_dur);
                    if self.silence_mode == "auto" {
                        self.update_adaptive_limit();
                    }
                }
            }
            if !self.is_speaking {
                // 语音起始：前置环缓冲回放（用阈值当置信度，避免假谷）
                for pre in std::mem::take(&mut self.pre_buffer) {
                    self.speech_samples += pre.len();
                    self.speech_buffer.push(pre);
                    self.confidence_history.push(effective_threshold);
                }
            }
            self.is_speaking = true;
            self.silence_counter = 0;
            self.push_chunk(chunk, confidence);
        } else if self.is_speaking {
            self.silence_counter += 1;
            self.push_chunk(chunk, confidence);
        } else {
            // 未在说话：喂前置环缓冲
            self.pre_buffer.push_back(chunk.to_vec());
            while self.pre_buffer.len() > self.pre_speech_chunks {
                self.pre_buffer.pop_front();
            }
        }

        // 达到最大时长 → 回溯最佳切分点
        if self.speech_samples >= self.max_speech_samples {
            return self.split_at_best_pause();
        }

        // 静音足够（渐进阈值）→ 收段
        if self.is_speaking && self.silence_counter >= eff_silence_limit {
            if self.speech_samples >= self.min_speech_samples {
                return self.flush_segment();
            } else if self.was_trimmed {
                tracing::debug!(
                    "Short segment after trim ({:.1}s), force flushing for interim final",
                    self.speech_samples as f64 / self.sample_rate as f64
                );
                return self.force_flush();
            } else {
                tracing::debug!(
                    "Short segment {:.1}s < min {:.1}s, keeping for merge",
                    self.speech_samples as f64 / self.sample_rate as f64,
                    self.min_speech_samples as f64 / self.sample_rate as f64
                );
                self.is_speaking = false;
                self.silence_counter = 0;
                return None;
            }
        }
        None
    }

    fn push_pause(&mut self, dur: f64) {
        if self.pause_history.len() >= PAUSE_HISTORY_MAX {
            self.pause_history.pop_front();
        }
        self.pause_history.push_back(dur);
    }

    /// 平滑置信度谷底搜索（fast commentary 下也能找到可用切分点）
    fn find_best_split_index(&self) -> i64 {
        let n = self.confidence_history.len();
        if n < 4 {
            return -1;
        }
        let smooth_win = 5.min(n / 2);
        let half = smooth_win / 2;
        let smoothed: Vec<f64> = (0..n)
            .map(|i| {
                let lo = i.saturating_sub(half);
                let hi = (i + half + 1).min(n);
                self.confidence_history[lo..hi]
                    .iter()
                    .sum::<f64>()
                    / (hi - lo) as f64
            })
            .collect();

        let search_start = 1.max(n * 3 / 10);
        let mut min_val = f64::INFINITY;
        let mut min_idx: i64 = -1;
        for i in search_start..n {
            if smoothed[i] <= min_val {
                min_val = smoothed[i];
                min_idx = i as i64;
            }
        }
        if min_idx <= 0 {
            return -1;
        }
        let avg_conf =
            smoothed[search_start..].iter().sum::<f64>() / 1.max(n - search_start) as f64;
        let dip_ratio = min_val / avg_conf.max(1e-6);
        let effective_threshold = self.effective_threshold();
        if min_val < effective_threshold || dip_ratio < 0.8 {
            tracing::debug!(
                "Split point at chunk {min_idx}/{n}: smoothed={min_val:.3}, avg={avg_conf:.3}, dip_ratio={dip_ratio:.2}"
            );
            return min_idx;
        }
        if min_val < avg_conf {
            tracing::debug!(
                "Split point (fallback) at chunk {min_idx}/{n}: smoothed={min_val:.3}, avg={avg_conf:.3}"
            );
            return min_idx;
        }
        -1
    }

    /// 达到最大时长：回溯切分，前半出段、余量续存
    fn split_at_best_pause(&mut self) -> Option<Vec<f32>> {
        if self.speech_buffer.is_empty() {
            return None;
        }
        let split_idx = self.find_best_split_index();
        if split_idx <= 0 {
            tracing::info!(
                "Max duration reached, no good split point, hard flush {:.1}s",
                self.speech_samples as f64 / self.sample_rate as f64
            );
            return self.flush_segment();
        }
        let idx = split_idx as usize;
        let remain = self.speech_buffer.split_off(idx);
        let remain_confs = self.confidence_history.split_off(idx);
        let first_samples = self.speech_samples
            - remain.iter().map(|b| b.len()).sum::<usize>();
        let remain_samples = remain.iter().map(|b| b.len()).sum::<usize>();

        tracing::info!(
            "Max duration split at {:.1}s, keeping {:.1}s remainder",
            first_samples as f64 / self.sample_rate as f64,
            remain_samples as f64 / self.sample_rate as f64
        );

        let mut segment = Vec::with_capacity(first_samples);
        for b in std::mem::take(&mut self.speech_buffer) {
            segment.extend_from_slice(&b);
        }
        self.confidence_history.clear();
        self.speech_buffer = remain;
        self.confidence_history = remain_confs;
        self.speech_samples = remain_samples;
        self.is_speaking = true;
        self.silence_counter = 0;
        // 缓冲头部已整体更替：作废在途增量通道的 trim 依据（AH-4/D-27）
        self.generation = self.generation.wrapping_add(1);
        Some(segment)
    }

    /// 收段 + 语音密度过滤（<25% 丢弃）
    fn flush_segment(&mut self) -> Option<Vec<f32>> {
        if self.speech_buffer.is_empty() {
            return None;
        }
        if self.confidence_history.len() >= 4 {
            let effective_threshold = self.effective_threshold();
            let voiced = self
                .confidence_history
                .iter()
                .filter(|c| **c >= effective_threshold)
                .count();
            let density = voiced as f64 / self.confidence_history.len() as f64;
            if density < 0.25 {
                tracing::debug!(
                    "Low speech density {:.0}% ({voiced}/{}), discarding {:.1}s segment",
                    density * 100.0,
                    self.confidence_history.len(),
                    self.speech_samples as f64 / self.sample_rate as f64
                );
                self.reset();
                return None;
            }
        }
        let mut segment = Vec::with_capacity(self.speech_samples);
        for b in self.speech_buffer.drain(..) {
            segment.extend_from_slice(&b);
        }
        self.reset();
        Some(segment)
    }

    pub fn reset(&mut self) {
        self.speech_buffer.clear();
        self.confidence_history.clear();
        self.speech_samples = 0;
        self.is_speaking = false;
        self.silence_counter = 0;
        self.was_trimmed = false;
        // 缓冲清空：作废在途增量通道的 trim 依据（AH-4/D-27）
        self.generation = self.generation.wrapping_add(1);
    }

    /// 读取当前缓冲（增量 ASR 用；不冲刷）。返回 `(音频, 时长, 代际)`——
    /// 代际供 `trim_front_checked` 校验（AH-4/D-27）
    pub fn peek_buffer(&self) -> Option<(Vec<f32>, f64, u64)> {
        if self.speech_buffer.is_empty() || !self.is_speaking {
            return None;
        }
        let audio = self.concat_buffer();
        let duration = self.speech_samples as f64 / self.sample_rate as f64;
        Some((audio, duration, self.generation))
    }

    /// 从缓冲头部移除 n_samples（增量 ASR 消费；部分裁剪置 was_trimmed）。
    /// 带代际校验（AH-4/D-27）：peek 之后若 VAD 已收段/切分（代际推进），
    /// 本次 trim 依据的音频边界已失效——放弃裁剪并返回 false，防误裁新段
    /// 头部（识别期间 capture 线程仍在写 VAD，全程持锁会阻塞采集）
    pub fn trim_front_checked(&mut self, n_samples: usize, generation: u64) -> bool {
        if self.generation != generation {
            tracing::debug!(
                "trim_front_checked: 代际不符（peek={generation} 当前={}），放弃本次裁剪",
                self.generation
            );
            return false;
        }
        self.trim_front(n_samples);
        true
    }

    /// 从缓冲头部移除 n_samples（无校验；增量通道应优先用
    /// [`Self::trim_front_checked`]，AH-4/D-27）
    pub fn trim_front(&mut self, n_samples: usize) {
        if n_samples == 0 {
            return;
        }
        let mut removed = 0usize;
        while removed < n_samples {
            let Some(chunk) = self.speech_buffer.first().cloned() else {
                break;
            };
            if removed + chunk.len() <= n_samples {
                self.speech_buffer.remove(0);
                self.confidence_history.remove(0);
                removed += chunk.len();
            } else {
                let keep = removed + chunk.len() - n_samples;
                self.speech_buffer[0] = chunk[chunk.len() - keep..].to_vec();
                removed = n_samples;
            }
        }
        self.speech_samples = self.speech_buffer.iter().map(|b| b.len()).sum();
        self.was_trimmed = true;
        tracing::debug!(
            "trim_front: removed {removed} samples, remaining {:.2}s",
            self.speech_samples as f64 / self.sample_rate as f64
        );
    }

    /// 无视 min_speech 直接收段
    pub fn force_flush(&mut self) -> Option<Vec<f32>> {
        if self.speech_buffer.is_empty() {
            return None;
        }
        let segment = self.concat_buffer();
        self.reset();
        Some(segment)
    }

    /// 收尾：达标收段，否则丢弃
    pub fn flush(&mut self) -> Option<Vec<f32>> {
        if self.speech_samples >= self.min_speech_samples {
            return self.flush_segment();
        }
        self.reset();
        None
    }

    fn concat_buffer(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.speech_samples);
        for b in &self.speech_buffer {
            out.extend_from_slice(b);
        }
        out
    }

    pub fn is_speaking(&self) -> bool {
        self.is_speaking
    }

    pub fn speech_samples(&self) -> usize {
        self.speech_samples
    }
}

// ─────────────────────────── 测试 ───────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 脚本化置信度源：按序弹出（超出后重复末值），chunk 内容不计
    struct Script {
        confs: Vec<f64>,
        i: usize,
    }
    impl Script {
        fn new(confs: &[f64]) -> Self {
            Self { confs: confs.to_vec(), i: 0 }
        }
    }
    impl ConfidenceSource for Script {
        fn confidence(&mut self, _chunk: &[f32]) -> anyhow::Result<f64> {
            let c = self.confs[self.i.min(self.confs.len() - 1)];
            self.i += 1;
            Ok(c)
        }
    }

    /// 32ms/chunk 语义（512 样本）；min=1s、max=15s 的标准处理器
    fn make(confs: &[f64]) -> VadProcessor<Script> {
        let mut p = VadProcessor::new(Script::new(confs), 16000, 0.5, 1.0, 15.0, 0.032);
        p.mode = "silero".into();
        p
    }

    fn feed(p: &mut VadProcessor<Script>, n: usize) -> Vec<Vec<f32>> {
        let chunk = vec![0.5f32; 512];
        (0..n).filter_map(|_| p.process_chunk(&chunk)).collect()
    }

    #[test]
    fn speech_then_silence_flushes() {
        // 1s 语音（31 chunks）→ 静音：silence_limit=25 chunks，第 25 个静音触发收段
        // （段内含静音尾巴：31+25=56 chunks）
        let mut confs = vec![0.9; 31];
        confs.extend(vec![0.05; 31]);
        let mut p = make(&confs);
        let segs = feed(&mut p, 62);
        assert_eq!(segs.len(), 1, "一次语音爆发应产出一段");
        assert_eq!(segs[0].len(), 56 * 512);
        assert!(!p.is_speaking());
    }

    #[test]
    fn short_segment_kept_for_merge() {
        // 5 chunks 语音 + 25 chunks 静音 = 30 chunks < min(16000=31.25 chunks)
        // 且未 trim → 保留待合并（若语音 ≥7 chunks 则总数会过 min，直接收段）
        let mut confs = vec![0.9; 5];
        confs.extend(vec![0.05; 25]);
        let mut p = make(&confs);
        let segs = feed(&mut p, 30);
        assert!(segs.is_empty(), "短段应保留合并");
        assert!(!p.is_speaking());
        // 下一轮语音到来时合并出段（保留缓冲 30 + pre 3 + 新语音 30 + 静音 25）
        let confs2 = [vec![0.05; 5], vec![0.9; 30], vec![0.05; 40]].concat();
        p.conf = Script::new(&confs2);
        let segs = feed(&mut p, 70);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].len(), 88 * 512);
    }

    #[test]
    fn low_density_discarded() {
        // 长静音中偶发毛刺：密度 < 25% → 丢弃
        let mut confs = vec![0.05; 100];
        for i in [10usize, 30, 50] {
            confs[i] = 0.9;
        }
        let mut p = make(&confs);
        let segs = feed(&mut p, 100);
        assert!(segs.is_empty());
    }

    #[test]
    fn prespeech_buffer_prepended() {
        // 起始前的 3 个 chunk（低置信度）应随 pre_buffer 并入段头
        let mut confs = vec![0.1; 3];
        confs.extend(vec![0.9; 31]);
        confs.extend(vec![0.05; 30]);
        let mut p = make(&confs);
        let segs = feed(&mut p, 64);
        assert_eq!(segs.len(), 1);
        // 3 (pre) + 31 (语音) + 25 (静音尾巴) = 59 chunks；pre 的置信度被记为阈值 0.5
        assert_eq!(segs[0].len(), 59 * 512);
        assert_eq!(p.conf.i, 64);
    }

    #[test]
    fn max_duration_split_at_best_pause() {
        // 长语音中段插入真实低谷：max=2s 加速触发（64 chunks）
        let mut p = VadProcessor::new(Script::new(&[]), 16000, 0.5, 0.0, 2.0, 0.032);
        p.mode = "silero".into();
        // 前 60 高、中段 55-57 深谷（0.05）→ 平滑谷在尾部
        let mut confs = vec![0.9; 80];
        confs[55] = 0.05;
        confs[56] = 0.05;
        confs[57] = 0.05;
        p.conf = Script::new(&confs);
        let chunk = vec![0.5f32; 512];
        let mut segs = Vec::new();
        for _ in 0..80 {
            if let Some(s) = p.process_chunk(&chunk) {
                segs.push(s);
            }
        }
        assert!(!segs.is_empty(), "达到 max 必须出段");
        // 段边界应落在谷底附近（第 54-58 chunk），且余量仍在积累
        let first_samples: usize = segs[0].len();
        assert!(first_samples >= 54 * 512 && first_samples <= 58 * 512,
            "split at {} samples, want near chunk 56", first_samples / 512);
        assert!(p.speech_samples > 0, "余量应保留");
    }

    #[test]
    fn hard_flush_when_no_valley() {
        // 全高置信度（无谷）：dip_ratio ≥ 0.8 且 min_val ≥ avg → 找不到切分点 → 硬冲刷
        let mut p = VadProcessor::new(Script::new(&[]), 16000, 0.5, 0.0, 1.0, 0.032);
        p.mode = "silero".into();
        p.conf = Script::new(&[0.95; 40]);
        let chunk = vec![0.5f32; 512];
        let mut segs = Vec::new();
        for _ in 0..40 {
            if let Some(s) = p.process_chunk(&chunk) {
                segs.push(s);
            }
        }
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].len(), 32 * 512); // 1s = 32 chunks（第 33 个触发）
    }

    #[test]
    fn progressive_silence_tiers() {
        // 回读原版循环语义：tier 是"超过该秒数才应用乘数"，break 在第一个
        // 未超的 tier。故 <3s→1.0；3~10s→0.5（6s 档的 0.25 实际 ≥10s 才生效）。
        let mut p = make(&[]);
        p.silence_mode = "fixed".into();
        p.silence_limit = p.seconds_to_chunks(0.8); // 25
        p.speech_samples = 16000 * 2;
        assert_eq!(p.effective_silence_limit(), 25); // <3s → 全额
        p.speech_samples = 16000 * 4; // 4s：过 3s 档(1.0)、停在 6s 档
        assert_eq!(p.effective_silence_limit(), 25); // mult 仍 1.0
        p.speech_samples = 16000 * 7; // 7s：过 6s 档 → 0.5
        assert_eq!(p.effective_silence_limit(), 12); // 25*0.5=12.5 → banker's → 12
        p.speech_samples = 16000 * 11; // 11s：过 10s 档 → 0.25
        assert_eq!(p.effective_silence_limit(), 6); // 25*0.25=6.25 → 6
    }

    #[test]
    fn banker_rounding() {
        assert_eq!(VadProcessor::<Script>::py_round(12.5), 12.0); // 半到偶
        assert_eq!(VadProcessor::<Script>::py_round(13.5), 14.0);
        assert_eq!(VadProcessor::<Script>::py_round(0.4), 0.0);
        assert_eq!(VadProcessor::<Script>::py_round(0.6), 1.0);
        assert_eq!(VadProcessor::<Script>::py_round(2.5), 2.0);
    }

    #[test]
    fn adaptive_silence_updates_limit() {
        // pause_history 填 0.3s 停顿 → P75*1.2=0.36s → 11 chunks（< 默认 25）
        let mut p = make(&[]);
        p.silence_mode = "auto".into();
        for _ in 0..5 {
            p.push_pause(0.3);
        }
        p.update_adaptive_limit();
        assert_eq!(p.silence_limit, 11); // round(0.36/0.032)=11.25→11
    }

    #[test]
    fn trim_front_partial_and_flag() {
        let mut p = make(&[]);
        // 手工构造缓冲：3 chunks
        for _ in 0..3 {
            p.speech_buffer.push(vec![0.5f32; 512]);
            p.confidence_history.push(0.9);
            p.speech_samples += 512;
        }
        p.is_speaking = true;
        p.trim_front(512 + 100); // 第一个整块 + 第二块部分裁剪
        assert!(p.was_trimmed);
        assert_eq!(p.speech_samples, 2 * 512 - 100);
        assert_eq!(p.speech_buffer[0].len(), 512 - 100);
        // 全部裁空后 force_flush 应为 None
        p.trim_front(p.speech_samples);
        assert!(p.force_flush().is_none());
    }

    #[test]
    fn disabled_mode_confidence_constant() {
        let mut p = VadProcessor::new(DisabledVad, 16000, 0.5, 1.0, 15.0, 0.032);
        p.mode = "disabled".into();
        let chunk = vec![0.0f32; 512];
        let mut segs = Vec::new();
        for _ in 0..60 {
            if let Some(s) = p.process_chunk(&chunk) {
                segs.push(s);
            }
        }
        // disabled 恒 1.0 → 永远"在说话"，永不因静音收段（max 未到）
        assert_eq!(p.last_confidence, 1.0);
        assert!(segs.is_empty());
        assert!(p.is_speaking());
        // 手动 flush（≥min）出段
        let seg = p.flush().expect("60 chunks=30720 ≥ min 16000 应出段");
        assert_eq!(seg.len(), 60 * 512);
    }

    #[test]
    fn energy_mode_uses_half_threshold() {
        // energy 模式的有效阈值固定 0.5（非用户阈值！§5.1）
        let mut p = VadProcessor::new(EnergyVad { threshold: 0.02 }, 16000, 0.9, 1.0, 15.0, 0.032);
        p.mode = "energy".into();
        // rms≈0.03 → conf=0.03/0.04=0.75 ≥ 0.5 → 说话（用户阈值 0.9 会误判）
        let chunk = vec![0.03f32; 512];
        p.process_chunk(&chunk);
        assert!(p.is_speaking(), "energy 模式必须用 0.5 固定阈值");
    }

    #[test]
    fn peek_and_flush_semantics() {
        let confs = vec![0.9; 10];
        let mut p = make(&confs);
        let chunk = vec![0.5f32; 512];
        for _ in 0..10 {
            p.process_chunk(&chunk);
        }
        let (audio, dur, _gen) = p.peek_buffer().expect("说话中应有缓冲");
        assert_eq!(audio.len(), 10 * 512);
        assert!((dur - 0.32).abs() < 1e-9);
        // flush：min=1s 未达 → 丢弃
        assert!(p.flush().is_none());
        // 达标则收段
        let mut p2 = make(&[0.9; 40]);
        for _ in 0..40 {
            p2.process_chunk(&chunk);
        }
        // 40 chunks = 20480 ≥ min 16000，静音为 0 不自动收段
        assert!(p2.is_speaking());
        let seg = p2.flush().expect("达标应出段");
        assert_eq!(seg.len(), 40 * 512);
    }

    #[test]
    fn clamp_max_speech_only_qwen3_and_only_down() {
        // AH-8/D-28：仅 qwen3 生效且只向下收敛（update_settings 之后调用可收敛）
        let mut s = VadSettings::default();
        s.max_speech_duration = 30.0;
        let mut p = make(&[]);
        p.update_settings(&s);
        assert!(!p.clamp_max_speech("funasr", 15.0), "非 qwen3 不钳制");
        assert_eq!(p.max_speech_samples, (30.0f64 * 16000.0) as usize);
        let mut p2 = make(&[]);
        p2.update_settings(&s);
        assert!(p2.clamp_max_speech("qwen3", 15.0));
        assert_eq!(p2.max_speech_samples, (15.0f64 * 16000.0) as usize);
        // 已达上限不再动
        assert!(!p2.clamp_max_speech("qwen3", 15.0));
    }

    #[test]
    fn trim_checked_rejects_after_generation_bump() {
        // AH-4/D-27：同代裁剪成功；peek 后缓冲被收段/清空（reset 代际推进）
        // → 裁剪必须被拒绝（防误裁新段头部）
        let chunk = vec![0.5f32; 512];
        let mut p = make(&[0.9; 10]);
        for _ in 0..10 {
            p.process_chunk(&chunk);
        }
        let (_, _, gen) = p.peek_buffer().expect("说话中应有缓冲");
        assert!(p.trim_front_checked(512, gen), "同代裁剪应成功");

        let mut p2 = make(&[0.9; 10]);
        for _ in 0..10 {
            p2.process_chunk(&chunk);
        }
        let (_, _, gen2) = p2.peek_buffer().expect("说话中应有缓冲");
        p2.reset(); // 模拟识别期间 capture 线程侧收段（flush→reset）
        // 新语音已开始积累（新代际）
        for _ in 0..4 {
            p2.process_chunk(&chunk);
        }
        assert!(!p2.trim_front_checked(512, gen2), "代际推进后裁剪必须被拒绝");
        // 新段缓冲完好未裁（防误裁新段头部）
        assert_eq!(p2.speech_samples, 4 * 512);
    }
}
