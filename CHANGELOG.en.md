# Changelog

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
