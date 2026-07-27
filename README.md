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

## Phase 1: Rust recorder

The first goal is a Rust command that records audio in the foreground and stops cleanly with `Ctrl+C`.

Not in the first version:

- background daemon support;
- transcription or LLM summaries;
- per-app audio capture;
- Windows or Linux support.

## Planned command shape

```text
rusteze start
rusteze stop
rusteze status
rusteze list
rusteze transcribe <file>
rusteze summarize <file>
```

We will introduce these commands one at a time as the project grows.
