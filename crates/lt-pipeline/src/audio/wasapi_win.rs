//! WASAPI 音频后端（Windows）：loopback 事件驱动采集 + 麦克风混合前缓冲。
//!
//! 移植基准：`LiveTranslate/audio_capture.py::AudioCapture._read_loop`，行为逐条对齐：
//! - 默认输出设备 2s 轮询比对，变更自动重启（原版 DEVICE_CHECK_INTERVAL）
//! - `__disabled__` 喂零推进（mic-only 模式），零块同样参与 mic 混合
//! - 麦克风先排空到 16k mono 缓冲，随 loopback chunk 等长混合（不足补零）
//! - 读错误 → 0.5s 后重启流；重启后清空 chunk 队列
//!
//! 差异（D 系记录）：原版用 pyaudiowpatch 轮询读；此处 loopback 同为轮询排空
//! ——WASAPI 回环采集不支持事件驱动（Initialize 带 EVENTCALLBACK 能成功但
//! 缓冲永远不进数据，实测 RMS 恒 0），无数据时空转 sleep 5ms（=原版 0.005s）。

use super::{mix_with_mic, resample_linear, to_mono, BoundedDropQueue, CHUNK_DURATION, CHUNK_SAMPLES, TARGET_RATE};
use anyhow::Context as _;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use wasapi::{initialize_mta, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

/// chunk 时长（秒）；与公共层常量一致
const CHUNK_SECS: f64 = CHUNK_DURATION;
/// 默认输出设备轮询间隔（原版 DEVICE_CHECK_INTERVAL）
const DEVICE_CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// loopback 无新数据空转间隔（原版 _read_loop 尾部 sleep(0.005)）
const POLL_IDLE_MS: u64 = 5;

/// 控制面 → 采集线程命令
#[derive(Debug)]
enum BackendCmd {
    SetDevice(Option<String>),
    SetMic(Option<String>),
}

/// Windows 后端句柄（控制面）
pub struct WasapiBackend {
    device: Option<String>,
    mic_device: Option<String>,
    running: Arc<AtomicBool>,
    cmd_tx: Option<crossbeam_channel::Sender<BackendCmd>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WasapiBackend {
    pub fn new() -> Self {
        Self {
            device: None,
            mic_device: None,
            running: Arc::new(AtomicBool::new(false)),
            cmd_tx: None,
            thread: None,
        }
    }
}

impl super::AudioBackend for WasapiBackend {
    fn list_output_devices(&self) -> anyhow::Result<Vec<String>> {
        // 枚举可能在任意线程调用（UI/音频线程），COM 按线程初始化（经验 E-09）
        let _ = initialize_mta();
        let en = DeviceEnumerator::new()?;
        names(&en, &Direction::Render)
    }

    fn list_input_devices(&self) -> anyhow::Result<Vec<String>> {
        let _ = initialize_mta();
        let en = DeviceEnumerator::new()?;
        names(&en, &Direction::Capture)
    }

    fn current_default_output(&self) -> anyhow::Result<Option<String>> {
        let _ = initialize_mta();
        let en = DeviceEnumerator::new()?;
        Ok(Some(en.get_default_device(&Direction::Render)?.get_friendlyname()?))
    }

    fn start(
        &mut self,
        device: Option<String>,
        mic_device: Option<String>,
        chunk_tx: Arc<BoundedDropQueue<(Vec<f32>, Option<f32>)>>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(self.thread.is_none(), "backend already started");
        self.device = device;
        self.mic_device = mic_device;
        self.running.store(true, Ordering::Relaxed);
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        self.cmd_tx = Some(cmd_tx);
        let running = self.running.clone();
        let device = self.device.clone();
        let mic = self.mic_device.clone();
        let h = std::thread::Builder::new()
            .name("lt-audio".into())
            .spawn(move || {
                let _ = initialize_mta();
                read_loop(device, mic, cmd_rx, chunk_tx, running);
                wasapi::deinitialize();
            })?;
        self.thread = Some(h);
        Ok(())
    }

    fn set_device(&mut self, device: Option<String>) {
        if device == self.device {
            return;
        }
        tracing::info!("Audio device changed: {:?} -> {:?}", self.device, device);
        self.device = device;
        if let Some(tx) = &self.cmd_tx {
            let _ = tx.send(BackendCmd::SetDevice(self.device.clone()));
        }
    }

    fn set_mic_device(&mut self, mic_device: Option<String>) {
        if mic_device == self.mic_device {
            return;
        }
        tracing::info!("Mic device changed: {:?} -> {:?}", self.mic_device, mic_device);
        self.mic_device = mic_device;
        if let Some(tx) = &self.cmd_tx {
            let _ = tx.send(BackendCmd::SetMic(self.mic_device.clone()));
        }
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
        self.cmd_tx = None;
    }
}

fn names(en: &DeviceEnumerator, dir: &Direction) -> anyhow::Result<Vec<String>> {
    let col = en.get_device_collection(dir)?;
    let mut out = Vec::new();
    for d in &col {
        out.push(d?.get_friendlyname()?);
    }
    Ok(out)
}

/// 按友好名在设备集合中查找（原版按名匹配语义）
fn find_device_by_name(
    en: &DeviceEnumerator,
    dir: &Direction,
    name: &str,
) -> anyhow::Result<Option<wasapi::Device>> {
    let col = en.get_device_collection(dir)?;
    for d in &col {
        let d = d?;
        if d.get_friendlyname()? == name {
            return Ok(Some(d));
        }
    }
    Ok(None)
}

// ─────────────────────────── 采集线程 ───────────────────────────

/// 已打开的 loopback 流（native 混合格式）
struct LoopStream {
    audio_client: wasapi::AudioClient,
    capture: wasapi::AudioCaptureClient,
    /// native 帧/chunk（= round(native_rate * CHUNK_SECS)，对齐原版 native_chunk）
    native_chunk_frames: usize,
    blockalign: usize,
    sample_type: SampleType,
    bits: u16,
    channels: usize,
    native_rate: u32,
    /// 待转换的字节积压
    bytes: VecDeque<u8>,
    /// 打开的设备友好名（默认设备比对基准）
    device_name: String,
}

struct MicStream {
    audio_client: wasapi::AudioClient,
    capture: wasapi::AudioCaptureClient,
    blockalign: usize,
    sample_type: SampleType,
    bits: u16,
    channels: usize,
    native_rate: u32,
}

fn open_loopback(target: Option<&str>) -> anyhow::Result<LoopStream> {
    let en = DeviceEnumerator::new()?;
    let device = match target {
        Some(name) => match find_device_by_name(&en, &Direction::Render, name)? {
            Some(d) => d,
            None => {
                tracing::warn!("指定输出设备未找到: {name}，回退系统默认");
                en.get_default_device(&Direction::Render)?
            }
        },
        None => en.get_default_device(&Direction::Render)?,
    };
    let name = device.get_friendlyname()?;
    let mut audio_client = device.get_iaudioclient()?;
    let fmt: WaveFormat = audio_client.get_mixformat()?;
    let (def_time, _min) = audio_client.get_device_period()?;
    // render 设备 + Capture 方向 = loopback（crate 自动加 AUDCLNT_STREAMFLAGS_LOOPBACK）；
    // 必须轮询模式：回环流的事件句柄永不触发且缓冲不进数据（见模块头 D 系记录）
    let mode = StreamMode::PollingShared {
        autoconvert: false,
        buffer_duration_hns: def_time,
    };
    audio_client.initialize_client(&fmt, &Direction::Capture, &mode)?;
    let capture = audio_client.get_audiocaptureclient()?;
    audio_client.start_stream()?;

    let native_rate = fmt.get_samplespersec();
    let channels = fmt.get_nchannels() as usize;
    tracing::info!(
        "Loopback device: {name} (native {native_rate}Hz, {channels}ch, {}bit) -> {TARGET_RATE}Hz mono",
        fmt.get_bitspersample(),
    );
    Ok(LoopStream {
        audio_client,
        capture,
        native_chunk_frames: (native_rate as f64 * CHUNK_SECS).round() as usize,
        blockalign: fmt.get_blockalign() as usize,
        sample_type: fmt.get_subformat().unwrap_or(SampleType::Float),
        bits: fmt.get_bitspersample(),
        channels,
        native_rate,
        bytes: VecDeque::new(),
        device_name: name,
    })
}

fn open_mic(target: Option<&str>) -> anyhow::Result<MicStream> {
    let en = DeviceEnumerator::new()?;
    let device = match target {
        None | Some("__default__") | Some("default") => en.get_default_device(&Direction::Capture)?,
        Some(name) => {
            find_device_by_name(&en, &Direction::Capture, name)?
                .with_context(|| format!("麦克风设备未找到: {name}"))?
        }
    };
    let name = device.get_friendlyname()?;
    let mut audio_client = device.get_iaudioclient()?;
    let fmt = audio_client.get_mixformat()?;
    let (def_time, _min) = audio_client.get_device_period()?;
    // mic 由主循环轮询排空，无需事件
    let mode = StreamMode::PollingShared {
        autoconvert: false,
        buffer_duration_hns: def_time,
    };
    audio_client.initialize_client(&fmt, &Direction::Capture, &mode)?;
    let capture = audio_client.get_audiocaptureclient()?;
    audio_client.start_stream()?;
    tracing::info!(
        "Mic device: {name} ({}Hz, {}ch)",
        fmt.get_samplespersec(),
        fmt.get_nchannels()
    );
    Ok(MicStream {
        audio_client,
        capture,
        blockalign: fmt.get_blockalign() as usize,
        sample_type: fmt.get_subformat().unwrap_or(SampleType::Float),
        bits: fmt.get_bitspersample(),
        channels: fmt.get_nchannels() as usize,
        native_rate: fmt.get_samplespersec(),
    })
}

fn close_loopback(st: &mut Option<LoopStream>) {
    if let Some(s) = st {
        let _ = s.audio_client.stop_stream();
    }
    *st = None;
}

fn close_mic(mic: &mut Option<MicStream>) {
    if let Some(m) = mic {
        let _ = m.audio_client.stop_stream();
    }
    *mic = None;
}

/// 读一个 WASAPI 包追加到积压；无包返回 false。
/// 注意：GetNextPacketSize 无包时返回 Some(0)（crate 不转 None），必须显式拦 0 帧。
fn read_packet(st: &mut LoopStream) -> anyhow::Result<bool> {
    let Some(frames) = st.capture.get_next_packet_size()? else {
        return Ok(false);
    };
    if frames == 0 {
        return Ok(false);
    }
    let mut buf = vec![0u8; frames as usize * st.blockalign];
    st.capture.read_from_device(&mut buf)?;
    st.bytes.extend(buf);
    Ok(true)
}

/// 按 subformat 解码为 f32 interleaved
fn decode_samples(bytes: &[u8], st: SampleType, bits: u16) -> Vec<f32> {
    match (st, bits) {
        (SampleType::Float, 32) => bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        (SampleType::Float, 64) => bytes
            .chunks_exact(8)
            .map(|c| f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]) as f32)
            .collect(),
        (SampleType::Int, 16) => bytes
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
            .collect(),
        (SampleType::Int, 32) => bytes
            .chunks_exact(4)
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2147483648.0)
            .collect(),
        _ => {
            tracing::error!("不支持的采样格式: {st:?} {bits}bit，按静音处理");
            vec![0.0; bytes.len() / 4]
        }
    }
}

