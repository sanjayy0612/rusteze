use super::{CaptureMode, HelperError, PermissionStatus};
use std::path::Path;

pub struct CaptureProcess;

pub fn check_permissions(_mode: CaptureMode) -> Result<PermissionStatus, HelperError> {
    Err(HelperError::Backend(
        "Rusteze capture is currently supported only on macOS and Windows.".to_string(),
    ))
}

pub fn request_permissions(_mode: CaptureMode) -> Result<PermissionStatus, HelperError> {
    Err(HelperError::Backend(
        "Rusteze capture is currently supported only on macOS and Windows.".to_string(),
    ))
}

pub fn start_capture(
    _session_folder: &Path,
    _mode: CaptureMode,
) -> Result<CaptureProcess, HelperError> {
    Err(HelperError::Backend(
        "Rusteze capture is currently supported only on macOS and Windows.".to_string(),
    ))
}

impl CaptureProcess {
    pub fn check_health(&mut self) -> Result<(), HelperError> {
        Ok(())
    }

    pub fn stop(self) -> Result<(), HelperError> {
        Ok(())
    }
}
