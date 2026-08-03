use std::{
    env, fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

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
    started_at_unix_seconds: u64,
    ended_at_unix_seconds: Option<u64>,
    recovery_reason: Option<String>,
}

impl MeetingSession {
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    fn save(&self) -> io::Result<()> {
        fs::write(self.folder.join("session.json"), self.json())
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

        format!(
            concat!(
                "{{\n",
                "  \"session_id\": \"{}\",\n",
                "  \"title\": \"{}\",\n",
                "  \"state\": \"{}\",\n",
                "  \"started_at_unix_seconds\": {},\n",
                "  \"ended_at_unix_seconds\": {},\n",
                "  \"duration_seconds\": {},\n",
                "  \"recovery_reason\": {},\n",
                "  \"microphone_track\": null,\n",
                "  \"system_audio_track\": null\n",
                "}}\n"
            ),
            escape_json_string(&self.id),
            escape_json_string(&self.title),
            self.state.as_str(),
            self.started_at_unix_seconds,
            ended_at,
            duration,
            recovery_reason,
        )
    }
}

/// Creates an idle meeting folder for the early folder-structure command.
pub fn create(title: &str) -> io::Result<PathBuf> {
    let session = create_session(title, SessionState::Idle)?;
    Ok(session.folder)
}

/// Creates a session that is ready for the foreground recording lifecycle.
pub fn start(title: &str) -> io::Result<MeetingSession> {
    create_session(title, SessionState::Recording)
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

fn create_session(title: &str, initial_state: SessionState) -> io::Result<MeetingSession> {
    let started_at_unix_seconds = unix_seconds_now()?;
    let meeting_name = slugify(title);
    let meetings_directory = default_meetings_directory()?;

    fs::create_dir_all(&meetings_directory)?;

    let id = unique_session_id(&meetings_directory, started_at_unix_seconds, &meeting_name);
    let session = MeetingSession {
        folder: meetings_directory.join(&id),
        id,
        title: title.to_string(),
        state: initial_state,
        started_at_unix_seconds,
        ended_at_unix_seconds: None,
        recovery_reason: None,
    };

    fs::create_dir(&session.folder)?;
    session.save()?;
    Ok(session)
}

fn unix_seconds_now() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "System clock is before 1970."))
}

fn unique_session_id(meetings_directory: &Path, created_at: u64, meeting_name: &str) -> String {
    let base = format!("{created_at}-{meeting_name}");
    let mut id = base.clone();
    let mut duplicate_number = 2;

    while meetings_directory.join(&id).exists() {
        id = format!("{base}-{duplicate_number}");
        duplicate_number += 1;
    }

    id
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
    use super::{slugify, MeetingSession, SessionState};
    use std::path::PathBuf;

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
            started_at_unix_seconds: 100,
            ended_at_unix_seconds: Some(125),
            recovery_reason: None,
        };

        let json = session.json();
        assert!(json.contains("\"state\": \"completed\""));
        assert!(json.contains("\"duration_seconds\": 25"));
        assert!(json.contains("A \\\"quoted\\\" meeting"));
    }
}
