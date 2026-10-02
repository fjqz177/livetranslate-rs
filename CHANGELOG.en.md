# Changelog

## [1.1.0] - 2026-10-02

Realtime recognition redo and a round of UI experience improvements.

- **Realtime recognition redo**: While you speak, the overlay shows a greyed "streaming line" previewing the intermediate recognition; as soon as a sentence is finalized (a pause or other sentence-boundary signal), it is committed on screen and sent for translation — no more waiting for the whole utterance to finish. Enabled by default after upgrading, no settings needed. The subtitle window still only shows finalized text and does not flicker with intermediate states.
- **Instant UI language switching**: Switching the interface language (Chinese/English) in the control panel now updates all window titles, UI strings and the tray menu immediately — no restart required. The tray status line is localized as well.
- **Non-Latin scripts render correctly**: Four Noto script fonts (Arabic, Thai, Devanagari, Hebrew) are now embedded, so Arabic, Thai, Hindi, Hebrew and other such content no longer shows "tofu" boxes.
- **ModelScope direct sources for funasr-nano / qwen3**: Both ASR model families gain a self-hosted ModelScope mirror (same model files as HuggingFace) for direct downloads from mainland networks. The download source remains switchable in settings, with automatic fallback when the chosen source lacks a file.
- **First-launch model guidance**: When the app starts without an ASR model downloaded, a highlighted banner at the top of the control panel guides you straight to the download page; it disappears once the model is ready.
- **Honest translator status**: If a newly selected translation provider fails to initialize, the overlay now explicitly reports "still using the old provider, and why" instead of silently falling back; the actually-active configuration is shown whenever a request steps down to a fallback.
- **Deselect by clicking again**: In lists such as ASR model, audio device and translation preset, clicking the currently selected item again cancels the selection.
- **Taskbar icon fix**: Fixed the taskbar icon occasionally showing as a blank white sheet (the Start-menu shortcut's icon pointed to a path that no longer exists).
- **Narrow-window adaptation**: The control panel's minimum width now adapts to the interface language and its content; the overlay, secondary dialogs and download cards no longer silently clip text or push buttons out of view in narrow windows.
- **UI detail fixes**: The last item of dropdown popups is no longer clipped; list rows no longer jump on hover; and after clearing the record, a late-arriving translation no longer pulls old sentences back onto the screen.
- **Recognition chain stability**: Fixed several shutdown stalls and false reports on the exit path; stopping the recognition benchmark no longer hangs; several English UI strings corrected.

## [1.0.1] - 2026-09-25

Window experience fixes.

- **A complete interface right from launch**: every window is now painted before it is shown — no more white flash at startup and no separate "loading model" popup; what appears is simply the overlay (and the subtitle window, if enabled).
- **No loading popups while running**: when switching ASR models or audio devices, auto-reloading a corrupted model file, or recovering from a disconnect, the loading status now shows on the overlay's status line instead of a separate window that steals focus. Windows you open yourself — control panel, benchmark, confirm dialogs, export dialogs — are unaffected.
- **Clean exit**: once you confirm, all windows vanish within a frame or two while the app shuts down quietly in the background — no stray white window between the confirmation and process exit.

## [1.0.0] - 2026-09-19

First official release.

- **Real-time audio translation**: Captures what your PC is playing (microphone optional), runs local speech recognition + AI translation, and shows source text and translation side by side on the overlay, an OBS-capturable subtitle window, and the control panel. Great for watching videos, meetings, and live-stream subtitles.
- **Three local ASR engines**: SenseVoice (default), Whisper (tiny / base / small / medium / large-v3 / turbo), and Qwen3 — all pure CPU, offline once the model is downloaded.
- **Hassle-free model management**: Resumable downloads with per-file sha256 verification and switchable HuggingFace / ModelScope sources; every load is re-verified, and corrupted files are quarantined and re-downloaded automatically — no manual cleanup.
- **Translation providers out of the box**: Built-in presets for DeepSeek, OpenAI, GLM (Zhipu), Kimi, Qwen (Alibaba), and Doubao (Volcano Ark), plus local LM Studio and Ollama — pick a preset, paste your key, and go. Hot-swap providers from the overlay dropdown at any time; in-flight translations finish gracefully.
- **Non-blocking connection test**: Runs on its own thread with a four-state result and concrete reasons — no more permanent waits.
- **Reliable translations**: Chain-of-thought never leaks into the translation; failed requests retry with automatic step-down fallback; errors are clearly distinguishable from translations; adjustable context memory; cancellable at any time.
- **Transparent cost tracking**: Accumulated per session and reset on restart, with per-provider currency bookkeeping.
- **Multi-window + tray**: Draggable always-on-top overlay, OBS-capturable subtitle window, bilingual control panel, and a log window with unread badge and bottom-following; the tray icon handles show/hide, pause/resume, and exit.
- **Clean exit**: A unified confirm dialog (Enter / ESC); the sentence being recognized when you quit still gets recognized and translated — nothing is cut off.
- **Centralized data**: Settings, models, transcripts, and logs all live under one directory (`~/.config/livetranslate`); upgrading means replacing the exe, and transcripts are saved automatically.
- **Single instance**: Launching again simply brings up the existing window instead of starting a second copy.
