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

## Current phase: native macOS helper and permission preflight

`rusteze start [title]` creates a self-contained session folder under
`~/Documents/rusteze/meetings`, asks the included native macOS helper to check
Microphone and Screen Recording access, records the lifecycle in `session.json`,
and stops cleanly with `Ctrl+C`. Audio capture is deliberately not connected
yet; the next phase adds microphone capture.

```bash
cargo run -- start "Rust workshop"
```

The session progresses from `recording` to `stopping` to `completed`. If the
program cannot continue normally, `session.json` records a `failed` state and
a recoverable reason. To grant missing access, open **System Settings → Privacy
& Security**, then enable Rusteze's terminal/binary under **Microphone** and
**Screen Recording**.

The helper source lives at `macos-helper/Sources/main.swift`. Build it once
before running `start`:

```bash
./macos-helper/build.sh
```

Its current `check-permissions` protocol is the boundary Rust will use when
capture commands are added. Set `RUSTEZE_CAPTURE_HELPER` to use a helper binary
at another path during development.

Not in the first version:

- background daemon support;
- transcription or LLM summaries;
- per-app audio capture;
- Windows or Linux support.

## Planned command shape

```text
rusteze start
rusteze start [title]
rusteze status
rusteze list
rusteze transcribe <session-id-or-path>
```

We will introduce these commands one at a time as the project grows.
