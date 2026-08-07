# Architecture notes

## Runtime boundary

```text
Rust CLI
  ├─ creates session folder + session.json
  ├─ selects system, microphone, or both capture mode
  ├─ launches and supervises native helper
  ├─ receives Ctrl+C and sends helper: stop
  └─ finalizes session state

Swift macOS helper
  ├─ preflights / requests only the permissions needed by the mode
  ├─ AVAudioEngine → mic.caf when microphone capture is enabled
  └─ ScreenCaptureKit → system.caf when system capture is enabled

Completed session → TranscriptionEngine trait → transcript.md + transcript.json
```

The helper receives a tiny line-based protocol: `check-permissions [mode]`,
`request-permissions [mode]`, and `record <session-folder> <mode>`. During
recording Rust sends `stop` on standard input; it does not force-kill the helper.

## Source layout

```text
src/
├── main.rs            # command routing and foreground lifecycle
├── meeting.rs         # session folders, JSON metadata, lifecycle states
├── native_helper.rs   # helper protocol, process supervision, permissions
└── transcription.rs   # model-independent transcript types and writer

macos-helper/
├── Sources/main.swift # permissions, AVAudioEngine, ScreenCaptureKit
└── build.sh           # builds the helper with Xcode's Swift compiler
```

## Deliberate boundaries

- Rust owns session state and file ownership, not hardware drivers.
- Swift owns Apple-framework interaction, not CLI policy or transcript logic.
- `TranscriptionEngine` keeps Whisper, whisper.cpp, and alternatives replaceable.
- AI summarization and publishing remain outside this repository.
