use std::path::{Path, PathBuf};

/// Convert a recording to mono 16 kHz 16-bit PCM WAV, a broadly supported
/// format for speech transcription and audio upload tools.
pub fn prepare(input: &Path, output: Option<&Path>) -> Result<PathBuf, String> {
    if !input.is_file() {
        return Err(format!(
            "Input audio file does not exist: {}",
            input.display()
        ));
    }

    let output = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_output_path(input));
    if output.exists() {
        return Err(format!(
            "Refusing to overwrite existing output file: {}",
            output.display()
        ));
    }
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err(format!(
                "Output directory does not exist: {}",
                parent.display()
            ));
        }
    }

    #[cfg(target_os = "macos")]
    {
        let result = std::process::Command::new("afconvert")
            .args(["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"])
            .arg(input)
            .arg(&output)
            .output()
            .map_err(|error| format!("Could not run afconvert: {error}"))?;

        if !result.status.success() {
            let details = String::from_utf8_lossy(&result.stderr).trim().to_string();
            let _ = std::fs::remove_file(&output);
            return Err(if details.is_empty() {
                format!("afconvert failed with status {}", result.status)
            } else {
                format!("afconvert failed: {details}")
            });
        }

        return Ok(output);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = output;
        Err("prepare-audio currently requires macOS's afconvert tool.".to_string())
    }
}

fn default_output_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("audio");
    input.with_file_name(format!("{stem}-16k-mono.wav"))
}

#[cfg(test)]
mod tests {
    use super::default_output_path;
    use std::path::Path;

    #[test]
    fn defaults_to_a_speech_optimized_wav_next_to_input() {
        assert_eq!(
            default_output_path(Path::new("/tmp/system.caf")),
            Path::new("/tmp/system-16k-mono.wav")
        );
    }
}
