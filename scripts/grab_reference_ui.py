#!/usr/bin/env python3
"""原版参照截图重拍脚本（截图对象 = 工作区 LiveTranslate/ 参考副本）。

程序化截图（QWidget.grab()），全程无需人工交互，也遵守"不用 computer use"
的实机验证纪律：

    # 需要带 PyQt6 + PyYAML + psutil + openai 的 Python 环境（任意 venv 均可；
    # 本机可复用外部原版仓的 venv，仅当解释器用，跑的代码严格限于工作区副本，
    # 具体解释器路径因机器而异、不写入本库）：
    #   python scripts/grab_reference_ui.py

输出 assets/reference/（zh/en 各一套）：
  panel_{lang}.png              控制面板·识别 tab（默认落地页）
  panel_{tab}_{lang}.png        控制面板其余 6 个 tab（translation/style/
                                subtitle/benchmark/cache/changelog）
  overlay_{lang}.png            悬浮字幕窗（注入 3 组示例对话 + 监控条数据）
  subtitle_{lang}.png           OBS 字幕窗（注入示例双行字幕）
  log_{lang}.png                日志窗（注入 6 条示例日志，覆盖五级着色）

示例内容均为脚本注入的演示数据（非真实运行产物）；面板传入脱敏的
SAVED_SETTINGS（api_key 为占位符），不读取副本的 user_settings.json，
渲染过程不写回任何文件。
"""

import logging
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LT_DIR = ROOT / "LiveTranslate"
OUT_DIR = ROOT / "assets" / "reference"

if not LT_DIR.is_dir():
    sys.exit(f"参考副本不存在: {LT_DIR}")
sys.path.insert(0, str(LT_DIR))

os.environ["QT_LOGGING_RULES"] = "qt.text.font.db=false;qt.qpa.fonts.warning=false"

import yaml
from PyQt6.QtCore import QEventLoop, QTimer
from PyQt6.QtGui import QFont, QFontDatabase
from PyQt6.QtWidgets import QApplication, QTabWidget

# UI 模块在模块级 from i18n import t —— 必须先插 sys.path 再 import
from i18n import set_lang
from control_panel import ControlPanel
from log_window import LogWindow
from subtitle_overlay import SubtitleOverlay
from subtitle_window import SubtitleWindow

# 面板 tab 顺序 = control_panel.py 构造顺序；索引 0（识别）沿用 panel_{lang}.png
PANEL_TAB_SLUGS = ["", "translation", "style", "subtitle", "benchmark", "cache", "changelog"]
CACHE_TAB_INDEX = 5

# 脱敏的代表性设置：镜像 control_panel.py 的出厂缺省 dict（值取自 config.yaml），
# 模型名 = 出厂默认，api_key 占位。少给键会落到控件级缺省（如 VAD 模式显示
# "能量检测"而非出厂的 silero），拍出来就不是出厂状态。
MODEL_NAME = "hunyuan-mt-chimera-7b"


def build_saved_settings(config: dict) -> dict:
    tc = config["translation"]
    return {
        "vad_mode": "silero",
        "vad_threshold": config["asr"]["vad_threshold"],
        "energy_threshold": 0.02,
        "min_speech_duration": config["asr"]["min_speech_duration"],
        "max_speech_duration": config["asr"]["max_speech_duration"],
        "silence_mode": "auto",
        "silence_duration": 0.8,
        "asr_language": config["asr"].get("language", "auto"),
        "asr_engine": "funasr",
        "funasr_model": config["asr"].get("funasr_model", "sensevoice-small"),
        "asr_device": "cuda",
        "sensevoice_pad_seconds": config["asr"].get("sensevoice_pad_seconds", 0.5),
        "whisper_pad_seconds": config["asr"].get("whisper_pad_seconds", 0.5),
        "models": [
            {
                "name": tc["model"],
                "api_base": tc["api_base"],
                "api_key": "sk-demo-not-a-real-key",
                "model": tc["model"],
            }
        ],
        "active_model": 0,
        "hub": "ms",
    }

# 悬浮窗示例对话：(时间戳, 原文, 译文, ASR 耗时 ms, 翻译耗时 ms)
DEMO_DIALOG = [
    ("20:14:55", "Welcome back everyone, let's get started.", "欢迎各位回来，我们马上开始。", 96, 187),
    ("20:15:09", "Today we're taking a look at real-time translation.", "今天我们来看一看实时翻译。", 112, 203),
    ("20:15:23", "First, a quick look at the whole pipeline.", "首先快速过一遍整个流水线。", 105, 176),
]

# 日志窗示例行：(级别, 消息)；含 "ASR [" / "Translate:" 以覆盖专属着色
DEMO_LOG = [
    (logging.DEBUG, "Audio backend initialized: WASAPI (default device, 48.0 kHz)"),
    (logging.INFO, "ASR [sensevoice-onnx] model ready on cpu (warmup 412 ms)"),
    (logging.INFO, f"Translate: {MODEL_NAME} connected, 1 model active"),
    (logging.WARNING, "VAD flagged 2.4 s of continuous speech; buffer flushed"),
    (logging.ERROR, "Translation request failed (timeout after 10 s); retrying"),
    (logging.CRITICAL, "Uncaught exception in worker thread: RuntimeError('demo')"),
]


