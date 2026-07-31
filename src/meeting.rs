use std::{
    env, fs, io,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

/// Creates one folder for a meeting and writes its initial metadata.
pub fn create(title: &str) -> io::Result<PathBuf> {
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the system clock should be after 1970")
        .as_secs();
    let meeting_name = slugify(title);
    let meetings_directory = default_meetings_directory()?;

    fs::create_dir_all(&meetings_directory)?;

    let mut folder = meetings_directory.join(format!("{created_at}-{meeting_name}"));
    let mut duplicate_number = 2;

    while folder.exists() {
        folder = meetings_directory.join(format!("{created_at}-{meeting_name}-{duplicate_number}"));
        duplicate_number += 1;
    }

    fs::create_dir(&folder)?;
    fs::write(folder.join("metadata.json"), metadata_json(title, created_at))?;

    Ok(folder)
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

fn metadata_json(title: &str, created_at: u64) -> String {
    format!(
        concat!(
            "{{\n",
            "  \"title\": \"{}\",\n",
            "  \"created_at_unix_seconds\": {},\n",
            "  \"duration_seconds\": null,\n",
            "  \"recording\": null,\n",
            "  \"transcript\": null\n",
            "}}\n"
        ),
        escape_json_string(title),
        created_at
    )
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
    use super::slugify;

    #[test]
    fn makes_a_simple_folder_name_from_a_title() {
        assert_eq!(slugify("Rust Workshop: Week 1!"), "rust-workshop-week-1");
    }

    #[test]
    fn uses_untitled_when_a_title_has_no_letters_or_numbers() {
        assert_eq!(slugify("!!!"), "untitled");
    }
}
