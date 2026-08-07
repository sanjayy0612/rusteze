use std::{
    collections::HashMap,
    env,
    ffi::OsString,
    fmt, io,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureMode {
    System,
    Microphone,
    Both,
}

impl CaptureMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Microphone => "microphone",
            Self::Both => "both",
        }
    }

    pub fn requires_microphone(self) -> bool {
        matches!(self, Self::Microphone | Self::Both)
    }

    pub fn requires_screen_recording(self) -> bool {
        matches!(self, Self::System | Self::Both)
    }

    pub fn output_files(self) -> Vec<&'static str> {
        match self {
            Self::System => vec!["system.caf"],
            Self::Microphone => vec!["mic.caf"],
            Self::Both => vec!["system.caf", "mic.caf"],
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PermissionStatus {
    pub microphone: String,
    pub screen_recording: String,
}

impl PermissionStatus {
    pub fn is_ready_to_record(&self, mode: CaptureMode) -> bool {
        (!mode.requires_microphone() || self.microphone == "granted")
            && (!mode.requires_screen_recording() || self.screen_recording == "granted")
    }

    pub fn guidance(&self, mode: CaptureMode) -> String {
        let mut missing = Vec::new();

        if mode.requires_microphone() && self.microphone != "granted" {
            missing.push("Microphone");
        }
        if mode.requires_screen_recording() && self.screen_recording != "granted" {
            missing.push("Screen Recording");
        }

        format!(
            "{} permission is required. Open System Settings > Privacy & Security and grant it to Rusteze.",
            missing.join(" and ")
        )
    }
}

#[derive(Debug)]
pub enum HelperError {
    Missing(PathBuf),
    Launch(io::Error),
    Failed { status: Option<i32>, stderr: String },
    InvalidResponse(String),
}

pub struct CaptureProcess {
    child: Child,
    stdin: ChildStdin,
}

impl fmt::Display for HelperError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(path) => write!(
                formatter,
                "Native macOS helper was not built at {}. Run macos-helper/build.sh first.",
                path.display()
            ),
            Self::Launch(error) => write!(formatter, "Could not launch macOS helper: {error}"),
            Self::Failed { status, stderr } => write!(
                formatter,
                "macOS helper exited with status {}: {}",
                status
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
                stderr.trim()
            ),
            Self::InvalidResponse(message) => {
                write!(
                    formatter,
                    "macOS helper returned an invalid response: {message}"
                )
            }
        }
    }
}

impl HelperError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Failed {
                status: Some(status),
                ..
            } if *status == 64 || *status == 77 => *status,
            _ => 1,
        }
    }
}

