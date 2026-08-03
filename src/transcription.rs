use std::{error::Error, fmt, fs, io, path::Path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptSegment {
    pub start_milliseconds: u64,
    pub end_milliseconds: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub engine: String,
    pub model: Option<String>,
    pub text: String,
    pub segments: Vec<TranscriptSegment>,
}

#[derive(Debug)]
pub enum TranscriptionError {
    NoEngineConfigured,
    Engine(String),
    Storage(io::Error),
}

impl fmt::Display for TranscriptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoEngineConfigured => write!(
                formatter,
                "No local transcription engine is configured. Add an implementation of TranscriptionEngine first."
            ),
            Self::Engine(message) => write!(formatter, "Transcription engine failed: {message}"),
            Self::Storage(error) => write!(formatter, "Could not write transcript files: {error}"),
        }
    }
}

impl Error for TranscriptionError {}

/// The stable boundary a local Whisper, whisper.cpp, or other engine implements.
pub trait TranscriptionEngine {
    fn transcribe(&self, session_folder: &Path) -> Result<Transcript, TranscriptionError>;
}

/// Placeholder used until the user chooses a local model implementation.
pub struct UnconfiguredEngine;

impl TranscriptionEngine for UnconfiguredEngine {
    fn transcribe(&self, _session_folder: &Path) -> Result<Transcript, TranscriptionError> {
        Err(TranscriptionError::NoEngineConfigured)
    }
}

/// Persists the portable transcript files independently of the chosen engine.
pub fn write_transcript(
    session_folder: &Path,
    transcript: &Transcript,
) -> Result<(), TranscriptionError> {
    fs::write(session_folder.join("transcript.md"), markdown(transcript))
        .map_err(TranscriptionError::Storage)?;
    fs::write(session_folder.join("transcript.json"), json(transcript))
        .map_err(TranscriptionError::Storage)
}

fn markdown(transcript: &Transcript) -> String {
    let mut output = format!("# Transcript\n\nEngine: {}\n\n", transcript.engine);
    for segment in &transcript.segments {
        output.push_str(&format!(
            "[{}–{}] {}\n\n",
            timestamp(segment.start_milliseconds),
            timestamp(segment.end_milliseconds),
            segment.text
        ));
    }
    if transcript.segments.is_empty() {
        output.push_str(&transcript.text);
        output.push('\n');
    }
    output
}

fn json(transcript: &Transcript) -> String {
    let model = transcript
        .model
        .as_deref()
        .map(json_string)
        .unwrap_or_else(|| "null".to_string());
    let segments = transcript
        .segments
        .iter()
        .map(|segment| {
            format!(
                "    {{\"start_milliseconds\": {}, \"end_milliseconds\": {}, \"text\": {}}}",
                segment.start_milliseconds,
                segment.end_milliseconds,
                json_string(&segment.text)
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!(
        "{{\n  \"engine\": {},\n  \"model\": {},\n  \"text\": {},\n  \"segments\": [\n{}\n  ]\n}}\n",
        json_string(&transcript.engine), model, json_string(&transcript.text), segments
    )
}

fn timestamp(milliseconds: u64) -> String {
    format!(
        "{:02}:{:02}.{:03}",
        milliseconds / 60_000,
        (milliseconds / 1_000) % 60,
        milliseconds % 1_000
    )
}

fn json_string(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

#[cfg(test)]
mod tests {
    use super::{json, markdown, Transcript, TranscriptSegment};

    #[test]
    fn renders_timestamped_portable_transcript_formats() {
        let transcript = Transcript {
            engine: "test".to_string(),
            model: None,
            text: "Hello".to_string(),
            segments: vec![TranscriptSegment {
                start_milliseconds: 1_250,
                end_milliseconds: 3_000,
                text: "Hello".to_string(),
            }],
        };
        assert!(markdown(&transcript).contains("[00:01.250–00:03.000] Hello"));
        assert!(json(&transcript).contains("\"start_milliseconds\": 1250"));
    }
}
