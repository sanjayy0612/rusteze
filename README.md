# rusteze

A local-first macOS command-line tool for recording meeting audio, transcribing it on your laptop, and optionally creating a summary.

## Why it exists

Meeting tools often require a bot, a paid plan, or uploading audio to someone else's servers. `rusteze` aims to keep control with you:

- capture meeting audio directly on your Mac;
- keep recordings and transcription local;
- only send transcript text to an LLM if you explicitly ask for a summary.

Always get the consent of everyone being recorded and follow the laws and policies that apply to your meeting.

## Learning approach

This is a learning-by-building project. We will build one small, understandable piece at a time instead of starting with a large, complex application.

## Current phase: two-track recording, ready for hardware validation

`rusteze start [title]` creates a self-contained session folder under
`~/Documents/rusteze/meetings`, asks the included native macOS helper to check
Microphone and Screen Recording access, records the lifecycle in `session.json`,
starts separate microphone and system-audio tracks, and stops cleanly with
`Ctrl+C`.

```bash
cargo run -- start "Rust workshop"
```

The session progresses from `recording` to `stopping` to `completed`. If the
program cannot continue normally, `session.json` records a `failed` state and
a recoverable reason. To grant missing access, open **System Settings → Privacy
& Security**, then enable Rusteze's terminal/binary under **Microphone** and
**Screen Recording**.

Before capture begins, Rusteze requires 256 MiB of free disk space. On the next
`start`, it also marks any session left in `recording` or `stopping` after a
crash/interruption as `failed` while preserving the audio that was already
written.

The helper source lives at `macos-helper/Sources/main.swift`. Build it once
before running `start`:

```bash
./macos-helper/build.sh
```

Its current `check-permissions` protocol is the boundary Rust uses for capture.
It records separate `mic.caf` and `system.caf` tracks and finalizes both when
`rusteze` receives `Ctrl+C`. Set `RUSTEZE_CAPTURE_HELPER` to use a helper binary
at another path during development.

Before recording, allow the helper under **Microphone** and **Screen Recording**
in System Settings. You can request the initial macOS prompts with:

```bash
./macos-helper/.build/debug/rusteze-capture-helper request-permissions
```

`rusteze transcribe <session-path>` is present as the model-independent
transcription boundary. It intentionally reports that no engine is configured
until a local engine (such as whisper.cpp) is selected and implemented.

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
- Windows or Linux support.

## Planned command shape

```text
rusteze start [title]
rusteze create-meeting [title]
rusteze transcribe <session-path>
```

We will introduce these commands one at a time as the project grows.
