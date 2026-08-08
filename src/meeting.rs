use std::{
    env, fs, io,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{native_helper::CaptureMode, storage};

pub const MINIMUM_FREE_BYTES: u64 = 256 * 1024 * 1024;
pub(crate) const RECORDING_SPACE_CHECK_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(5);
const MAX_RECOVERY_METADATA_BYTES: u64 = 1024 * 1024;
const SENSITIVE_SESSION_FILES: &[&str] = &[
    "session.json",
    "mic.caf",
    "system.caf",
    "mic.wav",
    "system.wav",
    "mixed.caf",
    "mixed.wav",
    "transcript.md",
    "transcript.json",
];
static TEMPORARY_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionState {
    Idle,
    Recording,
    Stopping,
    Completed,
    Failed,
}

impl SessionState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Recording => "recording",
            Self::Stopping => "stopping",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// A local meeting session and the metadata needed to recover its state.
pub struct MeetingSession {
    folder: PathBuf,
    id: String,
    title: String,
    state: SessionState,
    capture_mode: Option<CaptureMode>,
    started_at_unix_seconds: u64,
    ended_at_unix_seconds: Option<u64>,
    recovery_reason: Option<String>,
}

impl MeetingSession {
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    fn save(&self) -> io::Result<()> {
        write_atomically(&self.folder.join("session.json"), &self.json())
    }

    fn transition_to(&mut self, state: SessionState) -> io::Result<()> {
        self.state = state;
        self.save()
    }

    fn json(&self) -> String {
        let ended_at = self
            .ended_at_unix_seconds
            .map(|time| time.to_string())
            .unwrap_or_else(|| "null".to_string());
        let duration = self
            .ended_at_unix_seconds
            .map(|time| {
                time.saturating_sub(self.started_at_unix_seconds)
                    .to_string()
            })
            .unwrap_or_else(|| "null".to_string());
        let recovery_reason = self
            .recovery_reason
            .as_deref()
            .map(|reason| format!("\"{}\"", escape_json_string(reason)))
            .unwrap_or_else(|| "null".to_string());
        let capture_mode = self
            .capture_mode
            .map(|mode| format!("\"{}\"", mode.as_str()))
            .unwrap_or_else(|| "null".to_string());
        let enabled_files = self
            .capture_mode
            .map(CaptureMode::output_files)
            .unwrap_or_default();
        let microphone_track = track_json_value(&enabled_files, "mic.");
        let system_audio_track = track_json_value(&enabled_files, "system.");
        let mixed_audio_track = if self.folder.join("mixed.caf").is_file() {
            "\"mixed.caf\"".to_string()
        } else if self.folder.join("mixed.wav").is_file() {
            "\"mixed.wav\"".to_string()
        } else {
            "null".to_string()
        };

        format!(
            concat!(
                "{{\n",
                "  \"session_id\": \"{}\",\n",
                "  \"title\": \"{}\",\n",
                "  \"state\": \"{}\",\n",
                "  \"capture_mode\": {},\n",
                "  \"started_at_unix_seconds\": {},\n",
                "  \"ended_at_unix_seconds\": {},\n",
                "  \"duration_seconds\": {},\n",
                "  \"recovery_reason\": {},\n",
                "  \"microphone_track\": {},\n",
                "  \"system_audio_track\": {},\n",
                "  \"mixed_audio_track\": {}\n",
                "}}\n"
            ),
            escape_json_string(&self.id),
            escape_json_string(&self.title),
            self.state.as_str(),
            capture_mode,
            self.started_at_unix_seconds,
            ended_at,
            duration,
            recovery_reason,
            microphone_track,
            system_audio_track,
            mixed_audio_track,
        )
    }
}

fn track_json_value(files: &[&str], prefix: &str) -> String {
    files
        .iter()
        .copied()
        .find(|file| file.starts_with(prefix))
        .map(|file| format!("\"{file}\""))
        .unwrap_or_else(|| "null".to_string())
}

/// Creates an idle meeting folder for the early folder-structure command.
pub fn create(title: &str) -> io::Result<PathBuf> {
    let session = create_session(title, SessionState::Idle, None)?;
    Ok(session.folder)
}

/// Creates a session that is ready for the foreground recording lifecycle.
pub fn start(title: &str, capture_mode: CaptureMode) -> io::Result<MeetingSession> {
    create_session(title, SessionState::Recording, Some(capture_mode))
}

/// Finds sessions left active by a crash and records a recoverable failure.
pub fn recover_interrupted_sessions() -> io::Result<Vec<PathBuf>> {
    let meetings_directory = ensure_meetings_directory()?;
    let mut recovered = Vec::new();
    let entries = fs::read_dir(meetings_directory)?;

    for entry in entries {
        let entry = entry?;
        let entry_path = entry.path();
        match fs::symlink_metadata(&entry_path) {
            Ok(metadata) if metadata.is_dir() && !storage::is_link_or_reparse_point(&metadata) => {}
            Ok(_) => continue,
            Err(_) => continue,
        }
        storage::enforce_private_directory(&entry_path)?;
        harden_existing_session_files(&entry_path)?;

        let session_path = entry_path.join("session.json");
        match fs::symlink_metadata(&session_path) {
            Ok(metadata)
                if metadata.is_file()
                    && !storage::is_link_or_reparse_point(&metadata)
                    && metadata.len() <= MAX_RECOVERY_METADATA_BYTES => {}
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => continue,
        }
        let contents = match fs::read_to_string(&session_path) {
            Ok(contents) => contents,
            Err(_) => continue,
        };
        let now = unix_seconds_now()?;
        let Some(recovered_json) = recover_session_json(&contents, now) else {
            continue;
        };
        write_atomically(&session_path, &recovered_json)?;
        recovered.push(session_path);
    }

    Ok(recovered)
}

fn recover_session_json(contents: &str, ended_at: u64) -> Option<String> {
    let was_active = contents.contains("\"state\": \"recording\"")
        || contents.contains("\"state\": \"stopping\"");
    if !was_active {
        return None;
    }

    Some(
        contents
            .replace("\"state\": \"recording\"", "\"state\": \"failed\"")
            .replace("\"state\": \"stopping\"", "\"state\": \"failed\"")
            .replace(
                "\"ended_at_unix_seconds\": null",
                &format!("\"ended_at_unix_seconds\": {ended_at}"),
            )
            .replace(
                "\"recovery_reason\": null",
                "\"recovery_reason\": \"Rusteze was interrupted before recording could finish. Audio already written was preserved.\"",
            ),
    )
}

/// Refuses to start a new recording when there is too little disk space for a safe session.
pub fn ensure_recording_space(session: &MeetingSession) -> io::Result<()> {
    ensure_path_has_recording_space(session.folder())
}

pub(crate) fn ensure_path_has_recording_space(path: &Path) -> io::Result<()> {
    let available = available_disk_space(path)?;
    if available < MINIMUM_FREE_BYTES {
        return Err(io::Error::other(format!(
            "Only {available} bytes are free; Rusteze keeps a {MINIMUM_FREE_BYTES}-byte reserve while processing audio."
        )));
    }
    Ok(())
}

/// Records the safe foreground shutdown sequence after Ctrl+C.
pub fn complete(session: &mut MeetingSession) -> io::Result<()> {
    session.transition_to(SessionState::Stopping)?;
    session.ended_at_unix_seconds = Some(unix_seconds_now()?);
    session.transition_to(SessionState::Completed)
}

/// Leaves a recoverable explanation in metadata when normal recording cannot continue.
pub fn fail(session: &mut MeetingSession, reason: &str) -> io::Result<()> {
    session.ended_at_unix_seconds = Some(unix_seconds_now()?);
    session.recovery_reason = Some(reason.to_string());
    session.transition_to(SessionState::Failed)
}

fn create_session(
    title: &str,
    initial_state: SessionState,
    capture_mode: Option<CaptureMode>,
) -> io::Result<MeetingSession> {
    let started_at_unix_seconds = unix_seconds_now()?;
    let meeting_name = slugify(title);
    let meetings_directory = ensure_meetings_directory()?;

    let (id, folder) = create_unique_session_directory(
        &meetings_directory,
        started_at_unix_seconds,
        &meeting_name,
    )?;
    let session = MeetingSession {
        folder,
        id,
        title: title.to_string(),
        state: initial_state,
        capture_mode,
        started_at_unix_seconds,
        ended_at_unix_seconds: None,
        recovery_reason: None,
    };

    session.save()?;
    Ok(session)
}

fn write_atomically(path: &Path, contents: &str) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Metadata path has no parent directory.",
        )
    })?;
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Metadata path has no file name.",
        )
    })?;

    for _ in 0..128 {
        let counter = TEMPORARY_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary_path = parent.join(format!(
            ".{}.{}.{}.tmp",
            file_name.to_string_lossy(),
            std::process::id(),
            counter
        ));
        let mut temporary_file = match storage::create_private_file_new(&temporary_path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };

        let result = (|| {
            temporary_file.write_all(contents.as_bytes())?;
            temporary_file.sync_all()?;
            drop(temporary_file);
            storage::replace_file_atomically(&temporary_path, path)
        })();

        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        return result;
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Could not allocate a safe temporary metadata file.",
    ))
}

