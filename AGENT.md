# Rusteze agent guide

## Read first

Read `README.md`, `PLAN.md`, `CONTEXT.md`, `ARCHITECTURE.md`, and this file before changing the project.

## Current milestone

The selectable foreground capture path is implemented for macOS, Windows, and
Linux. macOS uses the Swift helper; Windows uses native WASAPI; Linux uses
native PipeWire. Each platform still needs end-to-end recording tests on its
respective operating system.

```bash
./macos-helper/build.sh
./macos-helper/.build/debug/rusteze-capture-helper request-permissions system
cargo run -- start "Rust workshop"
```

## Working rules

- Keep recordings, transcripts, and metadata local.
- Do not add AI-provider, Notion, Obsidian, cloud, or MCP code.
- Do not overwrite recordings or transcripts.
- Treat helper process failures as recoverable session failures.
- Preserve separate microphone and system-audio tracks; do not mix them yet.
- Keep the transcription engine replaceable. Do not choose one without an explicit implementation decision.
- Add focused tests for Rust parsing, state changes, and file formats.

## Verification

```bash
cargo fmt --check
cargo test
./macos-helper/build.sh
```

The Swift build uses Xcode’s compiler cache and may need to run outside a restricted sandbox. Windows WASAPI requires a real Windows audio endpoint. Linux requires PipeWire development files at build time and a running desktop PipeWire session at runtime. Do not attempt real audio capture without the user’s explicit permission and appropriate OS privacy approval.