def pump(ms: int):
    """在事件循环内等待 ms 毫秒（让 QTimer/信号/布局/动画走完）。"""
    loop = QEventLoop()
    QTimer.singleShot(ms, loop.quit)
    loop.exec()


def grab(widget, out_name: str):
    path = OUT_DIR / out_name
    pix = widget.grab()
    if not pix.save(str(path)):
        sys.exit(f"保存失败: {path}")
    print(f"  {out_name}  ({pix.width()}x{pix.height()})")


def capture_panel(config, saved, lang):
    panel = ControlPanel(config, saved_settings=dict(saved))
    panel.show()
    pump(200)  # _fit_height singleShot + 首 tab 布局

    tabs = panel.findChild(QTabWidget)
    if tabs is None:
        sys.exit("未找到 QTabWidget（control_panel.py 结构变更？）")

    for index, slug in enumerate(PANEL_TAB_SLUGS):
        tabs.setCurrentIndex(index)
        if index == CACHE_TAB_INDEX:
            # 缓存页后台线程扫 models 目录体积，等扫描落地（超时则按现状拍）
            for _ in range(80):
                if panel._cache_list.count() > 0:
                    break
                pump(100)
            pump(150)
        else:
            pump(150)
        name = f"panel_{slug}_{lang}.png" if slug else f"panel_{lang}.png"
        grab(panel, name)

    panel.close()
    panel.deleteLater()
    pump(100)


def capture_overlay(config, lang):
    overlay = SubtitleOverlay(config["subtitle"])
    overlay.set_models([{"name": MODEL_NAME}], 0)
    overlay.set_target_language("zh")
    overlay.set_source_language("auto")
    overlay.set_running(True)
    overlay.update_asr_device("cuda")
    overlay.update_monitor(0.062, 0.41)
    overlay.update_stats(len(DEMO_DIALOG), len(DEMO_DIALOG), 214, 623, 0.0)
    for msg_id, (ts, orig, trans, asr_ms, tl_ms) in enumerate(DEMO_DIALOG, 1):
        overlay.add_message(msg_id, ts, orig, "en", asr_ms)
        overlay.update_translation(msg_id, trans, tl_ms)
    overlay.show()
    pump(400)  # scroll-to-bottom singleShot(50) + 首帧布局
    grab(overlay, f"overlay_{lang}.png")
    overlay.close()
    overlay.deleteLater()
    pump(100)


def capture_subtitle(config, lang):
    subwin = SubtitleWindow(None)
    subwin.update_text(
        "Technology should make communication borderless for everyone.",
        {"zh": "科技应当让沟通没有边界，惠及每一个人。"},
    )
    subwin.show()
    pump(400)  # 高度自适应 + 首帧绘制
    grab(subwin, f"subtitle_{lang}.png")
    subwin.close()
    subwin.deleteLater()
    pump(100)


def capture_log(config, lang):
    logwin = LogWindow()
    logwin._show_debug.setChecked(True)  # 让 DEBUG 行可见（覆盖五级着色）
    # Qt 6.11 grab() 怪癖：QTextEdit 自身样式表的背景不进 grab（真实显示正常），
    # 给 viewport 补同色才能拍出与实机一致的 #1e1e1e 深色文本区
    logwin._text.viewport().setStyleSheet("background-color: #1e1e1e;")
    demo = logging.getLogger("LiveTranslate.Demo")
    demo.setLevel(logging.DEBUG)
    handler = logwin.get_handler()
    demo.addHandler(handler)
    logwin.show()
    for level, msg in DEMO_LOG:
        demo.log(level, msg)
    pump(200)
    grab(logwin, f"log_{lang}.png")
    demo.removeHandler(handler)
    logwin.close()
    logwin.deleteLater()
    pump(100)


def main():
    config = yaml.safe_load((LT_DIR / "config.yaml").read_text("utf-8"))
    saved = build_saved_settings(config)

    app = QApplication(sys.argv)
    app.setQuitOnLastWindowClosed(False)
    # 与 main.py 相同的字体钉死，规避 Windows 位图字体回退
    if "Segoe UI" in QFontDatabase.families():
        app.setFont(QFont("Segoe UI", 9))

    OUT_DIR.mkdir(parents=True, exist_ok=True)

    for lang in ("zh", "en"):
        print(f"[{lang}]")
        set_lang(lang)  # 必须在创建任何窗口之前
        capture_panel(config, saved, lang)
        capture_overlay(config, lang)
        capture_subtitle(config, lang)
        capture_log(config, lang)

    print(f"完成 → {OUT_DIR}")


if __name__ == "__main__":
    main()