#[cfg(target_os = "macos")]
fn available_disk_space(path: &Path) -> io::Result<u64> {
    use std::{ffi::CString, mem::MaybeUninit, os::unix::ffi::OsStrExt};

    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Session path contains a null byte.",
        )
    })?;
    let mut stats = MaybeUninit::<libc::statvfs>::zeroed();
    let result = unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    let stats = unsafe { stats.assume_init() };
    Ok((stats.f_bavail as u64).saturating_mul(stats.f_frsize))
}

#[cfg(target_os = "windows")]
fn available_disk_space(path: &Path) -> io::Result<u64> {
    use std::{iter, os::windows::ffi::OsStrExt};
    use windows::{core::PCWSTR, Win32::Storage::FileSystem::GetDiskFreeSpaceExW};

    let path = path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let mut available = 0u64;
    unsafe { GetDiskFreeSpaceExW(PCWSTR(path.as_ptr()), Some(&mut available), None, None) }
        .map_err(|error| {
            io::Error::other(format!("Could not determine free disk space: {error}"))
        })?;
    Ok(available)
}

#[cfg(target_os = "linux")]
fn available_disk_space(path: &Path) -> io::Result<u64> {
    use std::{ffi::CString, mem::MaybeUninit, os::unix::ffi::OsStrExt};

    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Session path contains a null byte.",
        )
    })?;
    let mut stats = MaybeUninit::<libc::statvfs>::zeroed();
    let result = unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    let stats = unsafe { stats.assume_init() };
    Ok((stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn available_disk_space(_path: &Path) -> io::Result<u64> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Rusteze recording is currently supported only on macOS, Windows, and Linux.",
    ))
}

