# Rusteze — System Plan

## Purpose

Rusteze is a personal, **local-first macOS and Windows CLI** for recording meetings and turning completed recordings into local transcripts.

It is a Rust learning project as well as a useful tool. The core program owns meeting capture, session folders, metadata, and transcription orchestration. It does **not** contain AI-agent, Notion, Obsidian, or cloud-provider code.

The intended flow is:

```text
rusteze start
  -> capture microphone + meeting playback locally
  -> stop cleanly
  -> preserve a self-contained session folder
  -> transcribe locally
  -> produce transcript.md and transcript.json
```

Later, an external AI agent may read a transcript and create summaries or publish notes through its own MCP connection. That is a separate, optional workflow—not part of Rusteze itself.

## Guiding Rules

- Build for personal use on macOS first.
- Learn one small concept at a time, while keeping the whole system design visible.
- Keep recordings and transcripts local.
- Keep Rusteze independent from AI-agent, Notion, Obsidian, and API integrations.
- Add complexity only when the previous step works and is understood.
- Prefer safe, recoverable recording behavior over clever features.
- The transcript engine is replaceable; do not lock Whisper or another model until the recording pipeline is reliable.
- Obtain participant consent and follow applicable meeting-recording laws and policies.

## Current Product Boundary

### Included

- macOS and Windows capture backends
- Terminal-first workflow
- Foreground recording with clean `Ctrl+C` shutdown
- Microphone capture as one track
- System-audio capture as a separate track
- One local folder per meeting
- Local transcription boundary after recording (no engine selected yet)
- Session metadata

### Not Included Yet

- Summaries or LLM calls inside Rusteze
- Cloud upload or cloud transcription
- Choosing a fixed Whisper/model implementation
- Notion or Obsidian API code
- AI-agent/MCP code
- Per-application audio capture
- Automatic meeting detection
- Multi-speaker diarization
- Background daemon recording
- GUI application
- Linux support
- Homebrew packaging

## Architecture

Rusteze has a Rust CLI and a platform-specific capture backend.

```text
Rust CLI                         Capture backend
--------                         ---------------
commands and session state       macOS: Swift helper
meeting folders and metadata     Windows: native WASAPI
Ctrl+C / graceful shutdown       captures microphone audio
transcription orchestration      captures system audio
```

The Rust CLI is the project brain. It owns the command-line UX, creates sessions, tracks recording state, writes metadata, and later invokes a local transcription engine.

The macOS helper uses Apple-supported frameworks instead of Rusteze speaking
directly to hardware drivers. The Windows backend uses shared-mode WASAPI with
the default render endpoint in loopback mode and the default capture endpoint
for microphone input.

| Rusteze need | macOS responsibility |
|---|---|
| Your own voice | `AVAudioEngine` / Core Audio microphone capture |
| Zoom/Meet playback | `ScreenCaptureKit` system-audio capture |
| Permission decisions | macOS privacy system |
| Stored session files | normal filesystem APIs |

The real path is:

```text
Rust CLI -> native helper -> Apple framework -> macOS -> audio source
```

macOS continuously gives the helper tiny new pieces of microphone and system audio. The helper sends them for local storage. Rusteze does not implement audio drivers or control hardware directly.

## Permissions and Packaging

Before recording, the finished app needs the permissions required by the selected
capture mode:

1. **Microphone access** for microphone-only or combined capture.
2. **Screen Recording access** for system-only or combined capture.

Rusteze stays terminal-first, but the finished project should be packaged and signed as a small macOS app/helper. That gives macOS a stable identity for privacy permissions and makes installation on another Mac reliable.

Missing permission is an expected program result. Rusteze should explain what is missing and how to grant it instead of crashing.

## Session Folder Contract

Every meeting is a self-contained local folder.

```text
~/Documents/rusteze/meetings/<session-id>/
  session.json
  mic.<audio-format>
  system.<audio-format>
  transcript.md               # created after transcription
  transcript.json             # timestamps and structured segments
```

Sessions contain only the enabled microphone and/or system-audio track. macOS
uses `mic.caf` / `system.caf`; Windows uses `mic.wav` / `system.wav`. The
mode-aware `session.json` stores the session ID, title, lifecycle state, capture
mode, enabled tracks, start/end times, duration, and a recoverable failure
reason.

Keep microphone and system audio as separate tracks initially. This avoids premature mixing problems and leaves room for better playback, transcript alignment, or speaker work later.

## Commands

```bash
rusteze start
rusteze status
rusteze list
rusteze transcribe <session-id-or-path>
```

Initially, `rusteze start` stays in the foreground and `Ctrl+C` stops it. A separate `rusteze stop` command belongs to the later background-daemon stage.

## Main Runtime Flow

```text
$ rusteze start
        |
        v
create a new meeting folder and session.json
        |
        v
check microphone and Screen Recording permissions
        |
        v
start mic + system-audio capture through the native helper
        |
        v
save incoming audio continuously as separate local tracks
        |
        v
user presses Ctrl+C
        |
        v
stop capture streams, finalize files, mark session completed
        |
        v
$ rusteze transcribe <session>
        |
        v
run a local transcription adapter
        |
        v
write transcript.md and transcript.json
```

## Recording Rules

During a meeting, protecting audio is the only priority.

The recording path should only:

1. Receive new audio from macOS.
2. Preserve timing information.
3. Queue short chunks when necessary.
4. Write the tracks safely to disk.