/// Starts the native macOS helper and reads its permission preflight result.
pub fn check_permissions(mode: CaptureMode) -> Result<PermissionStatus, HelperError> {
    let helper_path = helper_path();
    if !helper_path.is_file() {
        return Err(HelperError::Missing(helper_path));
    }

    let output = Command::new(helper_path)
        .arg("check-permissions")
        .arg(mode.as_str())
        .stdin(Stdio::null())
        .output()
        .map_err(HelperError::Launch)?;

    if !output.status.success() {
        return Err(HelperError::Failed {
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    parse_permission_status(&String::from_utf8_lossy(&output.stdout))
}

/// Requests only the permissions needed by the selected capture mode.
pub fn request_permissions(mode: CaptureMode) -> Result<PermissionStatus, HelperError> {
    let helper_path = helper_path();
    if !helper_path.is_file() {
        return Err(HelperError::Missing(helper_path));
    }

    let output = Command::new(helper_path)
        .arg("request-permissions")
        .arg(mode.as_str())
        .stdin(Stdio::null())
        .output()
        .map_err(HelperError::Launch)?;

    if !output.status.success() {
        return Err(HelperError::Failed {
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    parse_permission_status(&String::from_utf8_lossy(&output.stdout))
}

/// Starts the selected native capture streams and waits for the helper's ready signal.
pub fn start_capture(
    session_folder: &std::path::Path,
    mode: CaptureMode,
) -> Result<CaptureProcess, HelperError> {
    let helper_path = helper_path();
    if !helper_path.is_file() {
        return Err(HelperError::Missing(helper_path));
    }

    let mut child = Command::new(helper_path)
        .args(record_arguments(session_folder, mode))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(HelperError::Launch)?;
    let stdin = child.stdin.take().ok_or_else(|| {
        HelperError::InvalidResponse("macOS helper did not provide a control channel".to_string())
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        HelperError::InvalidResponse("macOS helper did not provide a status channel".to_string())
    })?;
    let mut status_line = String::new();
    BufReader::new(stdout)
        .read_line(&mut status_line)
        .map_err(HelperError::Launch)?;

    if status_line.trim() != "RESULT recording-started" {
        let status = child.wait().ok().and_then(|result| result.code());
        return Err(HelperError::Failed {
            status,
            stderr: status_line,
        });
    }

    Ok(CaptureProcess { child, stdin })
}

fn record_arguments(session_folder: &Path, mode: CaptureMode) -> Vec<OsString> {
    vec![
        OsString::from("record"),
        session_folder.as_os_str().to_owned(),
        OsString::from(mode.as_str()),
    ]
}

impl CaptureProcess {
    /// Detects an unexpected helper exit, such as a capture device failure.
    pub fn check_health(&mut self) -> Result<(), HelperError> {
        match self.child.try_wait().map_err(HelperError::Launch)? {
            Some(status) => Err(HelperError::Failed {
                status: status.code(),
                stderr: "Capture helper exited unexpectedly; audio already written was preserved."
                    .to_string(),
            }),
            None => Ok(()),
        }
    }

    /// Asks the helper to finalize streams instead of force-killing it.
    pub fn stop(mut self) -> Result<(), HelperError> {
        self.stdin
            .write_all(b"stop\n")
            .map_err(HelperError::Launch)?;
        self.stdin.flush().map_err(HelperError::Launch)?;
        let status = self.child.wait().map_err(HelperError::Launch)?;
        if status.success() {
            Ok(())
        } else {
            Err(HelperError::Failed {
                status: status.code(),
                stderr: "Capture helper stopped with an error.".to_string(),
            })
        }
    }
}

fn helper_path() -> PathBuf {
    env::var_os("RUSTEZE_CAPTURE_HELPER")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("macos-helper")
                .join(".build")
                .join("debug")
                .join("rusteze-capture-helper")
        })
}

fn parse_permission_status(output: &str) -> Result<PermissionStatus, HelperError> {
    let values: HashMap<_, _> = output
        .lines()
        .filter_map(|line| line.split_once(' '))
        .collect();

    if values.get("RESULT") != Some(&"permission-status") {
        return Err(HelperError::InvalidResponse(output.to_string()));
    }

    let microphone = required_value(&values, "MICROPHONE", output)?;
    let screen_recording = required_value(&values, "SCREEN_RECORDING", output)?;

    Ok(PermissionStatus {
        microphone: microphone.to_string(),
        screen_recording: screen_recording.to_string(),
    })
}

fn required_value<'a>(
    values: &'a HashMap<&str, &str>,
    key: &str,
    output: &str,
) -> Result<&'a str, HelperError> {
    values
        .get(key)
        .copied()
        .ok_or_else(|| HelperError::InvalidResponse(output.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{parse_permission_status, record_arguments, CaptureMode, PermissionStatus};
    use std::path::Path;

    #[test]
    fn parses_a_permission_response_from_the_helper() {
        let result = parse_permission_status(
            "RESULT permission-status\nMICROPHONE granted\nSCREEN_RECORDING missing\n",
        )
        .unwrap();

        assert_eq!(result.microphone, "granted");
        assert_eq!(result.screen_recording, "missing");
        assert!(!result.is_ready_to_record(CaptureMode::Both));
        assert!(result.is_ready_to_record(CaptureMode::Microphone));
        assert!(result
            .guidance(CaptureMode::Both)
            .contains("Screen Recording"));
    }

    #[test]
    fn rejects_an_incomplete_helper_response() {
        assert!(parse_permission_status("RESULT permission-status\n").is_err());
    }

    #[test]
    fn maps_capture_modes_to_helper_values_and_track_files() {
        assert_eq!(CaptureMode::System.as_str(), "system");
        assert_eq!(CaptureMode::Microphone.as_str(), "microphone");
        assert_eq!(CaptureMode::Both.as_str(), "both");
        assert_eq!(CaptureMode::System.output_files(), vec!["system.caf"]);
        assert_eq!(CaptureMode::Microphone.output_files(), vec!["mic.caf"]);
        assert_eq!(
            CaptureMode::Both.output_files(),
            vec!["system.caf", "mic.caf"]
        );
    }

    #[test]
    fn passes_the_selected_mode_to_the_helper_record_command() {
        let arguments = record_arguments(Path::new("/tmp/session"), CaptureMode::Both);
        assert_eq!(arguments[0], "record");
        assert_eq!(arguments[1], "/tmp/session");
        assert_eq!(arguments[2], "both");
    }

    #[test]
    fn permission_requirements_are_mode_specific() {
        let permissions = PermissionStatus {
            microphone: "missing".to_string(),
            screen_recording: "granted".to_string(),
        };

        assert!(permissions.is_ready_to_record(CaptureMode::System));
        assert!(!permissions.is_ready_to_record(CaptureMode::Microphone));
        assert!(!permissions.is_ready_to_record(CaptureMode::Both));
        assert!(permissions
            .guidance(CaptureMode::Microphone)
            .contains("Microphone"));
        assert!(!permissions
            .guidance(CaptureMode::System)
            .contains("Microphone"));
    }
}