fn unix_seconds_now() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "System clock is before 1970."))
}

fn create_unique_session_directory(
    meetings_directory: &Path,
    created_at: u64,
    meeting_name: &str,
) -> io::Result<(String, PathBuf)> {
    let base = format!("{created_at}-{meeting_name}");
    for duplicate_number in 1u64.. {
        let id = if duplicate_number == 1 {
            base.clone()
        } else {
            format!("{base}-{duplicate_number}")
        };
        let folder = meetings_directory.join(&id);
        match storage::create_private_directory(&folder) {
            Ok(()) => return Ok((id, folder)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    unreachable!("the session suffix space is not finite")
}

fn ensure_meetings_directory() -> io::Result<PathBuf> {
    let meetings_directory = default_meetings_directory()?;
    let application_directory = meetings_directory.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Meetings directory has no application parent.",
        )
    })?;
    let documents_directory = application_directory.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Application directory has no Documents parent.",
        )
    })?;

    fs::create_dir_all(documents_directory)?;
    storage::ensure_private_directory(application_directory)?;
    storage::ensure_private_directory(&meetings_directory)?;
    Ok(meetings_directory)
}

fn harden_existing_session_files(session_directory: &Path) -> io::Result<()> {
    for file_name in SENSITIVE_SESSION_FILES {
        let path = session_directory.join(file_name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() && !storage::is_link_or_reparse_point(&metadata) => {
                storage::enforce_private_file(&path)?;
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn default_meetings_directory() -> io::Result<PathBuf> {
    let home_directory = env::var("HOME").map_err(|_| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Could not find your home directory.",
        )
    })?;

    Ok(PathBuf::from(home_directory)
        .join("Documents")
        .join("rusteze")
        .join("meetings"))
}

fn escape_json_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn slugify(title: &str) -> String {
    let mut slug = String::new();
    let mut previous_character_was_dash = false;

    for character in title.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            previous_character_was_dash = false;
        } else if !slug.is_empty() && !previous_character_was_dash {
            slug.push('-');
            previous_character_was_dash = true;
        }
    }

    let slug = slug.trim_end_matches('-');

    if slug.is_empty() {
        "untitled".to_string()
    } else {
        slug.to_string()
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::write_atomically;
    use super::{recover_session_json, slugify, MeetingSession, SessionState};
    use crate::native_helper::CaptureMode;
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::{fs, time::SystemTime};

    #[cfg(unix)]
    fn temporary_directory(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rusteze-meeting-test-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn makes_a_simple_folder_name_from_a_title() {
        assert_eq!(slugify("Rust Workshop: Week 1!"), "rust-workshop-week-1");
    }

    #[test]
    fn uses_untitled_when_a_title_has_no_letters_or_numbers() {
        assert_eq!(slugify("!!!"), "untitled");
    }

    #[test]
    fn writes_lifecycle_fields_as_json() {
        let session = MeetingSession {
            folder: PathBuf::from("/tmp/meeting"),
            id: "123-demo".to_string(),
            title: "A \"quoted\" meeting".to_string(),
            state: SessionState::Completed,
            capture_mode: Some(CaptureMode::Both),
            started_at_unix_seconds: 100,
            ended_at_unix_seconds: Some(125),
            recovery_reason: None,
        };

        let json = session.json();
        let output_files = CaptureMode::Both.output_files();
        assert!(json.contains("\"state\": \"completed\""));
        assert!(json.contains("\"capture_mode\": \"both\""));
        assert!(json.contains(&format!("\"microphone_track\": \"{}\"", output_files[1])));
        assert!(json.contains(&format!("\"system_audio_track\": \"{}\"", output_files[0])));
        assert!(json.contains("\"duration_seconds\": 25"));
        assert!(json.contains("A \\\"quoted\\\" meeting"));
    }

    #[test]
    fn metadata_lists_only_tracks_enabled_by_the_capture_mode() {
        let session = MeetingSession {
            folder: PathBuf::from("/tmp/meeting"),
            id: "123-system".to_string(),
            title: "System audio".to_string(),
            state: SessionState::Recording,
            capture_mode: Some(CaptureMode::System),
            started_at_unix_seconds: 100,
            ended_at_unix_seconds: None,
            recovery_reason: None,
        };

        let json = session.json();
        let system_file = CaptureMode::System.output_files()[0];
        assert!(json.contains("\"capture_mode\": \"system\""));
        assert!(json.contains("\"microphone_track\": null"));
        assert!(json.contains(&format!("\"system_audio_track\": \"{}\"", system_file)));
        assert!(!json.contains(CaptureMode::Microphone.output_files()[0]));
    }

    #[test]
    fn recovers_an_interrupted_recording_without_touching_completed_sessions() {
        let active = "{\n  \"state\": \"recording\",\n  \"ended_at_unix_seconds\": null,\n  \"recovery_reason\": null\n}";
        let recovered = recover_session_json(active, 42).unwrap();
        assert!(recovered.contains("\"state\": \"failed\""));
        assert!(recovered.contains("\"ended_at_unix_seconds\": 42"));
        assert!(recovered.contains("Audio already written was preserved"));
        assert!(recover_session_json("{\"state\": \"completed\"}", 42).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_metadata_write_does_not_follow_a_predictable_temporary_symlink() {
        use std::os::unix::fs::symlink;

        let directory = temporary_directory("symlink");
        let metadata_path = directory.join("session.json");
        let temporary_path =
            metadata_path.with_extension(format!("json.{}.tmp", std::process::id()));
        let target = directory.join("target.txt");
        fs::write(&target, "do not replace").unwrap();
        symlink(&target, &temporary_path).unwrap();

        write_atomically(&metadata_path, "safe metadata").unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "do not replace");
        assert_eq!(fs::read_to_string(&metadata_path).unwrap(), "safe metadata");
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_metadata_write_enforces_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = temporary_directory("permissions");
        let metadata_path = directory.join("session.json");
        let temporary_path =
            metadata_path.with_extension(format!("json.{}.tmp", std::process::id()));
        fs::write(&temporary_path, "attacker-controlled temporary file").unwrap();
        fs::set_permissions(&temporary_path, fs::Permissions::from_mode(0o666)).unwrap();

        write_atomically(&metadata_path, "private metadata").unwrap();

        let mode = fs::metadata(&metadata_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        fs::remove_dir_all(directory).unwrap();
    }
}
