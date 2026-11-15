//! GUI-only creation of new Windows Credential Manager mapping tokens.
//! Windows credentials are scoped to the user, not to a per-application ACL.
use super::NativeVault;
use crate::{
    error::{Result, fail},
    identity::sha256,
};
use std::path::PathBuf;
use zeroize::Zeroizing;

pub(super) struct MappingSecretCreator {
    root: PathBuf,
    helper_sha256: String,
}
impl MappingSecretCreator {
    pub(super) fn new(root: PathBuf, helper_sha256: String) -> Self {
        Self {
            root,
            helper_sha256,
        }
    }
    pub(super) fn create(&self, binding_id: &str, value: &[u8]) -> Result<()> {
        let id = uuid::Uuid::parse_str(binding_id).map_err(|_| fail("invalid_identifier"))?;
        if id.to_string() != binding_id {
            return Err(fail("invalid_identifier"));
        }
        if value.is_empty() || value.len() > 64 * 1024 {
            return Err(fail("invalid_secret"));
        }
        let _scope = crate::windows_fs::lock_directory(&self.root)?;
        crate::windows_fs::identity(&crate::windows_fs::directory(&self.root)?, true)?;
        // All Orbit creators and helper deployment coordinate on this lock.
        // Credential Manager has no atomic create-only operation; absence and
        // readback are checked while the cooperative lock remains held.
        let _lock = crate::updates::exclusive_helper_lock(&self.root)?;
        let bin = self.root.join("bin");
        let _bin = crate::windows_fs::lock_directory(&bin)?;
        crate::windows_fs::identity(&crate::windows_fs::directory(&bin)?, true)?;
        let helper = bin.join(crate::helper_artifact::binary_name());
        let _helper = crate::windows_fs::lock_installed_file(&helper)?;
        let bytes = crate::images::bounded_read(&helper, crate::helper_artifact::MAX_HELPER_BYTES)?;
        if sha256(&bytes) != self.helper_sha256 {
            return Err(fail("helper_artifact_invalid"));
        }
        let reference = format!("claude-map/{binding_id}");
        let entry = NativeVault::entry(&reference)?;
        match entry.get_secret() {
            Err(keyring_core::Error::NoEntry) => {}
            _ => return Err(fail("vault_unavailable")),
        }
        entry
            .set_secret(value)
            .map_err(|_| fail("vault_unavailable"))?;
        let readback = Zeroizing::new(entry.get_secret().map_err(|_| fail("vault_unavailable"))?);
        if readback.as_slice() != value {
            return Err(fail("vault_unavailable"));
        }
        Ok(())
    }
}
