# 资产来源记录（docs/archive/rewrite-plan.md §2.5 要求：入库资产记录来源与 sha256）

## silero_vad.onnx

- 模型：Silero VAD v5（ONNX 导出版，f32）——v5 末态，精确版本 = PyPI `silero-vad==5.1.2`
- 版本身份（2026-09-24 四方实测闭环，sha256 逐位一致）：本仓入库份 = PyPI 官方包
  `silero-vad==5.1.2` wheel 内 `silero_vad/data/silero_vad.onnx` = GitHub 官方仓
  `snakers4/silero-vad` commit `f0d880d`（2024-07-09 "make package structure"，v5 末态）=
  ModelScope 镜像 `pengzhendong/silero-vad`。上游 master 已于 2025-11-06 换入 v6.2
  （sha256 `1a153a22…`，I/O 契约未验证）——**一切引用钉 v5，禁追 master**。
  （HF 上无 Silero 官方库；`onnx-community/silero-vad` 为第三方重导出版，大小/哈希均不符。）
- 复现 / 重取：`uv sync`（pyproject dev 组钉版 `silero-vad==5.1.2`，装进仓库内 .venv）；
  仓库内位置 = `.venv\Lib\site-packages\silero_vad\data\`（gitignored，构建期由
  vad.rs include_bytes! 内嵌）
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
  （= 微软官方 release 产物，未改动；2026-09-24 实测 wheel 内 DLL 与入库份逐位一致）
- 复现 / 重取：`uv sync`（pyproject dev 组钉版 `onnxruntime==1.29.0`，装进仓库内 .venv）；
  仓库内位置 = `.venv\Lib\site-packages\onnxruntime\capi\`（gitignored，构建期由
  vad.rs include_bytes! 内嵌）
- sha256：`4075344f1057c0d16883fb5a93428aeac5fecfbcb24e1560defb1cb80a6e0a85`（18,093,368 B，2026-09-08 实测入库文件）

## icons/（应用与托盘图标，2026-09-06 由用户提供的原版工作区副本 icons/ 搬入）

- `app.icns`（78,542 B）sha256 `958b4c63e2f2983d01b8aeca1e166662ddbda746f8740fdfd91ee87e42a7534b`
- `app.ico`（27,917 B）sha256 `87881526136933bdaf60441ca38c3ff68686a9c7857fe720ef946054eb6f6b7a`
- `app.png`（25,127 B）sha256 `87348eb2aae2216a97baac3871003d33a20edc269fc858771e773e6ff3cb0542`
- `app_128.png`（6,304 B）sha256 `eedd09a9a234f43826b16a33060c8d9801f68c1fb4bc5d3a18b628a5cd0fe98c`
- `app_16.png`（644 B）sha256 `d92d588a689f9b0618b0f998d670d0d2afc14aa2ca3f75dbbcde26d9ab41ba08`
- `app_24.png`（999 B）sha256 `41f76e2bd47000a1eaa7c24f2afaca5de7784ecf2ce4260b8de38e844487e586`
- `app_256.png`（13,009 B）sha256 `31b2ae2466dbddd32d6d7651684fb47aa37aa8b5422f0c4e895b22a2fd93d3eb`
- `app_32.png`（1,399 B）sha256 `fe063de43e622da09d342da20e1c7273d324b5cee7c477b58197c0fa6f00afb4`
- `app_48.png`（2,255 B）sha256 `28c3dedffd444ff2eda267d7c0635f42402773ac7277bc6ade16702f707fdec8`
- `app_512.png`（25,127 B）sha256 `87348eb2aae2216a97baac3871003d33a20edc269fc858771e773e6ff3cb0542`
- `app_64.png`（2,958 B）sha256 `3c8f772f7f8d37282f5a160ff3ca2f23bea3bf7e045861d78a816d3588732111`
- `spin_down_dark.png`（133 B）sha256 `904ce26d0b7b47a6bea4b9bc0890fae80d80354a3a5a7811314f4e864bd79217`
- `spin_down_light.png`（133 B）sha256 `d6d44dd6696fff1bcc304627007c294a56550c4bd198ed008bf4b3521525d29f`
- `spin_up_dark.png`（126 B）sha256 `6018152a4fe19b4309a55a7105568b54d6bfdbf1129fbdbbbb4fd3614432cda8`
- `spin_up_light.png`（126 B）sha256 `be345d7e2d3709f51f4fbfef383b149c0f9e7ba1cda6730f20adac6d12bd8d80`
- `tray_error_128.png`（7,648 B）sha256 `e7cac03198299259b24988ce8d1eae790c3127829b2b8aea2ae0020704ccfa8b`
- `tray_error_16.png`（717 B）sha256 `66c2cda6b0f7651709acd3b4dda0ac8aca6dbc8365191e2e318b44d8eb703212`
- `tray_error_24.png`（1,176 B）sha256 `25d199db3cd05c21da958c77db59fbb163ae2318ae0669bcf534190d0420ccef`
- `tray_error_256.png`（15,677 B）sha256 `ac11978fd57042555cd3ff378b6088795abd9183ccabf5bca147c7c32e4fd3f1`
- `tray_error_32.png`（1,702 B）sha256 `1b8aab201a6dae8e021e28e0f2b59255c0ae74c4a940557be3e68e5f07b9ca1b`
- `tray_error_48.png`（2,691 B）sha256 `e545794a61f7bd550212802060be07379f2c7e2cf8c2e1a4aea5d3d6fe922d4e`
- `tray_error_512.png`（29,833 B）sha256 `ef521aa68ccae79364f78d5b943701edf6724005819b58ea19e1c0a21ef1e205`
- `tray_error_64.png`（3,562 B）sha256 `3cddbea895923cc88eb305e67afc99a5f554577786cdf86f87cbe062ff048888`
- `tray_pause_128.png`（7,540 B）sha256 `adf3a3545e1bbeae3bd1307ead844dd3a22523f497667930e5834dbb04ef3194`
- `tray_pause_16.png`（715 B）sha256 `e1e244b49d9c224b678dd930e8cd08cac7c06a4badddf872d2d752df6c7680a7`
- `tray_pause_24.png`（1,170 B）sha256 `b6307cf0483380fd5e9e8183cdecd2c566ea9e92bc5f300e4a082a50e73c25d0`
- `tray_pause_256.png`（15,506 B）sha256 `4f71eb26f8f2d0aed98a9e9cb5bdf7fc1a0b8cfa6960a32bf125de9046e58101`
- `tray_pause_32.png`（1,681 B）sha256 `e358a41735d651cb7d8ec228fdad7b86bb0a297b44f6b3f5827b90316994c0cd`
- `tray_pause_48.png`（2,649 B）sha256 `27f2bf09fde64aa8a63595e1bfd4c5f8624d41d4fecad2847b56cd542205f5cf`
- `tray_pause_512.png`（29,815 B）sha256 `f388a98afe6438f22db963f36053ceaa281f72524bd641b2dde19f6ab5f4335c`
- `tray_pause_64.png`（3,498 B）sha256 `47e2f5ec8be9be9b13d6b2b2743a0e6983cbdcae0969a2f8048e15193f085cad`
- `tray_run_128.png`（7,565 B）sha256 `f4971a7da279f149a728de3061704a31e282b94c11f35f749e2904538d4c703e`
- `tray_run_16.png`（716 B）sha256 `960d86dc80dcc6487962a29ec17c7c06bf170dbbe9352ec7419565d498d628e4`
- `tray_run_24.png`（1,172 B）sha256 `49992c8b7f8d60e50f5fbeb77e4cedc7333963a4ecd8e49b41df5ad7422f7641`
- `tray_run_256.png`（15,439 B）sha256 `9b71beb25d75d329d5463845f08fe3c88952117b104b82fc61888d9fbe96542f`
- `tray_run_32.png`（1,679 B）sha256 `550050f1de42a67f59d0a11390c7f7cffcdb724213726e8500ffd699263601dc`
- `tray_run_48.png`（2,663 B）sha256 `2e66515831bf3a43f23b7e77e09f0f8ae5ad72d9439642eaadf39a861606e46e`
- `tray_run_512.png`（29,485 B）sha256 `5002f142262e7073fda77d792889959f3c7ffc46e9ba20d267592bff6700414b`
- `tray_run_64.png`（3,485 B）sha256 `3a50fd4dad8f0813607e074206bcec1243feaef2a44e898e690da1e914969b6d`

## fonts/（W-7 定稿：brotli 压缩资产，运行时一次性解压——字形零损失）

体积策略：全量 OTF/TTF 内嵌共 ~34MB → brotli(q11) 入库 ~12.7MB（exe 中同量）；
解压发生在进程内一次（启动 +~100ms），解压后 `FontData::from_static` 零拷贝。

### 压缩资产（仓库实存）

| 文件 | 大小 | 压缩体 sha256 |
|---|---|---|
| `NotoSansCJKsc-Regular.otf.br` | 11,457,451 B | `d35b738332ab714af2511b8b3cd73f3bc164cb1b519406738dfce4ee0020cd02` |
| `NotoSansMono-VF.ttf.br` | 761,491 B | 118384f344fdc8f3d664a3c5fcf49424bdb9793e2c3b60af43270a13420aeec8 |
| `NotoSansSymbols2-Regular.ttf.br` | 435,506 B | 7ab1b92eae75a194d8ba653e06ab0e60af902d1a6154a082673f87e4c14e5b27 |
| `NotoSansArabic-VF.ttf.br` | 368,819 B | 311340b84de7170f63af6dd1e127a0748dee80c1a4715e1776621234c86d5dc1 |
| `NotoSansThai-VF.ttf.br` | 111,668 B | b90b71e4abeae0ee481cfe75b75ef4e0df5e7f245c2587ddf912018b9bac5829 |
| `NotoSansDevanagari-VF.ttf.br` | 271,833 B | 4ac4207bd705aa2cb4b269088d63cc49ef6da12c4de8740af4acc8708fd3e8d5 |
| `NotoSansHebrew-VF.ttf.br` | 61,973 B | 2d1c7382661c651a8b1345c6c3ae84172c33fc6c854953ea6eb704d132876caf |
| `OFL.txt` | 4,388 B | 1c05c68c34f9708415aada51f17e1b0092d2cea709bf4a94cd38114f9e73d7d9 |

复现配方（任意机器可验证）：

```bash
# 解压（需 brotli/pip install brotli，或 brotli CLI）
python -c "import brotli;open('NotoSansCJKsc-Regular.otf','wb').write(brotli.decompress(open('NotoSansCJKsc-Regular.otf.br','rb').read()))"
# 压缩（重建资产）
python -c "import brotli;open('x.br','wb').write(brotli.compress(open('x','rb').read(),quality=11))"
```

### 原始字体来源（解压后 sha256 可核对上游）

- `NotoSansCJKsc-Regular.otf`（解压后 16,437,364 B）sha256 `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`
  - 思源黑体 SC（Noto Sans CJK SC = Adobe Source Han Sans SC 同设计；区域包全字库：汉字/假名/韩文音节/拉丁）
  - notofonts/noto-cjk 发布 Sans2.004 资产 `08_NotoSansCJKsc.zip`：`https://github.com/notofonts/noto-cjk/releases/tag/Sans2.004`
