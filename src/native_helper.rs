use std::{
    collections::HashMap,
    env, fmt, io,
    path::PathBuf,
    process::{Command, Stdio},
};

#[derive(Debug, PartialEq, Eq)]
pub struct PermissionStatus {
    pub microphone: String,
    pub screen_recording: String,
}

impl PermissionStatus {
    pub fn is_ready_to_record(&self) -> bool {
        self.microphone == "granted" && self.screen_recording == "granted"
    }

    pub fn guidance(&self) -> String {
        let mut missing = Vec::new();

        if self.microphone != "granted" {
            missing.push("Microphone");
        }
        if self.screen_recording != "granted" {
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

/// Starts the native macOS helper and reads its permission preflight result.
pub fn check_permissions() -> Result<PermissionStatus, HelperError> {
    let helper_path = helper_path();
    if !helper_path.is_file() {
        return Err(HelperError::Missing(helper_path));
    }

    let output = Command::new(helper_path)
        .arg("check-permissions")
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
    use super::parse_permission_status;

    #[test]
    fn parses_a_permission_response_from_the_helper() {
        let result = parse_permission_status(
            "RESULT permission-status\nMICROPHONE granted\nSCREEN_RECORDING missing\n",
        )
        .unwrap();

        assert_eq!(result.microphone, "granted");
        assert_eq!(result.screen_recording, "missing");
        assert!(!result.is_ready_to_record());
        assert!(result.guidance().contains("Screen Recording"));
    }

    #[test]
    fn rejects_an_incomplete_helper_response() {
        assert!(parse_permission_status("RESULT permission-status\n").is_err());
    }
}
