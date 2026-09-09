"""生成音频公共层单测对照数据（numpy 参考，逐位一致）。

复刻 LiveTranslate/audio_capture.py::_resample_to_mono 的精确算术顺序，
把输入/期望输出以 f32 LE 落盘，供 tests/audio_common.rs 读取比对。

用法：python tests/gen_fixtures.py（在 tests/fixtures/ 下生成）
"""

import json
import struct
from pathlib import Path

import numpy as np

FIX = Path(__file__).parent / "fixtures"
FIX.mkdir(exist_ok=True)


def make_signal(n: int, rate: int, seed: int = 0) -> np.ndarray:
    """确定性信号：正弦叠加 + LCG 抖动，float32。"""
    t = np.arange(n, dtype=np.float64)
    wave = 0.5 * np.sin(2 * np.pi * 440.0 * t / rate) + 0.25 * np.sin(
        2 * np.pi * 1320.0 * t / rate + 0.7
    )
    state = (seed * 6364136223846793005 + 1442695040888963407) % (1 << 64)

    def lcg():
        nonlocal state
        state = (state * 6364136223846793005 + 1442695040888963407) % (1 << 64)
        return (state >> 33) / float(1 << 31) - 1.0

    jitter = np.array([lcg() for _ in range(n)], dtype=np.float64)
    return (wave + 0.05 * jitter).astype(np.float32)


def to_mono(data: np.ndarray, channels: int) -> np.ndarray:
    """原版：interleaved → 各通道平均（float32 累加）。"""
    if channels > 1:
        return data.reshape(-1, channels).mean(axis=1)
    return data


def resample_linear(audio: np.ndarray, native_rate: int, target: int = 16000) -> np.ndarray:
    """原版：floor/ceil clamp + frac 线性插值（索引 f64、样本 f32）。"""
    if native_rate == target:
        return audio
    ratio = target / native_rate
    n_out = int(len(audio) * ratio)
    indices = np.arange(n_out) / ratio
    indices = np.clip(indices, 0, len(audio) - 1)
    idx_floor = indices.astype(np.int64)
    idx_ceil = np.minimum(idx_floor + 1, len(audio) - 1)
    frac = (indices - idx_floor).astype(np.float32)
    return audio[idx_floor] * (1 - frac) + audio[idx_ceil] * frac


def write_f32(path: Path, arr: np.ndarray) -> None:
    arr.astype("<f4").tofile(path)


cases = [
    # (名字, native_rate, channels, frames)
    ("res_44100_st2", 44100, 2, 1024),
    ("res_48000_st2", 48000, 2, 480),
    ("res_48000_st6", 48000, 6, 720),
    ("res_44100_mono", 44100, 1, 1024),
    ("res_32000_mono", 32000, 1, 1023),  # 非整倍：int() 截断路径
    ("res_16000_mono", 16000, 1, 512),   # 恒等路径
    ("res_8000_mono", 8000, 1, 256),     # 上采样路径
]

meta = {"cases": []}
for name, rate, ch, frames in cases:
    inter = np.stack([make_signal(frames, rate, seed=i) for i in range(ch)], axis=1).reshape(-1)
    assert inter.dtype == np.float32 and inter.shape == (frames * ch,)
    write_f32(FIX / f"{name}.in.bin", inter)
    mono = to_mono(inter, ch)
    out = resample_linear(mono, rate)
    write_f32(FIX / f"{name}.out.bin", out)
    meta["cases"].append({"name": name, "rate": rate, "channels": ch, "frames": frames,
                          "in_n": int(inter.size), "out_n": int(out.size)})

# 混音：loopback 512 样本 + mic 缓冲 300 样本（不足补零）→ 混合 + mic_rms
loop = make_signal(512, 16000, seed=11)
mic_full = make_signal(300, 16000, seed=12)
mic_chunk = np.zeros(512, dtype=np.float32)
mic_chunk[:300] = mic_full
mixed = loop + mic_chunk
mic_rms = np.float32(np.sqrt(np.mean(mic_chunk**2)))
loop_rms = np.float32(np.sqrt(np.mean(loop**2)))
write_f32(FIX / "mix_loop.bin", loop)
write_f32(FIX / "mix_mic.bin", mic_full)
write_f32(FIX / "mix_out.bin", mixed)
meta["mix"] = {"loop_n": 512, "mic_n": 300, "mic_rms": float(mic_rms), "loop_rms": float(loop_rms)}

# pad_bucket：16000 * 0.5s = 8000；取 8192 样本 → pad 到 16000；取 16000 → 原样
pad_in1 = make_signal(8192, 16000, seed=13)
pad_out1 = np.concatenate([pad_in1, np.zeros(16000 - 8192, dtype=np.float32)])
write_f32(FIX / "pad_in.bin", pad_in1)
write_f32(FIX / "pad_out.bin", pad_out1)
pad_in2 = make_signal(16000, 16000, seed=14)
write_f32(FIX / "pad_exact_in.bin", pad_in2)
meta["pad"] = {"quantum": 8000, "in_n": 8192, "out_n": 16000, "exact_n": 16000}

(FIX / "meta.json").write_text(json.dumps(meta, indent=1), encoding="utf-8")
print("fixtures written:", [c["name"] for c in meta["cases"]])
print("mic_rms =", mic_rms, "loop_rms =", loop_rms)
