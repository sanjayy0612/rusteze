# Linux PipeWire manual test checklist

Install the development prerequisites from `README.md`, ensure the PipeWire
daemon and desktop session are running, then run the checks from the repository:

```bash
cargo fmt -- --check
cargo test
cargo build
```

## System audio only

```bash
cargo run -- start "Linux system audio"
```

Play browser or meeting audio through the active/default output, stop with
`Ctrl+C`, and verify that the session contains `system.wav` only. Confirm that
the WAV contains audible playback.

## Microphone only

```bash
cargo run -- start "Linux microphone" --mic-only
```

Speak into the active/default microphone, stop with `Ctrl+C`, and verify that
the session contains `mic.wav` only.

## Both tracks

```bash
cargo run -- start "Linux both" --mic
```

Play system audio while speaking, stop with `Ctrl+C`, and verify that both WAV
files exist and remain independently audible.

## Failure and device checks

1. Disconnect or disable the microphone. Confirm microphone-only capture gives a
   useful PipeWire error, while system-only capture can still start.
2. Disconnect or switch the active output while recording. Confirm Rusteze either
   continues with the new PipeWire route or stops cleanly with a useful error.
3. Run inside a restricted Flatpak/Snap environment only as an explicit future
   experiment; portal integration is not included in this version.

Linux hardware capture is not simulated by the automated tests. It requires a
real PipeWire desktop session with active input/output nodes.
