LiveTranslate-rs — 实时音频翻译（Windows）

Rust 原生实时音频翻译：实时捕获系统声音（可选叠加麦克风）→ 本地纯 CPU
语音识别（FunASR SenseVoice / Whisper / Qwen3-ASR）→ 经任意 OpenAI 兼容
接口翻译 → 悬浮窗 / OBS 字幕窗 / 控制面板实时展示原文与译文。

版本：见同目录 exe 属性（或命令行 livetranslate.exe --version）。

运行前提
  1. Windows 10/11 x64；
  2. VC++ 2015–2022 x64 运行库（官方下载：https://aka.ms/vs/17/release/vc_redist.x64.exe）；
  3. 能出声的扬声器/耳机（抓系统音频）；可选麦克风；
  4. 下载识别模型需要网络；翻译需要一个能连通的 OpenAI 兼容接口
     （本地 LM Studio / Ollama，或云端 DeepSeek、硅基流动等）。

三步上手
  1. 双击 livetranslate.exe 启动（无向导，直接进主界面并自动识别）；
     模型未就绪时悬浮窗显示 unavailable，到「设置 → VAD / ASR」下载。
  2. 「设置 → VAD / ASR」：选 ASR 引擎与模型档位；音频默认抓系统声音，
     勾选「麦克风」叠加输入。
  3. 「设置 → 翻译」：选/填模型（api_base、api_key、model）并「测试连接」。
     「设置 → 字幕」可开关翻译（悬浮窗/字幕窗由悬浮窗按钮控制）。

数据位置
  配置文件：~/.config/livetranslate/settings.json
  （Windows 下为 C:\Users\<你>\.config\livetranslate\
   —— 是字面 home/.config，不是 %APPDATA%）。
  模型缓存：默认同目录 models\；可在 settings.json 的 models_dir 键改为
  任意路径（改后重启生效）。
  日志与转写记录：~/.config/livetranslate/logs\、transcripts\。

常见问题
  · 悬浮窗 unavailable → 模型未下载/路径不可用，「设置 → VAD / ASR」下载。
  · 翻译空白/报错 → 模型页「测试连接」；超时可在字幕页调大。
  · 隐藏后去哪找 → 托盘图标（左键显示悬浮窗，右键菜单）。

卸载
  1. 删除 livetranslate.exe 所在目录；
  2. 删除数据目录 C:\Users\<你>\.config\livetranslate\
     （若改过 models_dir，模型缓存在自定路径，一并删除）；
  3. 删除开始菜单快捷方式：
     %APPDATA%\Microsoft\Windows\Start Menu\Programs\LiveTranslate.lnk。
  彻底重置（保留程序）：仅删数据目录下的 settings.json 即可。

完整说明见 GitHub 仓库 README.md；许可信息见 LICENSE 与 NOTICES.md。
