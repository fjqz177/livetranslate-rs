LiveTranslate — 实时音频翻译
════════════════════════════════════

【这是什么】
  电脑播放什么声音，就实时出什么文字和翻译：
  抓系统声音（可选叠加麦克风）→ 本地识别（纯 CPU，
  SenseVoice / Whisper / Qwen3 三引擎可选）→ 经你的
  OpenAI 兼容接口翻译 → 悬浮窗 / OBS 字幕窗 / 控制面板
  同步显示原文与译文。看片、开会、直播字幕都行。

【开始前】
  1. Windows 10 / 11（64 位）
  2. VC++ 运行库（多数电脑已有；启动报缺 DLL 时装它）：
     https://aka.ms/vs/17/release/vc_redist.x64.exe
  3. 能出声的扬声器 / 耳机（抓的是系统声音）；可选麦克风
  4. 首次使用需联网下载识别模型（之后离线可用）
  5. 一个能连通的 OpenAI 兼容接口：
     云端：DeepSeek、硅基流动等（需 api_key）
     本地：LM Studio、Ollama 等（免 key）

【三步开跑】
  ① 双击 livetranslate.exe —— 无向导，直接进主界面并自动识别
  ② 设置 →「VAD / ASR」页：选引擎与模型档位 → 点「下载」
     （音频默认抓系统声音；想录自己说话，同页勾选「麦克风」。
       未就绪时悬浮窗显示 unavailable，下载完即恢复）
  ③ 设置 →「翻译」页：填 api_base、api_key、model → 点「测试」
     —— 显示「连接成功」就全通了。
  随便播放点有声内容，字幕即刻出现。

【常用操作】
  · 窗口藏了找不到 → 点托盘图标（左键显示悬浮窗，
    右键托盘有全部入口）
  · 悬浮窗 / 字幕窗的显隐 → 悬浮窗上的按钮直接控制
  · 开关翻译 → 设置「字幕」页
  · OBS 直播字幕 → 设置「字幕」页勾选「字幕窗口」，
    再在 OBS 里把该窗口加为捕获源

【出问题了】
  · 悬浮窗显示 unavailable → 模型没下好：「VAD / ASR」页重新下载
  · 翻译空白 / 测试失败 → 「翻译」页点「测试」看具体报错；
    老是超时就调大同页的「模型连接超时」
  · 识别没反应 → 确认电脑正在出声；麦克风不收音，
    检查「VAD / ASR」页「麦克风」勾选
  · 排查更细的问题 → 设置「日志」页

【文件在哪】
  全部数据集中在一个目录 —— C:\Users\<你>\.config\livetranslate\
  （注意：是字面的 home\.config，不是 %APPDATA%）：
      settings.json  配置
      models\        识别模型（可在 settings.json 的 models_dir
                     键改到任意路径，改后重启生效）
      logs\          日志
      transcripts\   转写记录

【怎么卸载】
  1. 删 livetranslate.exe 所在文件夹
  2. 删上面的数据目录（改过模型目录的，自定路径一并删）
  3. 删开始菜单快捷方式（如有）：
     %APPDATA%\Microsoft\Windows\Start Menu\Programs\LiveTranslate.lnk
  彻底重置（保留程序）：只删数据目录里的 settings.json 即可。

────────────────────────────────────
查版本：右键 livetranslate.exe → 属性，或命令行 --version
完整文档：GitHub 仓库 README.md
开源许可：LICENSE、NOTICES.md、OFL.txt
