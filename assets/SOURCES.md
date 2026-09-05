# 资产来源记录（PLAN.md §2.5 要求：入库资产记录来源与 sha256）

## silero_vad.onnx

- 模型：Silero VAD v5（ONNX 导出版，f32）
- 来源：ModelScope 镜像 `pengzhendong/silero-vad`（根目录 `silero_vad.onnx`，与上游
  `snakers4/silero-vad` master `src/silero_vad/data/silero_vad.onnx` 同一文件）
- 下载端点：`https://modelscope.cn/api/v1/models/pengzhendong/silero-vad/repo?Revision=master&FilePath=silero_vad.onnx`
- 大小：2,327,524 字节
- sha256：`2623a2953f6ff3d2c1e61740c6cdb7168133479b267dfef114a4a3cc5bdd788f`
- I/O 契约（v5，首次使用前以 examples/vad_check.rs 实际打印为准）：
  输入 `input [1,512] f32` + `state [2,1,128] f32` + `sr int64 标量`；
  输出 `output [1,1] f32` + `stateN [2,1,128] f32`（state 必须自持并逐 chunk 回喂）

## ort/onnxruntime.dll

- 用途：ort crate `load-dynamic` 运行时库（主进程 VAD 专用；worker 子进程用 sherpa 静态 ORT，
  每进程仅一份 ORT——R-4 预案 A 的落地形态，保持单 exe：dll 内嵌并首次运行解压）
- 版本：ONNX Runtime 1.29.0（C API 向后兼容，覆盖 ort 2.0-rc.13 所需 API 22）
- 来源：PyPI `onnxruntime-1.29.0` win_amd64 wheel 内 capi/onnxruntime.dll
  （= 微软官方 release 产物，未改动）
- sha256：见 git 历史（assets/SOURCES.md 随资产入库）
