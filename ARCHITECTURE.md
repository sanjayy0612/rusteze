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

Windows Rust backend
  ├─ default render endpoint + WASAPI loopback → system.wav
  └─ default capture endpoint + WASAPI → mic.wav

Completed session → TranscriptionEngine trait → transcript.md + transcript.json
```

On macOS, the Swift helper receives a tiny line-based protocol:
`check-permissions [mode]`, `request-permissions [mode]`, and
`record <session-folder> <mode>`. During recording Rust sends `stop` on standard
input; it does not force-kill the helper. On Windows, the Rust backend owns the
same lifecycle directly through WASAPI and supervises one capture worker per
enabled source.

## Source layout

```text
src/
├── main.rs            # command routing and foreground lifecycle
├── meeting.rs         # session folders, JSON metadata, lifecycle states
├── native_helper.rs   # platform boundary, process supervision, permissions
├── windows.rs         # native Windows WASAPI capture backend
├── unsupported.rs      # explicit Linux/other-platform boundary
└── transcription.rs   # model-independent transcript types and writer

macos-helper/
├── Sources/main.swift # permissions, AVAudioEngine, ScreenCaptureKit
└── build.sh           # builds the helper with Xcode's Swift compiler
```

## Deliberate boundaries

- Rust owns session state and file ownership, not hardware drivers.
- Swift owns Apple-framework interaction, not CLI policy or transcript logic.
- Windows Rust code owns WASAPI interaction and independent WAV writers.
- `TranscriptionEngine` keeps Whisper, whisper.cpp, and alternatives replaceable.
- AI summarization and publishing remain outside this repository.