- `NotoSansMono-VF.ttf`（解压后 1,708,408 B）sha256 `2cb2adb378a8f574e21323df697050b83c54c27df465a2015552740b2769a081`
  - Noto Sans Mono 变量字体（默认实例 Regular，等宽 chrome 保证回退；Consolas 为微软字体不可重分发，仅系统增强）
  - google/fonts 仓 `ofl/notosansmono/`（镜像 jsDelivr：`https://cdn.jsdelivr.net/gh/google/fonts@main/ofl/notosansmono/NotoSansMono%5Bwdth%2Cwght%5D.ttf`）
- `NotoSansSymbols2-Regular.ttf`（解压后 1,233,128 B）sha256 `7d5fb73b7ca67a6798101741f5d280a3d016a56a197afcd4199dbb57b4b82a21`
  - Noto Sans Symbols 2 Regular（补 ✗ 等思源缺失符号；segui_sym 微软字体已移出）
  - google/fonts 仓 `ofl/notosanssymbols2/`（镜像 jsDelivr 同目录）
- `NotoSansArabic-VF.ttf`（解压后 844,676 B）sha256 `63111b5b2e074dd48cc67692e0a2726d86ee94c1c37fe8598257b7b4e87e869e`
- `NotoSansThai-VF.ttf`（解压后 218,652 B）sha256 `5a1c559bb539583c8a1fd99d1c5b9491e5e14478c9cd2bd0970d5c3096cc9ef8`
- `NotoSansDevanagari-VF.ttf`（解压后 641,944 B）sha256 `14ec4af41f27482216d1c2229f417ff9b1425e1babb014e57d1d40d03229853e`
- `NotoSansHebrew-VF.ttf`（解压后 112,640 B）sha256 `7ef36a2c3593758cdb622e1bdef4f84523e92fbc3ccc667438dd80ff54c2de88`
  - 以上四件 = D-127 脚本补字（语言下拉 ar/th/hi/he 本文名 + 该四语言转写/译文/日志渲染；阿拉伯/泰/天城文/希伯来，默认实例 Regular）
  - google/fonts 仓 `ofl/notosansarabic|notosansthai|notosansdevanagari|notosanshebrew/`
    （镜像 jsDelivr 同目录，文件名 `NotoSans<Script>%5Bwdth%2Cwght%5D.ttf`）
