# LiveTranslate-rs

[中文](README.md) · Real-time audio translation on Windows: **capture system audio → recognize speech locally → translate through your own AI endpoint → show the result live in an overlay window or an OBS subtitle window.**

> 📥 **Download**: grab `LiveTranslate-<version>.zip` from [Releases](releases/latest), unzip and run · **中文**: [README.md](README.md)

- Windows 10/11 x64 (other systems not supported yet)
- **CPU only**: no GPU needed; speech recognition runs locally, **audio never leaves your machine**
- **Single exe**: double-click and go — onnxruntime, the voice-activity model and fonts are all bundled
- Three recognition engines: FunASR (SenseVoice Small / Fun-ASR-Nano), Whisper (tiny through turbo, six sizes), Qwen3-ASR
- Swap translation endpoints freely: local LM Studio / Ollama, or any cloud **OpenAI-compatible API** such as DeepSeek or SiliconFlow
- UI in Chinese and English

> This repository is the Rust rewrite of the original Python LiveTranslate. The `LiveTranslate/` folder in the workspace is a copy of the original code, kept for reference only; features no longer map one to one.

---

## 1. For users

### Install one thing first

**Microsoft Visual C++ 2015–2022 Redistributable x64** ([download](https://aka.ms/vs/17/release/vc_redist.x64.exe), double-click to install). The exe will not start without it on Windows.

### Three steps to get running

**1. Double-click `livetranslate.exe`**

There is no wizard: you land straight in the main UI and it starts working automatically. The overlay shows `unavailable` until a recognition model is downloaded — that is normal.

(Nothing installed yet? Grab the zip from [Releases](releases/latest), unzip, then double-click `livetranslate.exe`.)

**2. Download a recognition model** — "Settings → VAD / ASR" page

- Under "ASR engine", pick an engine and a model size — beginners should pick **SenseVoice** (good Chinese, small download);
- "Audio" captures system sound by default; tick "Microphone" if you also want your own voice;
- "Download source": pick **ModelScope** (fast from mainland China) — SenseVoice / Fun-ASR-Nano / Qwen3-ASR download from ModelScope directly, while Whisper automatically goes through the hf-mirror.com mirror; pick **HuggingFace** to connect everything to the official source;
- Click "**Download**"; the engine is ready as soon as the progress bar finishes.

Model size reference: SenseVoice ≈ 230 MB; Whisper from tiny ≈ 30 MB up to large-v3 ≈ 1.1 GB; Nano / Qwen3 ≈ 1 GB each. Downloads can be resumed and cancelled.

**3. Configure translation** — "Settings → Translation" page

Fill in three fields — **API address** (default `http://127.0.0.1:1234/v1`, meant for a local LM Studio), **API key**, **model name** — pick the target language, then click "**Test connection**". Once it passes, play anything with sound and the words appear.

### Where the UI lives

- **The overlay window is the main window**: the top row is Hide / Subtitles / Start-stop / Clear / Compact / Settings / Exit; the bottom row has Click-through, Always-on-top, Auto-scroll, Taskbar, plus three dropdowns for model, source language and target language. Right-click the text to copy or export.
- **Control panel**: click "Settings" on the overlay; 8 pages — VAD / ASR, Translation, Style, Subtitles, Benchmark, Cache, Changelog, Log.
- **Subtitle window** (for OBS, off by default; enable it on the "Subtitles" page): hover the top edge for buttons; drag with the top bar or the middle mouse button; with click-through on, the mouse reaches the windows behind it.
- **Tray icon**: pause/resume, show hidden windows, open settings, exit. Note: **hiding to the tray only tucks the windows away — the program keeps running** (a system notification reminds you).

### Where the data lives

Everything under `~/.config/livetranslate` (note: a literal `.config` **in your home directory**, not `%APPDATA%`):

| Content | Location | Notes |
|---|---|---|
| Settings | `settings.json` | `models_dir` inside can move the models to another drive |
| Models | `models/` | split by download source: `modelscope/`, `huggingface/` |
| Transcripts | `transcripts/` | auto-save can be turned off on the "Cache" page |
| Logs | `logs/` | also viewable live on the panel "Log" page |

The program is **single-instance**: launching it again just exits. To use a different config directory (testing or portable use), point the `LIVETRANSLATE_CONFIG_DIR` environment variable at it.

---

## 2. For developers

> Before touching code, read [AGENTS.md](AGENTS.md) first — hard constraints, layering rules and the routing table live there (full gotcha list = docs/gotchas.md; process source of truth = docs/agents/workflow.md; work state = GitHub Issues via `gh issue list`).

### 1. Install three tools

Install once, valid forever — other than these **you install nothing**:

| Tool | What it does | How |
|---|---|---|
| Rust (MSVC toolchain) | compiles the Rust code | install from [rustup.rs](https://rustup.rs), then run `rustup default stable-x86_64-pc-windows-msvc` |
| VS Build Tools | provides the C/C++ compiler and linker (whisper.cpp builds with it) | tick "**Desktop development with C++**" during setup |
| uv | puts libclang and cmake inside the project — **so you do NOT install LLVM / CMake separately** | `winget install astral-sh.uv` |

### 2. Three commands to a build

```bash
git clone <repo url> && cd livetranslate-rs
uv sync                                                                 # ~10 s (first install ~1 min with the torch dependency): puts libclang + cmake + the two bundled-asset packages into the in-repo .venv
pwsh -ExecutionPolicy Bypass -File scripts/fetch_sherpa_libs.ps1  # ~120 MB of prebuilt libraries, once only
```

After all three you can build — **the first build takes ten-plus minutes** (whisper.cpp compiles on the spot); afterwards it is incremental:

```bash
cargo run -p lt-app     # take it for a spin
```

If that 120 MB download stalls (common from mainland China when hitting GitHub directly), rerun with a mirror flag:

```powershell
pwsh -ExecutionPolicy Bypass -File scripts/fetch_sherpa_libs.ps1 -Mirror https://gh-proxy.com/
```

> Switching machines, or after accidentally deleting `.venv` / `.cache`: just rerun the two commands above. Every tool the project needs lives in the in-repo `.venv`; caches live in `.cache` and `target`; **your system keeps only those three tools**.

### 3. Everyday commands

```bash
cargo test --workspace             # full test suite (must run before wrapping up). ~600+ passing + 9 real-model/real-network probes skipped by default (rolling value, not a gate)
cargo test -p lt-ui                # test a single crate (swap in lt-asr / lt-proto etc.)
cargo clippy --workspace --all-targets   # run lint manually (the commit hook runs it too)
cargo run -p lt-app                # run the GUI locally
cargo build --release -p lt-app    # single exe: target/release/livetranslate.exe (~72 MB, rolling value)
```

To run the GUI without polluting your real config, use a throwaway config directory (note `models_dir` must point at your real model cache, otherwise every probe fails):

```powershell
$env:LIVETRANSLATE_CONFIG_DIR = "$env:TEMP\lt-smoke"
New-Item -ItemType Directory -Force $env:LIVETRANSLATE_CONFIG_DIR | Out-Null
'{ "models_dir": "C:/Users/<your-name>/.config/livetranslate/models" }' |
  Set-Content -Encoding Ascii "$env:LIVETRANSLATE_CONFIG_DIR\settings.json"   # must be Ascii; UTF8 writes a BOM that breaks config parsing
$env:LIVETRANSLATE_SHOW_PANEL = "1"   # walkthrough flag: open the control panel on start (drop it if unneeded)
cargo run -p lt-app
```

Minimal proof the artifact starts: `target/release/livetranslate.exe --version` (no GUI; prints the version and exits).

### 4. Committing code

```bash
git config core.hooksPath .githooks    # once per machine: enable the pre-commit checks
git add <specific files>                      # do not use git add -A
git commit -m "fix(overlay): short message"   # commit messages in Chinese
```

With the hook installed, `git commit` **automatically runs the full scripts/precommit.ps1 gate** (fast guards + clippy); the commit is only created when everything passes; asset-only commits skip it automatically. You can bypass with `--no-verify` in an emergency, but CI will catch it.

After pushing, GitHub Actions runs the same suite (single job: the same precommit.ps1 + full tests + a distribution drill) on every push to any branch and every PR; every green build leaves a downloadable zip artifact on the Actions page. **Green locally but red on CI has essentially one cause: you skipped the gate** — both sides run the same thing, see above.

Two more rules: planning/research docs are committed separately before the implementation; generated artifacts (`docs/architecture/`, `docs/ui-audit/`) never enter the repo.

### 5. How the code is split (10 crates, one-way dependencies)

```
lt-proto → lt-i18n → lt-models → lt-download → lt-audio → lt-asr → lt-translate → lt-orchestrator → lt-ui → lt-app
```

| crate | role |
|---|---|
| `lt-proto` | event/command/data contracts (**frozen**: adding fields needs review; UI features must not extend it) |
| `lt-i18n` | UI strings; the zh/en yaml pair under `assets/i18n/` **must change together** |
| `lt-models` | settings I/O, model registry, cache probing (no network) |
| `lt-download` | downloader |
| `lt-audio` | system-audio capture, VAD |
| `lt-asr` | speech recognition subprocess + interprocess protocol |
| `lt-translate` | calls the LLM for translation |
| `lt-orchestrator` | main pipeline, thread supervision, download management, log bridge |
| `lt-ui` | egui multi-window UI (depends only on proto/i18n/models) |
| `lt-app` | composition root: startup, command routing, worker dispatch |

The dependency rules are **machine-enforced** (`crates/lt-app/tests/topology.rs` + `cargo test`); read `docs/archive/architecture-v2.md` §3.1 before touching dependencies.

### 6. Stuck? Look here

| Symptom | Fix |
|---|---|
| Build fails with `Unable to find libclang` or missing `cmake` | you skipped `uv sync`, or `.venv` was deleted → run `uv sync` |
| Build fails with `SHERPA_ONNX_ARCHIVE_DIR does not contain expected archive` | the prefetch script has not run (it does not fall back to the network) → run `scripts\fetch_sherpa_libs.ps1` |
| Build fails with `bundled asset missing: …\.venv\…` | you skipped `uv sync` (the pyproject dev group pins onnxruntime + silero-vad, embedded into the exe at build time) |
| sherpa download slow or failing | rerun with `-Mirror https://gh-proxy.com/`, or set `HTTPS_PROXY` |
| Linker reports `LNK2005` / `LNK1169` (CRT clash) | the two `CMAKE_*` lines in `.cargo/config.toml` were changed → change them back (**do not touch**) |
| Tests report StorageFull / hang | the C: drive is full → point `TMPDIR` at another drive |
| Model downloads crawl | switch the download source to ModelScope on the "Settings → VAD / ASR" page |
| An "already running" prompt | single-instance by design → quit from the tray, or kill the old process in Task Manager |
| Switching the UI language does nothing | the language switch sits at the bottom of the "VAD / ASR" page, **takes effect after a restart** |

### 7. Where the docs are

| Doc | Content |
|---|---|
| [AGENTS.md](AGENTS.md) | project charter, hard constraints, layering rules, routing table and the 5 tripwires (**first stop for development**) |
| [docs/agents/workflow.md](docs/agents/workflow.md) | development process: the Matt Skills ↔ this-repo mapping (process source of truth, D-102) |
| GitHub Issues (`gh issue list`) | work board: ready-for-human awaiting decisions / ready-for-agent fully specified (≠ a work order) |
| [docs/README.md](docs/README.md) | doc index (live docs + archived decision history) |
| [docs/distribution.md](docs/distribution.md) | distribution route: packaging spec, release manual, backlog |
| [docs/archive/architecture-v2.md](docs/archive/architecture-v2.md) | architecture 2.0: the ten-crate topology and dependency whitelist |
| [docs/archive/](docs/archive/) | past decisions and finished work packages (**read-only**) |

---

## 3. License

Code under MIT. Sources and checksums of the bundled third-party components and assets (onnxruntime, silero-vad, whisper.cpp, sherpa-onnx, the three font families, etc.) are in [assets/SOURCES.md](assets/SOURCES.md).
