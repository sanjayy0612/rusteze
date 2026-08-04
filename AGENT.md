# Rusteze agent guide

## Read first

Read `README.md`, `PLAN.md`, `CONTEXT.md`, `ARCHITECTURE.md`, and this file before changing the project.

## Current milestone

The foreground two-track recording path is implemented in source and the Swift helper compile-checks with Xcode. It still needs an end-to-end recording test after the user grants Microphone and Screen Recording access.

```bash
./macos-helper/build.sh
./macos-helper/.build/debug/rusteze-capture-helper request-permissions
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

The Swift build uses Xcode’s compiler cache and may need to run outside a restricted sandbox. Do not attempt real audio capture without the user’s explicit permission and macOS privacy approval.
