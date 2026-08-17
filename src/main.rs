mod audio;
mod meeting;
mod native_helper;
mod prepare_audio;
mod storage;
mod transcription;

use native_helper::CaptureMode;
use std::{
    env, process,
    sync::mpsc,
    time::{Duration, Instant},
};
use transcription::TranscriptionEngine;

fn main() {
    let mut arguments = env::args().skip(1);

    match arguments.next().as_deref() {
        Some("start") => {
            let start_arguments = arguments.collect::<Vec<_>>();
            let (title, mode) = match parse_start_arguments(&start_arguments) {
                Ok(arguments) => arguments,
                Err(error) => {
                    eprintln!("Invalid start arguments: {error}");
                    print_usage();
                    process::exit(64);
                }
            };

            start_recording(&title, mode);
        }
        Some("create-meeting") => {
            let title = arguments.next().unwrap_or_else(|| "untitled".to_string());

            if arguments.next().is_some() {
                print_usage();
                process::exit(64);
            }

            match meeting::create(&title) {
                Ok(folder) => println!("Created meeting folder: {}", folder.display()),
                Err(error) => {
                    eprintln!("Could not create meeting folder: {error}");
                    process::exit(1);
                }
            }
        }
        Some("request-permissions") => {
            let mode_arguments = arguments.collect::<Vec<_>>();
            let mode = match parse_mode_arguments(&mode_arguments) {
                Ok(mode) => mode,
                Err(error) => {
                    eprintln!("Invalid permission arguments: {error}");
                    print_usage();
                    process::exit(64);
                }
            };

            match native_helper::request_permissions(mode) {
                Ok(permissions) if permissions.is_ready_to_record(mode) => {
                    println!(
                        "Required permissions granted for {} capture.",
                        mode.as_str()
                    );
                }
                Ok(permissions) => {
                    eprintln!("{}", permissions.guidance(mode));
                    process::exit(77);
                }
                Err(error) => {
                    eprintln!("{error}");
                    process::exit(error.exit_code());
                }
            }
        }
        Some("transcribe") => {
            let Some(session_path) = arguments.next() else {
                print_usage();
                process::exit(64);
            };
            if arguments.next().is_some() {
                print_usage();
                process::exit(64);
            }
            transcribe(&session_path);
        }
        Some("mix") => {
            let Some(session_path) = arguments.next() else {
                print_usage();
                process::exit(64);
            };
            if arguments.next().is_some() {
                print_usage();
                process::exit(64);
            }
            mix_audio(&session_path);
        }
        Some("prepare-audio") => {
            let Some(input_path) = arguments.next() else {
                print_usage();
                process::exit(64);
            };
            let output_path = arguments.next();
            if arguments.next().is_some() {
                print_usage();
                process::exit(64);
            }

            match prepare_audio::prepare(
                std::path::Path::new(&input_path),
                output_path.as_deref().map(std::path::Path::new),
            ) {
                Ok(path) => println!("Prepared audio written to {}", path.display()),
                Err(error) => {
                    eprintln!("Could not prepare audio: {error}");
                    process::exit(1);
                }
            }
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

fn mix_audio(session_path: &str) {
    match audio::mix_tracks(std::path::Path::new(session_path)) {
        Ok(path) => println!("Mixed audio written to {}", path.display()),
        Err(error) => {
            eprintln!("Could not mix audio: {error}");
            process::exit(1);
        }
    }
}

fn start_recording(title: &str, mode: CaptureMode) {
    match meeting::recover_interrupted_sessions() {
        Ok(recovered) if !recovered.is_empty() => {
            eprintln!("Recovered {} interrupted session(s).", recovered.len());
        }
        Ok(_) => {}
        Err(error) => {
            eprintln!("Could not check for interrupted sessions: {error}");
            process::exit(1);
        }
    }

    let mut session = match meeting::start(title, mode) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("Could not start meeting session: {error}");
            process::exit(1);
        }
    };

    if let Err(error) = meeting::ensure_recording_space(&session) {
        fail_and_exit(
            &mut session,
            &format!("Not enough disk space to record safely: {error}"),
        );
    }

    let permissions = match native_helper::check_permissions(mode) {
        Ok(permissions) => permissions,
        Err(error) => {
            let exit_code = error.exit_code();
            fail_and_exit_with_code(&mut session, &error.to_string(), exit_code)
        }
    };

    if !permissions.is_ready_to_record(mode) {
        fail_and_exit_with_code(&mut session, &permissions.guidance(mode), 77);
    }

    let mut capture = match native_helper::start_capture(session.folder(), mode) {
        Ok(capture) => capture,
        Err(error) => fail_and_exit_with_code(&mut session, &error.to_string(), error.exit_code()),
    };

    println!("Recording session: {}", session.folder().display());
    println!(
        "Recording {}. Press Ctrl+C to stop safely.",
        match mode {
            CaptureMode::System => "system audio only",
            CaptureMode::Microphone => "microphone only",
            CaptureMode::Both => "microphone and system audio",
        }
    );

    let (shutdown_sender, shutdown_receiver) = mpsc::channel();
    if let Err(error) = ctrlc::set_handler(move || {
        // Signal handlers must stay small. The main thread owns finalization.
        let _ = shutdown_sender.send(());
    }) {
        let _ = capture.stop();
        let _ = meeting::fail(&mut session, "Could not install Ctrl+C handler");
        eprintln!("Could not listen for Ctrl+C: {error}");
        process::exit(1);
    }

    let mut last_space_check = Instant::now();
    loop {
        match shutdown_receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(()) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Err(error) = capture.check_health() {
                    let reason = error.to_string();
                    let _ = capture.stop();
                    fail_and_exit(&mut session, &reason);
                }

                if last_space_check.elapsed() >= meeting::RECORDING_SPACE_CHECK_INTERVAL {
                    if let Err(error) = meeting::ensure_recording_space(&session) {
                        let mut reason = format!(
                            "Recording stopped before the disk reserve was exhausted: {error}"
                        );
                        if let Err(stop_error) = capture.stop() {
                            reason.push_str(&format!(
                                " The audio backend also failed to finalize: {stop_error}"
                            ));
                        }
                        fail_and_exit(&mut session, &reason);
                    }
                    last_space_check = Instant::now();
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                fail_and_exit(&mut session, "Shutdown signal channel closed unexpectedly");
            }
        }
    }

    if let Err(error) = capture.stop() {
        fail_and_exit(
            &mut session,
            &format!("Could not finalize audio files: {error}"),
        );
    }

    if mode == CaptureMode::Both {
        if let Err(error) = audio::mix_tracks(session.folder()) {
            fail_and_exit(
                &mut session,
                &format!(
                    "Audio tracks were preserved, but the mixed track could not be created: {error}"
                ),
            );
        }
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
    fail_and_exit_with_code(session, reason, 1)
}

fn fail_and_exit_with_code(
    session: &mut meeting::MeetingSession,
    reason: &str,
    exit_code: i32,
) -> ! {
    let _ = meeting::fail(session, reason);
    eprintln!("{reason}");
    process::exit(exit_code);
}

fn parse_start_arguments(arguments: &[String]) -> Result<(String, CaptureMode), String> {
    let mut title = None;
    let mut mode = CaptureMode::System;
    let mut mode_flag = None;

    for argument in arguments {
        let requested_mode = match argument.as_str() {
            "--mic" => Some(CaptureMode::Both),
            "--mic-only" => Some(CaptureMode::Microphone),
            value if value.starts_with('-') => {
                return Err(format!("unknown option '{value}'"));
            }
            value => {
                if title.is_some() {
                    return Err("only one title may be provided".to_string());
                }
                title = Some(value.to_string());
                None
            }
        };

        if let Some(requested_mode) = requested_mode {
            if mode_flag.is_some() {
                return Err("--mic and --mic-only cannot be combined or repeated".to_string());
            }
            mode = requested_mode;
            mode_flag = Some(argument.as_str());
        }
    }

    Ok((title.unwrap_or_else(|| "untitled".to_string()), mode))
}

fn parse_mode_arguments(arguments: &[String]) -> Result<CaptureMode, String> {
    let mut mode = CaptureMode::System;

    for argument in arguments {
        mode = match argument.as_str() {
            "--mic" => CaptureMode::Both,
            "--mic-only" => CaptureMode::Microphone,
            value if value.starts_with('-') => return Err(format!("unknown option '{value}'")),
            _ => return Err("a permission mode flag is required instead of a title".to_string()),
        };
    }

    if arguments.len() > 1 {
        return Err("--mic and --mic-only cannot be combined or repeated".to_string());
    }

    Ok(mode)
}

fn print_usage() {
    println!("Usage:");
    println!("  rusteze start [title] [--mic|--mic-only]");
    println!(
        "    (default: system audio only; --mic adds microphone; --mic-only uses microphone only)"
    );
    println!("  rusteze request-permissions [--mic|--mic-only]");
    println!("  rusteze create-meeting [title]");
    println!("  rusteze mix <session-path>");
    println!("  rusteze transcribe <session-path>");
    println!("  rusteze prepare-audio <input-path> [output-path]");
    println!("Example: rusteze start \"Project sync\" --mic");
}

#[cfg(test)]
mod tests {
    use super::parse_start_arguments;
    use crate::native_helper::CaptureMode;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn defaults_to_system_audio_only() {
        assert_eq!(
            parse_start_arguments(&arguments(&[])).unwrap(),
            ("untitled".to_string(), CaptureMode::System)
        );
    }

    #[test]
    fn maps_mic_flags_to_the_expected_modes() {
        assert_eq!(
            parse_start_arguments(&arguments(&["Workshop", "--mic"])).unwrap(),
            ("Workshop".to_string(), CaptureMode::Both)
        );
        assert_eq!(
            parse_start_arguments(&arguments(&["--mic-only"])).unwrap(),
            ("untitled".to_string(), CaptureMode::Microphone)
        );
    }

    #[test]
    fn rejects_conflicting_or_unknown_start_flags() {
        assert!(parse_start_arguments(&arguments(&["--mic", "--mic-only"])).is_err());
        assert!(parse_start_arguments(&arguments(&["--unknown"])).is_err());
        assert!(parse_start_arguments(&arguments(&["one", "two"])).is_err());
    }

    #[test]
    fn maps_permission_request_flags() {
        assert_eq!(
            super::parse_mode_arguments(&arguments(&[])).unwrap(),
            CaptureMode::System
        );
        assert_eq!(
            super::parse_mode_arguments(&arguments(&["--mic"])).unwrap(),
            CaptureMode::Both
        );
        assert_eq!(
            super::parse_mode_arguments(&arguments(&["--mic-only"])).unwrap(),
            CaptureMode::Microphone
        );
        assert!(super::parse_mode_arguments(&arguments(&["--mic", "--mic-only"])).is_err());
    }
}
