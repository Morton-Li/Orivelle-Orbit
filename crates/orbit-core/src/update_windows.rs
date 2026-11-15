//! Hand a verified Windows update to the NSIS installer. The installer performs
//! the in-place update and restart; failed handoff leaves the running app intact.
use super::{CURRENT_VERSION, VerifiedArtifact, version_parts};
use crate::{
    error::{Result, fail},
    identity::sha256,
    store::private_directory,
    windows_fs,
};
use std::{
    ffi::OsString,
    fs::{self, File},
    io::Read,
    os::windows::{ffi::OsStringExt, fs::MetadataExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use windows_sys::Win32::{
    Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT, System::Threading::CREATE_NO_WINDOW,
};

const PREFIX: &str = "windows-install-";
struct StagedInstaller {
    // Close the file and directory pins before TempDir attempts removal.
    _file: File,
    _pins: Vec<File>,
    directory: tempfile::TempDir,
    installation: PathBuf,
}

fn executable(bytes: &[u8]) -> bool {
    let Some(offset) = bytes.get(0x3c..0x40) else {
        return false;
    };
    let offset = u32::from_le_bytes(offset.try_into().unwrap()) as usize;
    bytes.starts_with(b"MZ") && bytes.get(offset..offset.saturating_add(4)) == Some(b"PE\0\0")
}

fn stage(root: &Path, current: &Path, artifact: &VerifiedArtifact) -> Result<StagedInstaller> {
    let candidate = artifact.candidate();
    if candidate.target != "windows-x86_64"
        || version_parts(&candidate.version)? <= version_parts(CURRENT_VERSION)?
        || !executable(artifact.bytes())
        || current
            .file_name()
            .is_none_or(|name| name != "orivelle-orbit.exe")
    {
        return Err(fail("invalid_update_artifact"));
    }
    let installation = current.parent().ok_or_else(|| fail("invalid_path"))?;
    let mut pins = windows_fs::lock_directory(installation)?;
    let updates = root.join("updates");
    private_directory(&updates)?;
    pins.extend(windows_fs::lock_directory(&updates)?);
    let directory = tempfile::Builder::new()
        .prefix(PREFIX)
        .tempdir_in(&updates)
        .map_err(|_| fail("update_install_failed"))?;
    private_directory(directory.path())?;
    let path = directory.path().join("setup.exe");
    crate::config::atomic_write(&path, artifact.bytes())?;
    pins.extend(windows_fs::lock_directory(directory.path())?);
    let mut file = windows_fs::lock_installed_file(&path)?;
    // Re-read through the handle that denies writes and deletion. Never execute
    // a path whose bytes could have changed between verification and launch.
    let mut bytes = Vec::new();
    (&mut file)
        .take(super::MAX_UPDATE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| fail("update_install_failed"))?;
    if sha256(&bytes) != sha256(artifact.bytes()) {
        return Err(fail("update_artifact_changed"));
    }
    // NSIS expects a normal drive path for /D, without Rust's verbatim prefix.
    use std::os::windows::ffi::OsStrExt;
    let wide = installation.as_os_str().encode_wide().collect::<Vec<_>>();
    let wide = wide.strip_prefix(&[92, 92, 63, 92]).unwrap_or(&wide);
    Ok(StagedInstaller {
        _file: file,
        _pins: pins,
        directory,
        installation: PathBuf::from(OsString::from_wide(wide)),
    })
}

/// Called while the GUI holds the maintenance lease and exclusive helper lock.
/// Launch the same passive/update/restart contract used by Tauri's NSIS updater.
pub fn launch(root: &Path, current: &Path, artifact: &VerifiedArtifact) -> Result<()> {
    let staged = stage(root, current, artifact)?;
    let mut destination = OsString::from("/D=");
    destination.push(&staged.installation);
    Command::new(staged.directory.path().join("setup.exe"))
        .args(["/P", "/UPDATE", "/R"])
        // NSIS requires /D last, with no quotes, even when its path has spaces.
        // CreateProcess runs this directly; no command shell interprets the path.
        .raw_arg(destination)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| fail("update_install_failed"))?;
    // Keep the executable until the installer and its self-extraction finish.
    // The following normal startup can remove it once Windows releases it.
    let _ = staged.directory.keep();
    Ok(())
}

/// Best-effort cleanup of our completed installer handoffs only. Windows refuses
/// removal while an installer is still mapped; a later startup retries it.
pub fn cleanup(root: &Path) {
    let Ok(entries) = fs::read_dir(root.join("updates")) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_str().is_some_and(|name| {
            name.strip_prefix(PREFIX).is_some_and(|suffix| {
                suffix.len() == 6 && suffix.bytes().all(|b| b.is_ascii_alphanumeric())
            })
        }) {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn artifact() -> VerifiedArtifact {
        let mut bytes = vec![0; 256];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&128u32.to_le_bytes());
        bytes[128..132].copy_from_slice(b"PE\0\0");
        VerifiedArtifact {
            candidate: crate::updates::Candidate {
                version: "9999999999.0.0".into(),
                target: "windows-x86_64".into(),
                url: "https://example.invalid/Orbit-setup.exe".into(),
                signature: "synthetic, not executed".into(),
                file_name: None,
                hash: None,
                verified: true,
            },
            bytes,
        }
    }
    #[test]
    fn staged_installer_is_private_and_locked_until_handoff() {
        let root = tempfile::tempdir().unwrap();
        let installation = root.path().join("installation with spaces");
        private_directory(&installation).unwrap();
        let current = installation.join("orivelle-orbit.exe");
        let staged = stage(root.path(), &current, &artifact()).unwrap();
        let path = staged.directory.path().join("setup.exe");
        windows_fs::identity(&staged._file, true).unwrap();
        assert!(fs::OpenOptions::new().write(true).open(&path).is_err());
        assert!(fs::rename(&path, path.with_extension("changed")).is_err());
        assert!(fs::rename(&installation, root.path().join("moved")).is_err());
        let directory = staged.directory.path().to_owned();
        drop(staged);
        assert!(!directory.exists());
    }
    #[test]
    fn rejects_other_targets_old_versions_and_nonexecutables_before_staging() {
        let root = tempfile::tempdir().unwrap();
        let current = root.path().join("orivelle-orbit.exe");
        for mutation in 0..4 {
            let mut artifact = artifact();
            match mutation {
                0 => artifact.candidate.target = "darwin-aarch64".into(),
                1 => artifact.candidate.version = CURRENT_VERSION.into(),
                2 => artifact.bytes = b"not an executable".to_vec(),
                _ => artifact.bytes[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes()),
            }
            assert!(stage(root.path(), &current, &artifact).is_err());
        }
        assert!(!root.path().join("updates").exists());
    }
    #[test]
    fn failed_process_creation_keeps_the_running_application_and_cleans_staging() {
        let root = tempfile::tempdir().unwrap();
        let current = root.path().join("orivelle-orbit.exe");
        // MZ/PE magic alone does not make this synthetic buffer executable.
        assert!(launch(root.path(), &current, &artifact()).is_err());
        assert_eq!(
            fs::read_dir(root.path().join("updates")).unwrap().count(),
            0
        );
    }
}
