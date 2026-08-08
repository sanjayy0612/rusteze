# Rusteze

Rusteze is a local-first meeting recorder written in Rust. It captures system audio, microphone audio, or both as separate tracks, keeps the recording on your computer, and provides a replaceable boundary for local transcription.

> Rusteze is a learning project and is currently in active development. Real hardware capture must still be validated on each target operating system.

Always tell participants that a recording is being made, obtain the required consent, and follow the laws and policies that apply to your meeting.

## What it does

- Captures audio directly on macOS, Windows, and Linux.
- Supports three recording modes:
  - system audio only — the default;
  - system audio plus microphone — `--mic`;
  - microphone only — `--mic-only`.
- Keeps microphone and system audio in independent files; it does not mix them.
- Stores each meeting in its own folder with lifecycle metadata in `session.json`.
- Stops cleanly with `Ctrl+C` and detects interrupted sessions on the next start.
- Keeps audio, metadata, and future transcripts local.
- Leaves the transcription engine replaceable instead of locking the project to one model.

## Platform backends

| Platform | Capture implementation | System track | Microphone track |
| --- | --- | --- | --- |
| macOS | Swift helper using Apple audio frameworks | `system.caf` | `mic.caf` |
| Windows | Native Rust WASAPI | `system.wav` | `mic.wav` |
| Linux | Native Rust PipeWire | `system.wav` | `mic.wav` |

The Windows backend uses the default render endpoint in loopback mode for system audio and the default capture endpoint for the microphone. The Linux backend uses PipeWire’s active output monitor and default input source. No device names are hardcoded.

## Quick start

### Prerequisites

Install Rust through [rustup](https://rustup.rs/), then clone the repository:

```bash
git clone <repository-url>
cd rusteze
```

Build and run the CLI:

```bash
cargo build
cargo run -- start "Rust workshop"
```

Press `Ctrl+C` to finish the recording safely.

### Recording modes

```bash
# System audio only (default)
cargo run -- start "Project sync"

# System audio and microphone
cargo run -- start "Project sync" --mic

# Microphone only
cargo run -- start "Project sync" --mic-only
```

The title is optional. If omitted, Rusteze uses `untitled`.

### Permissions

Request only the permissions required by a mode:

```bash
cargo run -- request-permissions
cargo run -- request-permissions --mic
cargo run -- request-permissions --mic-only
```

On macOS, enable the terminal or Rusteze binary under **System Settings → Privacy & Security → Microphone** and **Screen & System Audio Recording**. The macOS helper asks for the permissions required by the selected mode.

Windows permissions and endpoint availability are managed by Windows and the active audio session. Linux has no Rusteze permission prompt; PipeWire and the desktop session determine whether the default nodes are available.

## macOS setup

The macOS capture boundary is a small Swift helper built with Xcode’s Swift compiler:

```bash
./macos-helper/build.sh
cargo run -- start "macOS test"
```

The helper can also request permissions directly:

```bash
./macos-helper/.build/debug/rusteze-capture-helper request-permissions system
```

To use a helper binary at another path during development:

```bash
RUSTEZE_CAPTURE_HELPER=/path/to/rusteze-capture-helper \
  cargo run -- start "Custom helper test"
```

## Windows setup

Build on Windows with a Windows Rust toolchain:

```powershell
cargo build
cargo build --release
cargo run -- start "Windows test"
```

Rusteze uses COM to initialize the native Windows audio APIs and WASAPI to read the default render endpoint in loopback mode and the default microphone endpoint. No external recorder is required.

## Linux setup

Rusteze uses the native [`pipewire`](https://crates.io/crates/pipewire) Rust bindings. Install the development headers before building.

### Debian or Ubuntu

```bash
sudo apt update
sudo apt install build-essential pkg-config clang libclang-dev \
  libpipewire-0.3-dev libspa-0.2-dev
```

### Fedora

```bash
sudo dnf install gcc gcc-c++ make pkgconf-pkg-config clang pipewire-devel
```

### Arch Linux

```bash
sudo pacman -S --needed base-devel pkgconf clang pipewire libpipewire
```

Then build and run with an active PipeWire desktop session:

```bash
cargo build
cargo test
cargo run -- start "Linux test"
```

PipeWire negotiates the graph’s sample rate and channel count. Rusteze converts the negotiated floating-point frames to PCM16 WAV. The current Linux backend intentionally uses the default/active nodes; explicit device selection, portal integration, PulseAudio fallback, and seamless device-change recovery are future work.

## Where recordings go

Each recording is stored under:

```text
~/Documents/rusteze/meetings/
└── <timestamp>-<meeting-title>/
    ├── session.json
    ├── system.caf       # macOS, when system audio is enabled
    ├── mic.caf          # macOS, when microphone is enabled
    ├── system.wav       # Windows/Linux, when system audio is enabled
    ├── mic.wav          # Windows/Linux, when microphone is enabled
    ├── transcript.md    # written when a transcription engine is available
    └── transcript.json   # written when a transcription engine is available
```

Rusteze checks for at least 256 MiB of free space before recording. A session moves through `recording`, `stopping`, and `completed`. If capture or finalization fails, it records a `failed` state and a recoverable reason in `session.json`.

## Transcription status

The CLI already exposes the transcription boundary:

```bash
cargo run -- transcribe ~/Documents/rusteze/meetings/<session-folder>
```

The bundled engine is intentionally unconfigured. The project defines the portable transcript types and output files, but a local engine such as whisper.cpp still needs to be selected and connected.

## CLI reference

```text
rusteze start [title] [--mic|--mic-only]
rusteze request-permissions [--mic|--mic-only]
rusteze create-meeting [title]
rusteze transcribe <session-path>
```

## Architecture

```text
Rust CLI
  ├─ creates the meeting folder and session.json
  ├─ selects the capture mode
  ├─ starts and supervises the platform backend
  ├─ handles Ctrl+C
  └─ finalizes session state

macOS: Swift helper → Apple audio frameworks → CAF tracks
Windows: Rust → WASAPI → WAV tracks
Linux: Rust → PipeWire → WAV tracks

Completed session → TranscriptionEngine → transcript.md + transcript.json
```

The platform backends remain behind the native capture boundary. Shared Rust code owns session policy, metadata, shutdown, disk checks, and portable output behavior. See [ARCHITECTURE.md](ARCHITECTURE.md), [CONTEXT.md](CONTEXT.md), and [AGENT.md](AGENT.md) for repository-maintainer notes.

## Development checks

Run the platform-independent checks on every change:

```bash
cargo fmt -- --check
cargo test
git diff --check
```

macOS build check:

```bash
cargo build
./macos-helper/build.sh
```

Windows build check:

```powershell
cargo build
```

Linux build check:

```bash
cargo build
cargo test
```

Manual Linux capture scenarios are documented in [docs/linux-pipewire-testing.md](docs/linux-pipewire-testing.md). Windows manual scenarios are documented in [docs/windows-wasapi-testing.md](docs/windows-wasapi-testing.md).

## Project boundaries

Rusteze does not currently provide:

- cloud recording or cloud transcription;
- automatic meeting bots;
- LLM summaries or publishing integrations;
- per-application audio capture;
- explicit Linux device selection;
- a production transcription model;
- background daemon or system-service support.

These boundaries keep the project local-first, testable, and small enough to understand while the native capture paths mature.