/// 字节积压 → 完整 native chunk → 16k mono
fn take_native_chunks(st: &mut LoopStream, out: &mut Vec<Vec<f32>>) {
    let chunk_bytes = st.native_chunk_frames * st.blockalign;
    while st.bytes.len() >= chunk_bytes {
        let raw: Vec<u8> = st.bytes.drain(..chunk_bytes).collect();
        let inter = decode_samples(&raw, st.sample_type, st.bits);
        let mono = to_mono(&inter, st.channels);
        out.push(resample_linear(&mono, st.native_rate, TARGET_RATE));
    }
}

/// mic 流排空到积压（原版：一次读光 get_read_available）。
/// 返回 `false` = 读失败（AH-7/H12：调用方 warn+退避重开，镜像 loopback
/// 恢复语义——原实现静默 break，mic 被抢占后无声消失且无任何日志）
fn drain_mic(mic: &mut MicStream, mic_buf: &mut Vec<f32>) -> bool {
    loop {
        let Ok(Some(frames)) = mic.capture.get_next_packet_size() else {
            return true;
        };
        if frames == 0 {
            return true;
        }
        let mut buf = vec![0u8; frames as usize * mic.blockalign];
        if mic.capture.read_from_device(&mut buf).is_err() {
            return false;
        }
        let inter = decode_samples(&buf, mic.sample_type, mic.bits);
        let mono = to_mono(&inter, mic.channels);
        mic_buf.extend(resample_linear(&mono, mic.native_rate, TARGET_RATE));
    }
}

