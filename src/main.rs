mod meeting;
mod native_helper;
mod transcription;

use std::{env, process, sync::mpsc};
use transcription::TranscriptionEngine;

fn main() {
    let mut arguments = env::args().skip(1);

    match arguments.next().as_deref() {
        Some("start") => {
            let title = arguments.next().unwrap_or_else(|| "untitled".to_string());

            if arguments.next().is_some() {
                print_usage();
                process::exit(2);
            }

            start_recording(&title);
        }
        Some("create-meeting") => {
            let title = arguments.next().unwrap_or_else(|| "untitled".to_string());

            if arguments.next().is_some() {
                print_usage();
                process::exit(2);
            }

            match meeting::create(&title) {
                Ok(folder) => println!("Created meeting folder: {}", folder.display()),
                Err(error) => {
                    eprintln!("Could not create meeting folder: {error}");
                    process::exit(1);
                }
            }
        }
        Some("transcribe") => {
            let Some(session_path) = arguments.next() else {
                print_usage();
                process::exit(2);
            };
            if arguments.next().is_some() {
                print_usage();
                process::exit(2);
            }
            transcribe(&session_path);
        }
        _ => print_usage(),
    }
}

fn transcribe(session_path: &str) {
    let engine = transcription::UnconfiguredEngine;
    match engine.transcribe(std::path::Path::new(session_path)) {
        Ok(transcript) => {
            match transcription::write_transcript(std::path::Path::new(session_path), &transcript) {
                Ok(()) => println!("Transcript written to {session_path}"),
                Err(error) => {
                    eprintln!("{error}");
                    process::exit(1);
                }
            }
        }
        Err(error) => {
            eprintln!("{error}");
            process::exit(1);
        }
    }
}

fn start_recording(title: &str) {
    let mut session = match meeting::start(title) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("Could not start meeting session: {error}");
            process::exit(1);
        }
    };

    let permissions = match native_helper::check_permissions() {
        Ok(permissions) => permissions,
        Err(error) => fail_and_exit(&mut session, &error.to_string()),
    };

    if !permissions.is_ready_to_record() {
        fail_and_exit(&mut session, &permissions.guidance());
    }

    let capture = match native_helper::start_capture(session.folder()) {
        Ok(capture) => capture,
        Err(error) => fail_and_exit(&mut session, &error.to_string()),
    };

    println!("Recording session: {}", session.folder().display());
    println!("Recording microphone and system audio. Press Ctrl+C to stop safely.");

    let (shutdown_sender, shutdown_receiver) = mpsc::channel();
    if let Err(error) = ctrlc::set_handler(move || {
        // Signal handlers must stay small. The main thread owns finalization.
        let _ = shutdown_sender.send(());
    }) {
        let _ = meeting::fail(&mut session, "Could not install Ctrl+C handler");
        eprintln!("Could not listen for Ctrl+C: {error}");
        process::exit(1);
    }

    if shutdown_receiver.recv().is_err() {
        let _ = meeting::fail(&mut session, "Shutdown signal channel closed unexpectedly");
        eprintln!("Recording stopped unexpectedly.");
        process::exit(1);
    }

    if let Err(error) = capture.stop() {
        fail_and_exit(
            &mut session,
            &format!("Could not finalize audio files: {error}"),
        );
    }

    match meeting::complete(&mut session) {
        Ok(()) => println!("Session completed: {}", session.folder().display()),
        Err(error) => {
            let _ = meeting::fail(
                &mut session,
                &format!("Could not finalize session: {error}"),
            );
            eprintln!("Could not finalize meeting session: {error}");
            process::exit(1);
        }
    }
}

fn fail_and_exit(session: &mut meeting::MeetingSession, reason: &str) -> ! {
    let _ = meeting::fail(session, reason);
    eprintln!("{reason}");
    process::exit(1);
}

fn print_usage() {
    println!("Usage:");
    println!("  rusteze start [title]");
    println!("  rusteze create-meeting [title]");
    println!("  rusteze transcribe <session-path>");
    println!("Example: rusteze start \"Rust workshop\"");
}
