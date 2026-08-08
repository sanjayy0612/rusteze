# rusteze

A local-first macOS and Windows command-line tool for recording meeting audio, transcribing it on your laptop, and optionally creating a summary.

## Why it exists

Meeting tools often require a bot, a paid plan, or uploading audio to someone else's servers. `rusteze` aims to keep control with you:

- capture meeting audio directly on your Mac or Windows PC;
- keep recordings and transcription local;
- only send transcript text to an LLM if you explicitly ask for a summary.

Always get the consent of everyone being recorded and follow the laws and policies that apply to your meeting.

## Learning approach

This is a learning-by-building project. We will build one small, understandable piece at a time instead of starting with a large, complex application.

## Current phase: selectable audio capture, ready for macOS/Windows hardware validation

`rusteze start [title]` creates a self-contained session folder under
`~/Documents/rusteze/meetings`, asks the included native macOS helper to check
the permissions required by the selected capture mode, records the lifecycle in
`session.json`, and stops cleanly with `Ctrl+C`.

```bash
cargo run -- start "Rust workshop"            # system audio only (default)
cargo run -- start "Rust workshop" --mic      # system audio + microphone
cargo run -- start "Rust workshop" --mic-only # microphone only
```

System-only capture needs Screen Recording/System Audio access. Microphone-only
capture needs Microphone access. `--mic` requires both. Use
`rusteze request-permissions`, with the same optional mode flag, to request only
the permissions needed for that workflow.

The session progresses from `recording` to `stopping` to `completed`. If the
program cannot continue normally, `session.json` records a `failed` state and
a recoverable reason. To grant missing access, open **System Settings → Privacy
& Security**, then enable Rusteze's terminal/binary under **Microphone** and
**Screen Recording**.

Before capture begins, Rusteze requires 256 MiB of free disk space. On the next
`start`, it also marks any session left in `recording` or `stopping` after a
crash/interruption as `failed` while preserving the audio that was already
written.

On macOS, the helper source lives at `macos-helper/Sources/main.swift`. Build it
once before running `start`:

```bash
./macos-helper/build.sh
```

Its `check-permissions [mode]`, `request-permissions [mode]`, and
`record SESSION_FOLDER mode` protocols are the boundary Rust uses for capture.
System-only sessions contain `system.caf`, microphone-only sessions contain
`mic.caf`, and `--mic` sessions contain both separate files. Set
`RUSTEZE_CAPTURE_HELPER` to use a helper binary at another path during
development.

Before recording, allow the helper under the relevant sections of **System
Settings → Privacy & Security**. You can request the initial macOS prompts with:

```bash
./macos-helper/.build/debug/rusteze-capture-helper request-permissions system
```

`rusteze transcribe <session-path>` is present as the model-independent
transcription boundary. It intentionally reports that no engine is configured
until a local engine (such as whisper.cpp) is selected and implemented.

On Windows, Rusteze uses native WASAPI directly from Rust. System capture uses
the default render endpoint in loopback mode and microphone capture uses the
default capture endpoint. Windows recordings are written as `system.wav` and
`mic.wav`; macOS recordings remain CAF files.

Windows development builds require a Windows Rust toolchain:

```powershell
cargo build
cargo build --release
```

macOS regression build:

```bash
cargo build
./macos-helper/build.sh
```

## Local model setup

Rusteze uses the native `whisper.cpp` runtime rather than Python or a virtual
environment. The setup script downloads and builds the runtime, then downloads
the local `large-v3-turbo-q5_0` model (about 547 MiB):

```bash
./scripts/setup-whisper-cpp.sh
```

The runtime is stored in `tools/whisper.cpp/`; models are stored in `models/`.
Both are ignored by Git. See `.env.whisper.example` only when you need a custom
location.

Not in the first version:

- background daemon support;
- transcription or LLM summaries;
- per-app audio capture;
- Linux support.

## Planned command shape

```text
rusteze start [title] [--mic|--mic-only]
rusteze request-permissions [--mic|--mic-only]
rusteze create-meeting [title]
rusteze transcribe <session-path>
```

We will introduce these commands one at a time as the project grows.
