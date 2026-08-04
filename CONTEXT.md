# Project context

## Why Rusteze exists

Rusteze is a personal, local-first replacement for hosted meeting-recording and transcript tools. It is also a Rust and macOS-audio learning project. Recording data stays on the Mac; optional AI summaries and publishing are separate tools.

## What Rusteze owns

1. Create one local folder per meeting.
2. Capture microphone and system audio as separate CAF tracks.
3. Track session lifecycle and recoverable failures in `session.json`.
4. Define and persist a portable transcript result once a local engine exists.

## What Rusteze does not own

- LLM summaries, cloud upload, or cloud transcription.
- Notion, Obsidian, MCP, or other provider integrations.
- A fixed transcription model. The Rust trait is ready; an engine is not yet selected.

## Current working state

- The Rust CLI supports `start`, `create-meeting`, and `transcribe`.
- `start` launches the Swift helper, preflights Microphone and Screen Recording permissions, and on success records `mic.caf` and `system.caf`.
- `Ctrl+C` asks the helper to finish both files before the session is marked `completed`.
- The helper builds with `./macos-helper/build.sh` on a Mac with full Xcode.
- The transcription boundary writes `transcript.md` and `transcript.json` for a future engine result. The bundled engine intentionally returns “not configured.”

## Current folder contract

```text
~/Documents/rusteze/meetings/
└── <timestamp>-<meeting-title>/
    ├── session.json
    ├── mic.caf                 # after a successful recording
    ├── system.caf              # after a successful recording
    ├── transcript.md           # after a transcription engine writes a result
    └── transcript.json         # after a transcription engine writes a result
```

## Important limitations

- The current Mac must grant Microphone and Screen Recording access before a real recording can begin.
- Real hardware recording has not yet been validated on this machine because those permissions are currently denied/missing.
- Interrupted-session recovery, low-disk-space preflight (256 MiB), and unexpected helper-exit detection are implemented.
- Device-change/sleep validation, `status`, `list`, and a real transcription engine are still future work.