fn query_default_output_name() -> anyhow::Result<String> {
    let en = DeviceEnumerator::new()?;
    en.get_default_device(&Direction::Render)?.get_friendlyname().map_err(Into::into)
}

/// 采集线程主循环（原版 _read_loop 1:1）
fn read_loop(
    device: Option<String>,
    mic_device: Option<String>,
    cmd_rx: crossbeam_channel::Receiver<BackendCmd>,
    chunk_tx: Arc<BoundedDropQueue<(Vec<f32>, Option<f32>)>>,
    running: Arc<AtomicBool>,
) {
    let mut requested_device = device;
    let mut requested_mic = mic_device;
    let loopback_disabled = |dev: &Option<String>| dev.as_deref() == Some("__disabled__");

    let mut st: Option<LoopStream> = None;
    if !loopback_disabled(&requested_device) {
        st = open_loopback(requested_device.as_deref())
            .map_err(|e| tracing::error!("打开 loopback 失败: {e:#}"))
            .ok();
    } else {
        tracing::info!("Loopback disabled (mic-only mode)");
    }
    let mut mic: Option<MicStream> = None;
    if requested_mic.is_some() {
        mic = open_mic(requested_mic.as_deref())
            .map_err(|e| tracing::warn!("打开麦克风失败: {e:#}"))
            .ok();
    }
    let mut mic_buf: Vec<f32> = Vec::new();
    let mut last_device_check = Instant::now();
    // AH-7/H12：mic 读失败 → warn + 0.5s 退避重开（镜像 loopback 读错误恢复）；
    // 不清 chunk 队列（loopback 未受影响）。requested_mic 经参传入避免闭包
    // 与 SetMic 命令臂的可变借用冲突
    let handle_mic =
        |mic: &mut Option<MicStream>, mic_buf: &mut Vec<f32>, requested: &Option<String>| {
            if let Some(m) = mic {
                if !drain_mic(m, mic_buf) {
                    tracing::warn!("麦克风读取失败（设备可能被移除/抢占），0.5s 后尝试重开");
                    std::thread::sleep(Duration::from_millis(500));
                    close_mic(mic);
                    mic_buf.clear();
                    *mic = open_mic(requested.as_deref())
                        .map_err(|e| tracing::error!("读错误后重开麦克风失败: {e:#}"))
                        .ok();
                }
            }
        };

    while running.load(Ordering::Relaxed) {
        // ── 控制命令（原版 restart_event / mic_restart_event 分支）──
        if let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                BackendCmd::SetDevice(d) => {
                    requested_device = d;
                    close_loopback(&mut st);
                    close_mic(&mut mic);
                    mic_buf.clear();
                    if !loopback_disabled(&requested_device) {
                        st = open_loopback(requested_device.as_deref())
                            .map_err(|e| tracing::error!("重启 loopback 失败: {e:#}"))
                            .ok();
                    } else {
                        tracing::info!("Loopback disabled (mic-only mode)");
                    }
                    if requested_mic.is_some() {
                        mic = open_mic(requested_mic.as_deref())
                            .map_err(|e| tracing::warn!("重开麦克风失败: {e:#}"))
                            .ok();
                    }
                    chunk_tx.clear();
                    if let Some(s) = &st {
                        tracing::info!("Audio capture restarted on: {}", s.device_name);
                    }
                }
                BackendCmd::SetMic(m) => {
                    requested_mic = m;
                    close_mic(&mut mic);
                    mic_buf.clear();
                    if requested_mic.is_some() {
                        mic = open_mic(requested_mic.as_deref())
                            .map_err(|e| tracing::error!("打开麦克风失败: {e:#}"))
                            .ok();
                    } else {
                        tracing::info!("Mic disabled");
                    }
                }
            }
            continue;
        }

        // ── 默认设备 2s 轮询（仅系统默认模式；原版 _query_current_default）──
        if requested_device.is_none() && last_device_check.elapsed() >= DEVICE_CHECK_INTERVAL {
            last_device_check = Instant::now();
            match query_default_output_name() {
                Ok(current) => {
                    let changed = match &st {
                        Some(s) => !s.device_name.contains(&current),
                        None => true,
                    };
                    if changed {
                        tracing::info!("System default output changed -> {current}; restarting capture...");
                        close_loopback(&mut st);
                        st = open_loopback(None)
                            .map_err(|e| tracing::error!("设备变更重启失败: {e:#}"))
                            .ok();
                        chunk_tx.clear();
                        if let Some(s) = &st {
                            tracing::info!("Audio capture restarted on: {}", s.device_name);
                        }
                    }
                }
                Err(e) => tracing::warn!("Device check error: {e:#}"),
            }
        }

        // ── 本轮产出的 loopback chunk（0..N 个）──
        let mut produced: Vec<Vec<f32>> = Vec::new();
        if loopback_disabled(&requested_device) {
            // mic-only：按 chunk 节拍喂零（原版 sleep(chunk_duration) + zeros）
            std::thread::sleep(Duration::from_secs_f64(CHUNK_SECS));
            produced.push(vec![0.0; CHUNK_SAMPLES]);
        } else {
            match st.as_mut() {
                None => {
                    // 打开失败重试路径
                    st = open_loopback(requested_device.as_deref())
                        .map_err(|e| tracing::error!("重试打开 loopback: {e:#}"))
                        .ok();
                    if st.is_none() {
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    continue;
                }
                Some(s) => {
                    // 轮询排空（回环无事件通知；5ms 空转对齐原版 sleep(0.005)）
                    let drained = (|| -> anyhow::Result<bool> {
                        while read_packet(s)? {}
                        Ok(true)
                    })();
                    match drained {
                        Ok(_) => take_native_chunks(s, &mut produced),
                        Err(e) => {
                            tracing::warn!("Read error (device may have changed): {e:#}");
                            std::thread::sleep(Duration::from_millis(500));
                            close_loopback(&mut st);
                            st = open_loopback(requested_device.as_deref())
                                .map_err(|e| tracing::error!("读错误后重启失败: {e:#}"))
                                .ok();
                            chunk_tx.clear();
                            continue;
                        }
                    }
                    if produced.is_empty() {
                        // 无新数据（原版 sleep(0.005) continue）
                        handle_mic(&mut mic, &mut mic_buf, &requested_mic);
                        std::thread::sleep(Duration::from_millis(POLL_IDLE_MS));
                        continue;
                    }
                }
            }
        }

        // ── mic 排水 → 逐块混合推送（原版尾部逻辑）──
        handle_mic(&mut mic, &mut mic_buf, &requested_mic);
        for chunk in produced {
            let (mixed, mic_rms) = mix_with_mic(&chunk, &mut mic_buf);
            chunk_tx.push((mixed, mic_rms));
        }
    }

    close_loopback(&mut st);
    close_mic(&mut mic);
    tracing::info!("Audio capture stopped");
}