It must not run transcription, summaries, model downloads, or other expensive processing during capture. Those tasks can run only after the meeting ends.

Start with the easiest stable capture format provided by the macOS pipeline. Do not commit to Opus yet. Opus can later become an optional compressed/export format once capture and transcription are proven reliable.

## Clean Stop and Recovery

When the user presses `Ctrl+C`:

1. Rust changes session state from `Recording` to `Stopping`.
2. Rust tells the macOS helper to stop both capture streams.
3. The helper delivers/finalizes any remaining audio.
4. Rust closes files and computes final metadata.
5. Rust marks the session `Completed` in `session.json`.

If permissions are denied, an audio device disconnects, the Mac sleeps, or capture fails, Rusteze should preserve usable audio already written and mark the session with a clear failure/recovery reason.

## Local Transcription Boundary

Rusteze uses a stable interface rather than tying the project to one model:

```text
completed session -> transcription adapter -> transcript result
```

The adapter receives a completed session and returns:

- transcript text
- timestamped segments
- engine/model metadata
- a clear error if transcription fails

Whisper, whisper.cpp, faster-whisper, or another local model can be added later as one implementation of this adapter without redesigning recording, session folders, or commands.

## External Agent Workflow — Later

Once `transcript.md` exists, a separate AI agent may:

```text
transcript.md
  -> optional summary/notes
  -> optional publish to Notion or Obsidian through that agent's MCP connection
```

This is intentionally outside the Rusteze repository. The transcript remains a simple portable file that Codex or any other tool can consume when the user chooses.

## Suggested Repository Shape

```text
rusteze/
  src/                         # early single-crate learning stage
  crates/                      # split only when the project earns it
    rusteze-cli/               # command parsing and messages
    rusteze-core/              # session state and domain types
    rusteze-storage/           # folders, metadata, audio finalization
    rusteze-transcription/     # model-independent adapter
  macos-helper/                # Swift helper for Apple audio APIs
  docs/
  tests/
  PLAN.md
  README.md
```

Do not create the multi-crate workspace immediately. Begin in one understandable Cargo binary project, then split only when modules become clearly independent.

## Build Order

### 1. Project Foundation — Complete

- Cargo binary project.
- `README.md` explaining the purpose.
- Basic command entry point.

### 2. Meeting Folder Structure — Complete

- Choose the default directory: `~/Documents/rusteze/meetings`.
- Create one dated/unique folder per session.
- Write initial `session.json` with start time and state.

### 3. Foreground Recording State — Complete

- Implement `rusteze start`.
- Model states such as `Idle`, `Recording`, `Stopping`, `Completed`, and `Failed`.
- Handle `Ctrl+C` safely.
- Finalize a session even before real audio capture is connected.

### 4. Native Helper Skeleton — Complete

- Create a minimal Swift/macOS helper alongside Rust.
- Make Rust start it and receive clear success/failure messages.
- Add the permission-check flow before recording.

### 5. Microphone Capture — Implemented; hardware validation pending

- Request microphone access.
- Capture one microphone track through `AVAudioEngine` / Core Audio.
- Save it into the current meeting folder.
- Test with a short spoken recording.

### 6. System-Audio Capture — Implemented; hardware validation pending

- Request Screen Recording access.
- Capture macOS meeting playback through ScreenCaptureKit.
- Save it as a separate system-audio track.
- Test with Zoom, Meet, or a simple audio source.

### 7. Two-Track Session Integration — Implemented; end-to-end validation pending

- Start both sources in one session.
- Preserve timing and separate tracks.
- Confirm clean `Ctrl+C` shutdown and valid files.

### 8. Reliability Cases — Partially complete

- [x] Permission denial produces a failed session with setup guidance.
- [x] Missing/unavailable microphone or display causes helper startup to fail safely.
- [x] Interrupted `recording`/`stopping` sessions are recovered as failed on the next `start`; existing audio is preserved.
- [x] A 256 MiB free-space preflight prevents unsafe recording starts.
- [x] Rust detects an unexpectedly exited helper and records a recoverable failure.
- [ ] Validate audio-device switch/disconnect and sleep behavior on real hardware.

### 9. Local Transcription Adapter — Interface complete; engine pending

- Define the model-independent transcription interface.
- Produce `transcript.md` and `transcript.json`.
- Select and add the first local engine only after this interface is ready.

### 10. Later Improvements

- Background daemon plus `stop`, `status`, and managed sessions.
- Search across transcripts.
- Output-directory flag.
- Optional audio compression/export.
- Per-application capture.
- Speaker diarization.
- External agent workflows for summaries/Notion/Obsidian.
- Homebrew packaging after the personal workflow is reliable.

## Rust Learning Map

Each step should introduce Rust because the project needs it:

| Project need | Rust concept |
|---|---|
| Recording lifecycle | enums and `match` |
| Session object | structs and `impl` |
| Changing state | `&mut self` |
| File ownership | ownership and borrowing |
| Permission/file failures | `Result` and `Option` |
| Separate files/modules | `mod`, `pub`, and `use` |
| Replaceable transcript engines | traits |
| Helper communication | processes, messages, and later IPC |

## Definition of Success

Rusteze is successful when you can run one terminal command, approve macOS permissions, capture both your own voice and the meeting playback into a local session folder, stop safely with `Ctrl+C`, and produce a local timestamped transcript—while understanding which part of Rust and macOS performed each job.
