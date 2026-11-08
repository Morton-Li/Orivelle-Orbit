use crate::{
    config::atomic_write,
    error::{Result, fail},
    identity::sha256,
    store::{Store, private_directory},
    updates::exclusive_helper_lock,
};
use std::{fs, path::PathBuf};
pub const MAX_HELPER_BYTES: u64 = 64 * 1024 * 1024;
pub fn binary_name() -> &'static str {
    if cfg!(windows) {
        "orbit-credential-helper.exe"
    } else {
        "orbit-credential-helper"
    }
}
fn validate_architecture(bytes: &[u8]) -> Result<()> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        if bytes.len() < 32
            || bytes[..4] != [0xcf, 0xfa, 0xed, 0xfe]
            || u32::from_le_bytes(bytes[4..8].try_into().unwrap()) != 0x0100000c
            || u32::from_le_bytes(bytes[12..16].try_into().unwrap()) != 2
        {
            return Err(fail("helper_artifact_invalid"));
        }
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        if bytes.len() < 64 || bytes.get(..2) != Some(b"MZ") {
            return Err(fail("helper_artifact_invalid"));
        }
        let offset = u32::from_le_bytes(bytes[60..64].try_into().unwrap()) as usize;
        if bytes.get(offset..offset + 4) != Some(b"PE\0\0")
            || bytes.get(offset + 4..offset + 6) != Some(&[0x64, 0x86])
        {
            return Err(fail("helper_artifact_invalid"));
        }
    } else {
        return Err(fail("platform_unsupported"));
    }
    Ok(())
}
/// `bytes` and its digest are compiled into the GUI, never supplied by IPC or a
/// mutable download path. The GUI's reviewed signing chain covers both values.
pub fn deploy_embedded(
    store: &Store,
    bytes: &[u8],
    expected_sha256: &str,
    version: &str,
) -> Result<PathBuf> {
    if bytes.len() as u64 > MAX_HELPER_BYTES
        || sha256(bytes) != expected_sha256
        || version != env!("CARGO_PKG_VERSION")
    {
        return Err(fail("helper_artifact_invalid"));
    }
    validate_architecture(bytes)?;
    let _lock = exclusive_helper_lock(&store.root)?;
    let bin = store.root.join("bin");
    private_directory(&bin)?;
    let path = bin.join(binary_name());
    if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) {
        return Err(fail("helper_artifact_invalid"));
    }
    let matches = fs::metadata(&path).is_ok_and(|m| m.len() == bytes.len() as u64)
        && crate::images::bounded_read(&path, MAX_HELPER_BYTES)
            .is_ok_and(|current| sha256(&current) == expected_sha256);
    if !matches {
        atomic_write(&path, bytes)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .map_err(|_| fail("helper_deployment_failed"))?;
    }
    if sha256(&crate::images::bounded_read(&path, MAX_HELPER_BYTES)?) != expected_sha256 {
        return Err(fail("helper_deployment_failed"));
    }
    fs::File::open(&path)
        .and_then(|f| f.sync_all())
        .map_err(|_| fail("helper_deployment_failed"))?;
    store.set_meta("helper_deployed_version", version)?;
    store.set_meta("helper_deployed_sha256", expected_sha256)?;
    store.set_meta("helper_contract_version", "1")?;
    Ok(path)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_helper_hash_architecture_and_active_lock_gate_deployment() {
        if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut bytes = vec![0u8; 32];
        bytes[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        bytes[4..8].copy_from_slice(&0x0100000cu32.to_le_bytes());
        bytes[12..16].copy_from_slice(&2u32.to_le_bytes());
        assert!(
            deploy_embedded(&store, &bytes, &"0".repeat(64), env!("CARGO_PKG_VERSION")).is_err()
        );
        let path =
            deploy_embedded(&store, &bytes, &sha256(&bytes), env!("CARGO_PKG_VERSION")).unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let lock = exclusive_helper_lock(&store.root).unwrap();
        assert_eq!(
            deploy_embedded(&store, &bytes, &sha256(&bytes), env!("CARGO_PKG_VERSION"))
                .unwrap_err()
                .code,
            "update_busy"
        );
        drop(lock);
        let mut other = bytes.clone();
        other[4..8].copy_from_slice(&0x01000007u32.to_le_bytes());
        assert!(
            deploy_embedded(&store, &other, &sha256(&other), env!("CARGO_PKG_VERSION")).is_err()
        );
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}
