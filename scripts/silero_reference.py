"""Silero VAD 置信度对照（M1.4 完成标准：与 Rust 实现最大误差 < 1e-4）。

用 onnxruntime 逐窗口复算 Rust example（vad_check）导出的同一段输入，
state 逐 chunk 回喂（与 Rust/裸 ONNX 语义一致），输出逐点误差。

用法：先跑 `cargo run -p lt-pipeline --example vad_check`，再运行本脚本。
"""

import json
import sys
from pathlib import Path

import numpy as np
import onnxruntime as ort

ROOT = Path(__file__).parent.parent
MODEL = ROOT / "assets" / "silero_vad.onnx"
INPUT = ROOT / "target" / "vad_input.f32"
RUST = ROOT / "target" / "vad_conf_rust.json"

sr = 16000
session = ort.InferenceSession(str(MODEL), providers=["CPUExecutionProvider"])
print("inputs :", [(i.name, i.shape, i.type) for i in session.get_inputs()])
print("outputs:", [(o.name, o.shape, o.type) for o in session.get_outputs()])

audio = np.fromfile(INPUT, dtype=np.float32)
rust = json.loads(RUST.read_text())

state = np.zeros((2, 1, 128), dtype=np.float32)
win = 512
confs = []
for off in range(0, len(audio), win):
    chunk = audio[off : off + win]
    if len(chunk) < win:
        chunk = np.pad(chunk, (0, win - len(chunk)))
    out, state = session.run(
        None,
        {
            "input": chunk[None, :].astype(np.float32),
            "state": state,
            "sr": np.array(sr, dtype=np.int64),
        },
    )
    confs.append(float(out[0, 0]))

diffs = np.abs(np.array(confs) - np.array(rust))
print(f"chunks={len(confs)} max_abs_err={diffs.max():.3e} mean={diffs.mean():.3e}")
if diffs.max() < 1e-4:
    print("PASS: <1e-4")
else:
    print("FAIL: ≥1e-4")
    for i in np.argsort(diffs)[-5:][::-1]:
        print(f"  chunk {i}: rust={rust[i]:.6f} py={confs[i]:.6f}")
    sys.exit(1)