- 许可：七者均 SIL Open Font License 1.1（OFL.txt 随附）
- 裁剪记录（2026-09-07）：NotoSansMonoCJKsc 已移除——等宽 chrome 的中文回落思源（与有 Consolas 机器现状一致）；区域子集（NotoSansSC 8.3MB）经实测缺失韩文音节，否决不采纳。
- 补字记录（2026-10-01，D-127）：内嵌三字体联合缺阿拉伯/泰/天城文/希伯来四脚本（egui 无系统回退 → 语言下拉豆腐块）；红线 = 覆盖测试机械枚举 `lt_i18n::LANGUAGES` 全表（`embedded_chain_covers_language_native_names`）。

## .cache/sherpa-onnx/（构建期预取，gitignored 不入库）

- 用途：sherpa-onnx-sys 构建期静态库（ASR worker 链接用；预取+解包脚本 = `scripts/fetch_sherpa_libs.ps1`，
  版本号取自 Cargo.lock 的 sherpa-onnx-sys 条目）
- 版本：sherpa-onnx v1.13.7（win-x64-static-MT-Release-lib）
- 来源：上游官方仓 `k2-fsa/sherpa-onnx` Release `v1.13.7` 资产
  `sherpa-onnx-v1.13.7-win-x64-static-MT-Release-lib.tar.bz2`
  （`https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.7/<档名>`；镜像前缀走 `-Mirror`）
- sha256：`04734146fb3a21a297604c586ea826346dbb167c19b9ccc79c1f85d39f490395`（2026-09-28 本机
  .cache 实测，来源 = fetch 脚本自上游 Release 下载；同值已登记 fetch 脚本期望表，下载后强校验）
- 复现 / 重取：跑 `scripts/fetch_sherpa_libs.ps1`（幂等：extracted/lib + 来源标记在即秒退；
  强制重下 = 删 .cache/sherpa-onnx/ 下归档）
