use crate::error::{Result, fail};
use std::sync::Mutex;
use zeroize::Zeroizing;

#[cfg(target_os = "macos")]
mod macos_mapping_access;

/// Only native services and the narrowly scoped helper can access this trait.
pub trait Vault: Send + Sync {
    fn get(&self, reference: &str) -> Result<Zeroizing<Vec<u8>>>;
    /// Absence must be proven by the provider. Legacy adapters fail closed
    /// rather than interpreting a generic read error as a missing item.
    fn get_optional(&self, reference: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        self.get(reference).map(Some)
    }
    fn put(&self, reference: &str, value: &[u8]) -> Result<()>;
    /// Create one new mapping token, with no generic put or ACL-update fallback.
    /// Readers and legacy adapters do not acquire this capability implicitly.
    fn create_claude_mapping_secret(&self, _: &str, _: &[u8]) -> Result<()> {
        Err(fail("vault_unavailable"))
    }
    fn delete(&self, reference: &str) -> Result<()>;
}
pub struct NativeVault {
    lock: Mutex<()>,
    #[cfg(target_os = "macos")]
    mapping_creator: Option<macos_mapping_access::MappingSecretCreator>,
}
impl Default for NativeVault {
    fn default() -> Self {
        Self {
            lock: Mutex::new(()),
            #[cfg(target_os = "macos")]
            mapping_creator: None,
        }
    }
}
impl NativeVault {
    /// Inject immutable GUI deployment facts only. No OS or Keychain access is
    /// performed here; the helper's default vault has no creation capability.
    #[cfg(target_os = "macos")]
    pub fn with_claude_mapping_creator(root: std::path::PathBuf, helper_sha256: String) -> Self {
        Self {
            lock: Mutex::new(()),
            mapping_creator: Some(macos_mapping_access::MappingSecretCreator::new(
                root,
                helper_sha256,
            )),
        }
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn entry(reference: &str) -> Result<keyring_core::Entry> {
        use keyring_core::api::CredentialStoreApi;
        if reference.is_empty() || reference.len() > 240 || reference.chars().any(char::is_control)
        {
            return Err(fail("invalid_vault_reference"));
        }
        #[cfg(target_os = "macos")]
        let store = apple_native_keyring_store::keychain::Store::new()
            .map_err(|_| fail("vault_unavailable"))?;
        #[cfg(target_os = "windows")]
        let store =
            windows_native_keyring_store::Store::new().map_err(|_| fail("vault_unavailable"))?;
        #[cfg(target_os = "windows")]
        let modifiers = std::collections::HashMap::from([("persistence", "Local")]);
        #[cfg(target_os = "macos")]
        let modifiers = std::collections::HashMap::new();
        store
            .build("io.orivelle.orbit", reference, Some(&modifiers))
            .map_err(|_| fail("vault_unavailable"))
    }
}
impl Vault for NativeVault {
    fn get(&self, reference: &str) -> Result<Zeroizing<Vec<u8>>> {
        let _guard = self.lock.lock().map_err(|_| fail("vault_unavailable"))?;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let value = Self::entry(reference)?
                .get_secret()
                .map_err(|_| fail("credential_missing"))?;
            if value.is_empty() {
                return Err(fail("credential_missing"));
            }
            Ok(Zeroizing::new(value))
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = reference;
            Err(fail("vault_unavailable"))
        }
    }
    fn get_optional(&self, reference: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let _guard = self.lock.lock().map_err(|_| fail("vault_unavailable"))?;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            match Self::entry(reference)?.get_secret() {
                Ok(value) if value.is_empty() => Err(fail("credential_missing")),
                Ok(value) => Ok(Some(Zeroizing::new(value))),
                Err(keyring_core::Error::NoEntry) => Ok(None),
                Err(_) => Err(fail("vault_unavailable")),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = reference;
            Err(fail("vault_unavailable"))
        }
    }
    fn put(&self, reference: &str, value: &[u8]) -> Result<()> {
        let _guard = self.lock.lock().map_err(|_| fail("vault_unavailable"))?;
        if value.is_empty() || value.len() > 64 * 1024 {
            return Err(fail("invalid_secret"));
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            Self::entry(reference)?
                .set_secret(value)
                .map_err(|_| fail("vault_unavailable"))
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = reference;
            Err(fail("vault_unavailable"))
        }
    }
    fn delete(&self, reference: &str) -> Result<()> {
        let _guard = self.lock.lock().map_err(|_| fail("vault_unavailable"))?;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            match Self::entry(reference)?.delete_credential() {
                Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
                Err(_) => Err(fail("vault_unavailable")),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = reference;
            Err(fail("vault_unavailable"))
        }
    }
    fn create_claude_mapping_secret(&self, binding_id: &str, value: &[u8]) -> Result<()> {
        let _guard = self.lock.lock().map_err(|_| fail("vault_unavailable"))?;
        #[cfg(target_os = "macos")]
        {
            self.mapping_creator
                .as_ref()
                .ok_or_else(|| fail("vault_unavailable"))?
                .create(binding_id, value)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (binding_id, value);
            Err(fail("vault_unavailable"))
        }
    }
}
#[derive(Default)]
pub struct DeniedVault;
impl Vault for DeniedVault {
    fn get(&self, _: &str) -> Result<Zeroizing<Vec<u8>>> {
        Err(fail("vault_unavailable"))
    }
    fn put(&self, _: &str, _: &[u8]) -> Result<()> {
        Err(fail("vault_unavailable"))
    }
    fn delete(&self, _: &str) -> Result<()> {
        Err(fail("vault_unavailable"))
    }
}
#[cfg(test)]
#[derive(Default)]
pub struct FakeVault {
    entries: Mutex<std::collections::HashMap<String, Zeroizing<Vec<u8>>>>,
}
#[cfg(test)]
impl Vault for FakeVault {
    fn get(&self, key: &str) -> Result<Zeroizing<Vec<u8>>> {
        self.entries
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| fail("credential_missing"))
    }
    fn get_optional(&self, key: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let value = self.entries.lock().unwrap().get(key).cloned();
        if value.as_ref().is_some_and(|bytes| bytes.is_empty()) {
            return Err(fail("credential_missing"));
        }
        Ok(value)
    }
    fn put(&self, key: &str, value: &[u8]) -> Result<()> {
        self.entries
            .lock()
            .unwrap()
            .insert(key.into(), Zeroizing::new(value.to_vec()));
        Ok(())
    }
    fn create_claude_mapping_secret(&self, binding_id: &str, value: &[u8]) -> Result<()> {
        let id = uuid::Uuid::parse_str(binding_id).map_err(|_| fail("invalid_identifier"))?;
        if id.to_string() != binding_id {
            return Err(fail("invalid_identifier"));
        }
        if value.is_empty() || value.len() > 64 * 1024 {
            return Err(fail("invalid_secret"));
        }
        let key = format!("claude-map/{binding_id}");
        let mut entries = self.entries.lock().unwrap();
        if entries.contains_key(&key) {
            return Err(fail("vault_unavailable"));
        }
        entries.insert(key, Zeroizing::new(value.to_vec()));
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.entries.lock().unwrap().remove(key);
        Ok(())
    }
}
