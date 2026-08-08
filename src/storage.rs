use std::{
    fs::{self, DirBuilder, File, Metadata, OpenOptions},
    io,
    path::Path,
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

/// Creates one application-owned directory and enforces owner-only access.
pub(crate) fn create_private_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    #[cfg(not(unix))]
    let builder = DirBuilder::new();
    builder.create(path)?;
    enforce_private_directory(path)
}

/// Ensures an application-owned directory is real (not a link) and private.
pub(crate) fn ensure_private_directory(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_dir() || is_link_or_reparse_point(&metadata) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "Refusing to use {} because it is not a real directory.",
                        path.display()
                    ),
                ));
            }
            enforce_private_directory(path)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => create_private_directory(path),
        Err(error) => Err(error),
    }
}

/// Opens a brand-new owner-only file without following an existing symlink.
pub(crate) fn create_private_file_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);

    let file = options.open(path)?;
    if let Err(error) = enforce_private_file(path) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(file)
}

pub(crate) fn enforce_private_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    {
        apply_private_windows_dacl(path, true)?;
    }
    Ok(())
}

pub(crate) fn enforce_private_file(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(windows)]
    {
        apply_private_windows_dacl(path, false)?;
    }
    Ok(())
}

pub(crate) fn is_link_or_reparse_point(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }

    #[cfg(not(windows))]
    false
}

#[cfg(not(windows))]
pub(crate) fn replace_file_atomically(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
pub(crate) fn replace_file_atomically(source: &Path, destination: &Path) -> io::Result<()> {
    use std::{iter, os::windows::ffi::OsStrExt};
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        },
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();

    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| io::Error::other(format!("Could not replace metadata safely: {error}")))
}

#[cfg(windows)]
fn apply_private_windows_dacl(path: &Path, directory: bool) -> io::Result<()> {
    use std::{iter, os::windows::ffi::OsStrExt};
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::{
                Authorization::{
                    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
                },
                SetFileSecurityW, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
                PSECURITY_DESCRIPTOR,
            },
        },
    };

    // Owner Rights, Local System, and local Administrators receive full control.
    // Directories also pass the protected DACL to child files and directories.
    let sddl = if directory {
        "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"
    } else {
        "D:P(A;;FA;;;OW)(A;;FA;;;SY)(A;;FA;;;BA)"
    };
    let sddl = sddl.encode_utf16().chain(iter::once(0)).collect::<Vec<_>>();
    let path = path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();

    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(|error| io::Error::other(format!("Could not build a private Windows ACL: {error}")))?;

    let result = unsafe {
        SetFileSecurityW(
            PCWSTR(path.as_ptr()),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
        .ok()
    }
    .map_err(|error| io::Error::other(format!("Could not apply a private Windows ACL: {error}")));

    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    result
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::{create_private_directory, ensure_private_directory};
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temporary_directory(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rusteze-storage-test-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn private_directories_are_owner_only_and_existing_modes_are_hardened() {
        let directory = temporary_directory("mode");
        create_private_directory(&directory).unwrap();
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );

        fs::set_permissions(&directory, fs::Permissions::from_mode(0o777)).unwrap();
        ensure_private_directory(&directory).unwrap();
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn private_directory_validation_rejects_symlinks() {
        use std::os::unix::fs::symlink;

        let target = temporary_directory("target");
        let link = temporary_directory("link");
        fs::create_dir(&target).unwrap();
        symlink(&target, &link).unwrap();

        assert!(ensure_private_directory(&link).is_err());
        fs::remove_file(link).unwrap();
        fs::remove_dir(target).unwrap();
    }
}
