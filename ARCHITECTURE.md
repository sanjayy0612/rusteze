# Architecture notes

## Runtime boundary

```text
Rust CLI
  ├─ creates session folder + session.json
  ├─ selects system, microphone, or both capture mode
  ├─ selects and supervises the platform capture backend
  ├─ receives Ctrl+C and requests backend stop
  └─ finalizes session state

Swift macOS helper
  ├─ preflights / requests only the permissions needed by the mode
  ├─ AVAudioEngine → mic.caf when microphone capture is enabled
  └─ ScreenCaptureKit → system.caf when system capture is enabled

Windows Rust backend
  ├─ default render endpoint + WASAPI loopback → system.wav
  └─ default capture endpoint + WASAPI → mic.wav

Linux Rust backend
  ├─ default output monitor + PipeWire → system.wav
  └─ default input source + PipeWire → mic.wav

Completed session → TranscriptionEngine trait → transcript.md + transcript.json
```

On macOS, the Swift helper receives a tiny line-based protocol:
`check-permissions [mode]`, `request-permissions [mode]`, and
`record <session-folder> <mode>`. During recording Rust sends `stop` on standard
input; it does not force-kill the helper. On Windows and Linux, the Rust backend
owns the same lifecycle directly through WASAPI or PipeWire and supervises one
capture worker per enabled source.

## Source layout

```text
src/
├── main.rs            # command routing and foreground lifecycle
├── meeting.rs         # session folders, JSON metadata, lifecycle states
├── native_helper.rs   # platform boundary, process supervision, permissions
├── audio.rs            # shared PCM16 WAV writer and float conversion
├── windows.rs         # native Windows WASAPI capture backend
├── linux.rs            # native Linux PipeWire capture backend
├── unsupported.rs      # explicit other-platform boundary
└── transcription.rs   # model-independent transcript types and writer

macos-helper/
├── Sources/main.swift # permissions, AVAudioEngine, ScreenCaptureKit
└── build.sh           # builds the helper with Xcode's Swift compiler
```

## Deliberate boundaries

- Rust owns session state and file ownership, not hardware drivers.
- Swift owns Apple-framework interaction, not CLI policy or transcript logic.
- Windows Rust code owns WASAPI interaction and independent WAV writers.
- Linux Rust code owns PipeWire stream lifecycle and default-node capture.
- Windows and Linux share the PCM16 WAV writer and float conversion helpers.
- `TranscriptionEngine` keeps Whisper, whisper.cpp, and alternatives replaceable.
- AI summarization and publishing remain outside this repository.
