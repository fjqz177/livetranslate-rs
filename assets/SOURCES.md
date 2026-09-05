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
