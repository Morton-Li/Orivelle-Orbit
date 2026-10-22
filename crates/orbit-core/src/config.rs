use crate::{
    dto::{ClientKind, CodexCatalogBindingView, ReasoningPolicyView},
    error::{Result, fail},
    identity::{HelperBinding, Installation, sha256, uuid},
    store::{Store, private_file},
    system::CodexConfigurationTarget,
    vault::Vault,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    AeadCore, KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, OsRng, Payload},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, Table, value};

pub const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CodexModelInput {
    pub(crate) id: String,
    pub(crate) provider_id: String,
    pub(crate) revision: String,
    pub(crate) category: String,
    pub(crate) capabilities: Vec<String>,
    pub(crate) binding: Option<CodexCatalogBindingView>,
    pub(crate) reasoning: Option<ReasoningPolicyView>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CodexContext {
    pub(crate) panel_origin: String,
    pub(crate) account_id: String,
    pub(crate) installation_id: String,
    pub(crate) connection_id: String,
    pub(crate) connection_revision: u64,
    pub(crate) authorization_epoch: u64,
    pub(crate) credential_revision: String,
    pub(crate) credential_reference: String,
    pub(crate) credential_expires_at: Option<i64>,
    pub(crate) api_key_id: Option<String>,
    pub(crate) node_id: String,
    pub(crate) node_revision: String,
    pub(crate) edge_base_url: String,
    pub(crate) provider_id: String,
    pub(crate) catalog_revision: String,
    pub(crate) models: Vec<CodexModelInput>,
    pub(crate) profile_id: String,
    pub(crate) source_sha256: String,
    pub(crate) scope_revision: String,
    pub(crate) catalog_sha256: String,
    pub(crate) catalog_size: u64,
    pub(crate) aliases: Vec<String>,
    pub(crate) default_alias: String,
    pub(crate) default_reasoning_effort: String,
    pub(crate) catalog_path: String,
    pub(crate) config_path: String,
    // Legacy runtime-only contexts deserialize without a new receipt and fail
    // closed. They are never promoted into an installed-target qualification.
    #[serde(default)]
    pub(crate) configuration_target: Option<CodexConfigurationTarget>,
}
#[derive(Clone, Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
pub struct FileChange {
    pub path: String,
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
}
#[derive(Clone, Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
struct Operation {
    files: Vec<FileChange>,
    #[zeroize(skip)]
    enable_binding: Option<String>,
    #[zeroize(skip)]
    disable_bindings: Vec<String>,
    #[zeroize(skip)]
    #[serde(default)]
    grants_before: Vec<HelperBinding>,
    #[zeroize(skip)]
    #[serde(default)]
    grants_after: Vec<HelperBinding>,
    #[serde(default)]
    restore: bool,
    #[zeroize(skip)]
    #[serde(default)]
    source_journal_id: Option<String>,
    #[zeroize(skip)]
    #[serde(default)]
    codex_context: Option<CodexContext>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Journal {
    pub id: String,
    pub client: ClientKind,
    pub account_id: String,
    #[serde(default)]
    pub account_label: String,
    pub installation: Installation,
    pub created_at: String,
    pub expires_at: String,
    pub phase: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub target_version: Option<String>,
    #[serde(default)]
    pub target_bundle_id: Option<String>,
    #[serde(default)]
    pub target_policy_revision: Option<String>,
    pub nonce: String,
    #[serde(default)]
    pub ciphertext: String,
}
impl std::fmt::Debug for Journal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Journal")
            .field("id", &self.id)
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}
pub struct ConfigEngine<'a> {
    pub store: &'a Store,
    pub vault: &'a dyn Vault,
    pub allowed_roots: Vec<PathBuf>,
    pub(crate) policy_guard: Option<&'a dyn Fn() -> Result<()>>,
    pub(crate) context_guard: Option<&'a dyn Fn(Option<&CodexContext>, bool) -> Result<()>>,
}
pub fn read_file(path: &Path) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(fail("configuration_unreadable")),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_CONFIG_BYTES
    {
        return Err(fail("configuration_unreadable"));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|f| f.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|_| fail("configuration_unreadable"))?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(fail("configuration_unreadable"));
    }
    Ok(Some(bytes))
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| fail("invalid_path"))?;
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(fail("configuration_conflict"));
    }
    let mut staged =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| fail("local_save_failed"))?;
    private_file(staged.path())?;
    staged
        .write_all(bytes)
        .and_then(|_| staged.as_file().sync_all())
        .map_err(|_| fail("local_save_failed"))?;
    staged
        .persist(path)
        .map_err(|_| fail("local_save_failed"))?;
    #[cfg(unix)]
    File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|_| fail("local_save_failed"))?;
    Ok(())
}
fn equal_current(file: &FileChange, expected: &Option<Vec<u8>>) -> Result<bool> {
    Ok(read_file(Path::new(&file.path))? == *expected)
}
impl ConfigEngine<'_> {
    fn check_context(&self, journal: &Journal, operation: &Operation) -> Result<()> {
        self.check_context_for(journal, operation, false)
    }
    fn check_context_for(
        &self,
        journal: &Journal,
        operation: &Operation,
        restoring: bool,
    ) -> Result<()> {
        if journal.client != ClientKind::Codex {
            return Ok(());
        }
        if let Some(context) = &operation.codex_context {
            let target = context
                .configuration_target
                .as_ref()
                .ok_or_else(|| fail("codex_catalog_context_unverified"))?;
            target.validate_root()?;
            if context.panel_origin != journal.installation.origin
                || context.installation_id != journal.installation.id
                || context.account_id != journal.account_id
                || journal.paths
                    != operation
                        .files
                        .iter()
                        .map(|file| file.path.clone())
                        .collect::<Vec<_>>()
                || operation.files.len() != 2
                || operation.files[0].path != context.catalog_path
                || operation.files[1].path != context.config_path
                || context.catalog_path == context.config_path
                || Path::new(&context.config_path) != target.config_root.join("config.toml")
                || Path::new(&context.catalog_path)
                    != target.config_root.join("orivelle-orbit.models.json")
                || context.models.len() > crate::codex_catalog::MAX_AUTHORIZED_ALIASES
                || context.aliases.is_empty()
                || context.aliases.len() > crate::codex_catalog::MAX_MODELS
            {
                return Err(fail("codex_catalog_context_unverified"));
            }
            if !operation.restore {
                let catalog = operation.files[0]
                    .after
                    .as_ref()
                    .ok_or_else(|| fail("backup_invalid"))?;
                if catalog.len() as u64 != context.catalog_size
                    || catalog.len() > crate::codex_catalog::MAX_OUTPUT_BYTES
                    || sha256(catalog) != context.catalog_sha256
                {
                    return Err(fail("backup_invalid"));
                }
                let bytes = operation.files[1]
                    .after
                    .as_ref()
                    .ok_or_else(|| fail("backup_invalid"))?;
                let document = std::str::from_utf8(bytes)
                    .ok()
                    .and_then(|text| text.parse::<DocumentMut>().ok())
                    .ok_or_else(|| fail("backup_invalid"))?;
                if document.get("model").and_then(Item::as_str)
                    != Some(context.default_alias.as_str())
                    || document.get("model_catalog_json").and_then(Item::as_str)
                        != Some(context.catalog_path.as_str())
                    || document
                        .get("model_reasoning_effort")
                        .and_then(Item::as_str)
                        != Some(context.default_reasoning_effort.as_str())
                {
                    return Err(fail("backup_invalid"));
                }
            }
        }
        if let Some(guard) = self.context_guard {
            return guard(
                operation.codex_context.as_ref(),
                operation.restore || restoring,
            );
        }
        #[cfg(test)]
        return Ok(());
        #[cfg(not(test))]
        Err(fail("codex_catalog_context_unverified"))
    }
    fn check_policy(&self) -> Result<()> {
        if let Some(guard) = self.policy_guard {
            return guard();
        }
        #[cfg(test)]
        return Ok(());
        #[cfg(not(test))]
        Err(fail("managed_policy_unverified"))
    }
    fn validate_path(&self, path: &Path, context: Option<&CodexContext>) -> Result<()> {
        if !path.is_absolute() {
            return Err(fail("invalid_path"));
        }
        let requested_parent = path.parent().ok_or_else(|| fail("invalid_path"))?;
        let parent = match requested_parent.canonicalize() {
            Ok(parent) => parent,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let target = context
                    .and_then(|context| context.configuration_target.as_ref())
                    .ok_or_else(|| fail("invalid_path"))?;
                target.validate_root()?;
                if requested_parent != target.config_root
                    || !["config.toml", "orivelle-orbit.models.json"]
                        .iter()
                        .any(|name| path == target.config_root.join(name))
                    || !self.allowed_roots.iter().any(|root| {
                        *root == target.config_root
                            || root
                                .canonicalize()
                                .is_ok_and(|allowed| target.config_root.starts_with(allowed))
                    })
                {
                    return Err(fail("path_outside_scope"));
                }
                return Ok(());
            }
            Err(_) => return Err(fail("invalid_path")),
        };
        if !self
            .allowed_roots
            .iter()
            .any(|root| root.canonicalize().is_ok_and(|r| parent.starts_with(r)))
        {
            return Err(fail("path_outside_scope"));
        }
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(fail("configuration_conflict"));
        }
        Ok(())
    }
    fn lock(&self, client: ClientKind) -> Result<File> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(
                self.store
                    .root
                    .join("locks")
                    .join(format!("{}.lock", client.as_str())),
            )
            .map_err(|_| fail("configuration_busy"))?;
        file.try_lock_exclusive()
            .map_err(|_| fail("configuration_busy"))?;
        Ok(file)
    }
    fn cipher(&self, installation: &Installation) -> Result<XChaCha20Poly1305> {
        self.check_policy()?;
        let key = self.vault.get(&installation.backup_reference)?;
        self.check_policy()?;
        XChaCha20Poly1305::new_from_slice(&key).map_err(|_| fail("vault_unavailable"))
    }
    fn aad(journal: &Journal) -> String {
        if let Some(revision) = &journal.target_policy_revision {
            return format!(
                "orbit-config-v3\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
                journal.id,
                journal.installation.origin,
                journal.installation.id,
                journal.client.as_str(),
                journal.account_id,
                journal.target_version.as_deref().unwrap_or(""),
                journal.target_bundle_id.as_deref().unwrap_or(""),
                revision
            );
        }
        format!(
            "orbit-config-v2\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
            journal.id,
            journal.installation.origin,
            journal.installation.id,
            journal.client.as_str(),
            journal.account_id,
            journal.target_version.as_deref().unwrap_or(""),
            journal.target_bundle_id.as_deref().unwrap_or("")
        )
    }
    fn decrypt(&self, journal: &Journal) -> Result<Operation> {
        let nonce = URL_SAFE_NO_PAD
            .decode(&journal.nonce)
            .map_err(|_| fail("backup_invalid"))?;
        if nonce.len() != 24 {
            return Err(fail("backup_invalid"));
        }
        let ciphertext = URL_SAFE_NO_PAD
            .decode(&journal.ciphertext)
            .map_err(|_| fail("backup_invalid"))?;
        let plain = zeroize::Zeroizing::new(
            self.cipher(&journal.installation)?
                .decrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: &ciphertext,
                        aad: Self::aad(journal).as_bytes(),
                    },
                )
                .map_err(|_| fail("backup_invalid"))?,
        );
        serde_json::from_slice(&plain).map_err(|_| fail("backup_invalid"))
    }
    pub fn prepare(
        &self,
        client: ClientKind,
        account: &str,
        installation: &Installation,
        files: Vec<FileChange>,
        enable_binding: Option<String>,
        disable_bindings: Vec<String>,
    ) -> Result<Journal> {
        self.prepare_operation(
            client,
            account,
            installation,
            files,
            enable_binding,
            disable_bindings,
            None,
            false,
        )
    }
    pub(crate) fn prepare_codex(
        &self,
        account: &str,
        installation: &Installation,
        files: Vec<FileChange>,
        enable_binding: Option<String>,
        disable_bindings: Vec<String>,
        context: CodexContext,
    ) -> Result<Journal> {
        self.prepare_operation(
            ClientKind::Codex,
            account,
            installation,
            files,
            enable_binding,
            disable_bindings,
            Some(context),
            false,
        )
    }
    fn prepare_operation(
        &self,
        client: ClientKind,
        account: &str,
        installation: &Installation,
        files: Vec<FileChange>,
        enable_binding: Option<String>,
        disable_bindings: Vec<String>,
        codex_context: Option<CodexContext>,
        restore: bool,
    ) -> Result<Journal> {
        self.check_policy()?;
        if files.is_empty() || files.len() > 8 {
            return Err(fail("invalid_configuration"));
        }
        for file in &files {
            if file
                .before
                .as_ref()
                .is_some_and(|bytes| bytes.len() as u64 > MAX_CONFIG_BYTES)
                || file
                    .after
                    .as_ref()
                    .is_some_and(|bytes| bytes.len() as u64 > MAX_CONFIG_BYTES)
            {
                return Err(fail("invalid_configuration"));
            }
            self.validate_path(Path::new(&file.path), codex_context.as_ref())?;
            if !equal_current(file, &file.before)? {
                return Err(fail("configuration_conflict"));
            }
        }
        let now = chrono::Utc::now();
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let retention = self.store.settings()?.backup_retention_days.min(30);
        let mut journal = Journal {
            id: uuid(),
            client,
            account_id: account.into(),
            account_label: account.into(),
            installation: installation.clone(),
            created_at: now.to_rfc3339(),
            expires_at: (now + chrono::Duration::days(retention as i64)).to_rfc3339(),
            phase: "prepared".into(),
            paths: files.iter().map(|f| f.path.clone()).collect(),
            target_version: None,
            target_bundle_id: None,
            target_policy_revision: None,
            nonce: URL_SAFE_NO_PAD.encode(nonce),
            ciphertext: String::new(),
        };
        let operation = Operation {
            files,
            enable_binding,
            disable_bindings,
            grants_before: Vec::new(),
            grants_after: Vec::new(),
            restore,
            source_journal_id: None,
            codex_context,
        };
        let mut operation = operation;
        for id in operation
            .disable_bindings
            .iter()
            .chain(operation.enable_binding.iter())
        {
            let before = self
                .store
                .get::<HelperBinding>("binding", id)?
                .ok_or_else(|| fail("binding_disabled"))?;
            let mut after = before.clone();
            after.enabled = operation.enable_binding.as_ref() == Some(id);
            operation.grants_before.push(before);
            operation.grants_after.push(after);
        }
        self.check_context(&journal, &operation)?;
        self.persist_operation(&mut journal, &operation)?;
        Ok(journal)
    }
    fn persist_operation(&self, journal: &mut Journal, operation: &Operation) -> Result<()> {
        // A rewritten encrypted operation always gets a fresh nonce.
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        journal.nonce = URL_SAFE_NO_PAD.encode(nonce);
        let plain = zeroize::Zeroizing::new(
            serde_json::to_vec(&operation).map_err(|_| fail("serialization_failed"))?,
        );
        journal.ciphertext = URL_SAFE_NO_PAD.encode(
            self.cipher(&journal.installation)?
                .encrypt(
                    &nonce,
                    Payload {
                        msg: &plain,
                        aad: Self::aad(&journal).as_bytes(),
                    },
                )
                .map_err(|_| fail("backup_invalid"))?,
        );
        self.check_policy()?;
        self.store
            .put("journal", &journal.id, &journal.account_id, journal)?;
        Ok(())
    }
    pub fn freeze_target(
        &self,
        journal: &mut Journal,
        version: Option<String>,
        bundle_id: Option<String>,
    ) -> Result<()> {
        let operation = self.decrypt(journal)?;
        journal.target_version = version;
        journal.target_bundle_id = bundle_id;
        self.persist_operation(journal, &operation)
    }
    pub(crate) fn freeze_policy(&self, journal: &mut Journal, revision: &str) -> Result<()> {
        if revision.len() != 64
            || !revision
                .bytes()
                .all(|v| v.is_ascii_hexdigit() && !v.is_ascii_uppercase())
        {
            return Err(fail("managed_policy_unverified"));
        }
        let operation = self.decrypt(journal)?;
        journal.target_policy_revision = Some(revision.into());
        self.persist_operation(journal, &operation)
    }
    pub(crate) fn codex_context(&self, journal: &Journal) -> Result<Option<CodexContext>> {
        Ok(self.decrypt(journal)?.codex_context.clone())
    }
    #[cfg(test)]
    pub(crate) fn freeze_codex_context(
        &self,
        journal: &mut Journal,
        context: CodexContext,
    ) -> Result<()> {
        let mut operation = self.decrypt(journal)?;
        operation.codex_context = Some(context);
        self.check_context(journal, &operation)?;
        self.persist_operation(journal, &operation)
    }
    #[cfg(test)]
    pub(crate) fn synthetic_files(&self, journal: &Journal) -> Result<Vec<FileChange>> {
        Ok(self.decrypt(journal)?.files.clone())
    }
    #[cfg(test)]
    pub(crate) fn synthetic_without_context(&self, journal: &mut Journal) -> Result<()> {
        let mut operation = self.decrypt(journal)?;
        operation.codex_context = None;
        self.persist_operation(journal, &operation)
    }
    #[cfg(test)]
    pub(crate) fn synthetic_without_configuration_target(
        &self,
        journal: &mut Journal,
    ) -> Result<()> {
        let mut operation = self.decrypt(journal)?;
        operation
            .codex_context
            .as_mut()
            .unwrap()
            .configuration_target = None;
        self.persist_operation(journal, &operation)
    }
    fn publish(&self, journal: &mut Journal, operation: &Operation) -> Result<()> {
        self.check_policy()?;
        self.check_context(journal, operation)?;
        if let Some(id) = &operation.source_journal_id {
            let source = self
                .store
                .get::<Journal>("journal", id)?
                .ok_or_else(|| fail("backup_invalid"))?;
            if source.client != journal.client
                || source.account_id != journal.account_id
                || source.installation.id != journal.installation.id
                || source.installation.origin != journal.installation.origin
                || !["publishing", "conflict", "committed"].contains(&source.phase.as_str())
            {
                return Err(fail("backup_invalid"));
            }
        }
        let mut grants = Vec::new();
        for desired in &operation.grants_after {
            let mut current = self
                .store
                .get::<HelperBinding>("binding", &desired.id)?
                .ok_or_else(|| fail("binding_disabled"))?;
            if current.client != desired.client
                || current.origin != desired.origin
                || current.account_id != desired.account_id
                || current.installation_id != desired.installation_id
                || current.provider_id != desired.provider_id
                || current.credential_revision != desired.credential_revision
                || current.secret_reference != desired.secret_reference
                || current.authorization_epoch != desired.authorization_epoch
            {
                return Err(fail("configuration_conflict"));
            }
            current.enabled = desired.enabled;
            current.expires_at = match (current.expires_at, desired.expires_at) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) | (None, Some(a)) => Some(a),
                (None, None) => None,
            };
            self.check_policy()?;
            if current.enabled
                && crate::identity::bound_helper_token(
                    &current,
                    current.client,
                    &current.id,
                    self.store,
                    self.vault,
                    chrono::Utc::now().timestamp(),
                )
                .is_err()
            {
                if operation.restore {
                    current.enabled = false;
                } else {
                    return Err(fail("credential_missing"));
                }
            }
            grants.push(current);
        }
        // Inspect every file before any repair write. External changes stop the whole recovery.
        for file in &operation.files {
            self.validate_path(Path::new(&file.path), operation.codex_context.as_ref())?;
            if !equal_current(file, &file.before)? && !equal_current(file, &file.after)? {
                journal.phase = "conflict".into();
                self.store
                    .put("journal", &journal.id, &journal.account_id, journal)?;
                return Err(fail("configuration_conflict"));
            }
        }
        journal.phase = "publishing".into();
        self.check_policy()?;
        self.store
            .put("journal", &journal.id, &journal.account_id, journal)?;
        if operation.files.iter().any(|file| file.after.is_some()) {
            if let Some(target) = operation
                .codex_context
                .as_ref()
                .and_then(|context| context.configuration_target.as_ref())
            {
                self.check_policy()?;
                self.check_context(journal, operation)?;
                target.ensure_root()?;
            }
        }
        for file in &operation.files {
            self.check_policy()?;
            self.check_context(journal, operation)?;
            if equal_current(file, &file.after)? {
                continue;
            }
            if !equal_current(file, &file.before)? {
                journal.phase = "conflict".into();
                self.store
                    .put("journal", &journal.id, &journal.account_id, journal)?;
                return Err(fail("configuration_conflict"));
            }
            self.check_policy()?;
            self.check_context(journal, operation)?;
            match &file.after {
                Some(bytes) => atomic_write(Path::new(&file.path), bytes)?,
                None => {
                    if Path::new(&file.path).exists() {
                        fs::remove_file(&file.path).map_err(|_| fail("local_save_failed"))?;
                    }
                }
            }
            if !equal_current(file, &file.after)? {
                return Err(fail("configuration_readback_failed"));
            }
        }
        for id in &operation.disable_bindings {
            if let Some(mut b) = self.store.get::<HelperBinding>("binding", id)? {
                b.enabled = false;
                self.check_policy()?;
                self.check_context(journal, operation)?;
                self.store.put("binding", id, &b.account_id, &b)?;
            }
        }
        if operation.grants_after.is_empty() {
            if let Some(id) = &operation.enable_binding {
                self.check_policy()?;
                let mut b = self
                    .store
                    .get::<HelperBinding>("binding", id)?
                    .ok_or_else(|| fail("binding_disabled"))?;
                b.enabled = true;
                self.check_policy()?;
                self.check_context(journal, operation)?;
                self.store.put("binding", id, &b.account_id, &b)?;
            }
        }
        for grant in grants {
            self.check_policy()?;
            self.check_context(journal, operation)?;
            self.store
                .put("binding", &grant.id, &grant.account_id, &grant)?;
        }
        journal.phase = "committed".into();
        self.check_policy()?;
        self.check_context(journal, operation)?;
        self.store
            .commit_configuration(journal, operation.source_journal_id.as_deref())?;
        self.prune_backups_locked(journal.client)?;
        Ok(())
    }
    pub fn apply(&self, id: &str) -> Result<Journal> {
        self.check_policy()?;
        let mut journal = self
            .store
            .get::<Journal>("journal", id)?
            .ok_or_else(|| fail("preview_expired"))?;
        if journal.phase != "prepared" {
            return Err(fail("configuration_conflict"));
        }
        let operation = self.decrypt(&journal)?;
        self.check_context(&journal, &operation)?;
        let _lock = self.lock(journal.client)?;
        for file in &operation.files {
            if !equal_current(file, &file.before)? {
                return Err(fail("configuration_conflict"));
            }
        }
        self.publish(&mut journal, &operation)?;
        Ok(journal)
    }
    pub fn recover(&self, id: &str) -> Result<Journal> {
        self.check_policy()?;
        let mut journal = self
            .store
            .get::<Journal>("journal", id)?
            .ok_or_else(|| fail("backup_invalid"))?;
        let operation = self.decrypt(&journal)?;
        self.check_context(&journal, &operation)?;
        let _lock = self.lock(journal.client)?;
        if journal.phase == "conflict" {
            return Err(fail("configuration_conflict"));
        }
        if journal.phase == "committed" {
            for file in &operation.files {
                if !equal_current(file, &file.after)? {
                    return Err(fail("configuration_conflict"));
                }
            }
            self.check_policy()?;
            self.check_context(&journal, &operation)?;
            self.store
                .commit_configuration(&journal, operation.source_journal_id.as_deref())?;
            return Ok(journal);
        }
        if journal.phase != "publishing" {
            return Err(fail("preview_expired"));
        }
        self.publish(&mut journal, &operation)?;
        Ok(journal)
    }
    pub fn prepare_restore(&self, id: &str) -> Result<Journal> {
        self.check_policy()?;
        let journal =
            self.store
                .get::<Journal>("backup", id)?
                .or(self.store.get::<Journal>("journal", id)?.filter(|j| {
                    ["publishing", "conflict", "committed"].contains(&j.phase.as_str())
                }))
                .ok_or_else(|| fail("backup_invalid"))?;
        if chrono::DateTime::parse_from_rfc3339(&journal.expires_at)
            .map_err(|_| fail("backup_invalid"))?
            <= chrono::Utc::now()
        {
            return Err(fail("backup_expired"));
        }
        let original = self.decrypt(&journal)?;
        self.check_context_for(&journal, &original, true)?;
        let mut files = Vec::new();
        for file in &original.files {
            let current = read_file(Path::new(&file.path))?;
            if current != file.after && (journal.phase == "committed" || current != file.before) {
                return Err(fail("configuration_conflict"));
            }
            files.push(FileChange {
                path: file.path.clone(),
                before: current,
                after: file.before.clone(),
            });
        }
        let mut restored = self.prepare_operation(
            journal.client,
            &journal.account_id,
            &journal.installation,
            files,
            None,
            original.enable_binding.clone().into_iter().collect(),
            original.codex_context.clone(),
            true,
        )?;
        let mut operation = self.decrypt(&restored)?;
        operation.restore = true;
        operation.source_journal_id = Some(journal.id.clone());
        operation.grants_after = original.grants_before.clone();
        operation.codex_context = original.codex_context.clone();
        operation.grants_before = operation
            .grants_after
            .iter()
            .map(|b| {
                self.store
                    .get::<HelperBinding>("binding", &b.id)?
                    .ok_or_else(|| fail("binding_disabled"))
            })
            .collect::<Result<Vec<_>>>()?;
        restored.account_label = journal.account_label;
        restored.target_version = journal.target_version;
        restored.target_bundle_id = journal.target_bundle_id;
        self.persist_operation(&mut restored, &operation)?;
        Ok(restored)
    }
    pub fn prune_backups(&self, client: ClientKind) -> Result<()> {
        let _lock = self.lock(client)?;
        self.prune_backups_locked(client)
    }
    fn prune_backups_locked(&self, client: ClientKind) -> Result<()> {
        let settings = self.store.settings()?;
        let mut backups = self
            .store
            .list::<Journal>("backup", None)?
            .into_iter()
            .filter(|b| b.client == client)
            .collect::<Vec<_>>();
        backups.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        for (index, backup) in backups.into_iter().enumerate() {
            let expired = chrono::DateTime::parse_from_rfc3339(&backup.expires_at)
                .map_or(true, |at| at <= chrono::Utc::now());
            if expired || index >= settings.backup_max_groups.min(5) as usize {
                self.store.delete("backup", &backup.id)?;
                if self
                    .store
                    .get::<Journal>("journal", &backup.id)?
                    .is_some_and(|j| ["committed", "restored"].contains(&j.phase.as_str()))
                {
                    self.store.delete("journal", &backup.id)?;
                }
            }
        }
        Ok(())
    }
}
pub fn codex_patch(
    original: Option<&[u8]>,
    base_url: &str,
    model: &str,
    helper: &Path,
    binding: &str,
) -> Result<Vec<u8>> {
    crate::identity::validate_uuid(binding)?;
    if !helper.is_absolute() || model.is_empty() || model.len() > 255 {
        return Err(fail("invalid_configuration"));
    }
    let url = url::Url::parse(base_url).map_err(|_| fail("invalid_configuration"))?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(fail("invalid_configuration"));
    }
    let text = std::str::from_utf8(original.unwrap_or_default())
        .map_err(|_| fail("invalid_configuration"))?;
    let mut doc = text
        .parse::<DocumentMut>()
        .map_err(|_| fail("invalid_configuration"))?;
    doc["model"] = value(model);
    doc["model_provider"] = value("orivelle_orbit");
    if !doc.as_table().contains_key("model_providers") {
        doc["model_providers"] = Item::Table(Table::new());
    }
    if !doc["model_providers"].is_table() {
        return Err(fail("invalid_configuration"));
    }
    if doc["model_providers"].get("orivelle_orbit").is_none() {
        doc["model_providers"]["orivelle_orbit"] = Item::Table(Table::new());
    }
    let provider = doc["model_providers"]["orivelle_orbit"]
        .as_table_mut()
        .ok_or_else(|| fail("invalid_configuration"))?;
    provider["name"] = value("Orivelle Orbit");
    provider["base_url"] = value(base_url);
    provider["wire_api"] = value("responses");
    for key in [
        "env_key",
        "experimental_bearer_token",
        "requires_openai_auth",
    ] {
        provider.remove(key);
    }
    if provider.get("auth").is_some_and(|i| !i.is_table()) {
        return Err(fail("invalid_configuration"));
    }
    let mut auth = provider
        .get("auth")
        .and_then(Item::as_table)
        .cloned()
        .unwrap_or_default();
    auth["command"] = value(helper.to_str().ok_or_else(|| fail("invalid_path"))?);
    let mut args = toml_edit::Array::new();
    args.push("codex");
    args.push(binding);
    auth["args"] = value(args);
    auth["timeout_ms"] = value(5000);
    auth["refresh_interval_ms"] = value(300000);
    provider["auth"] = Item::Table(auth);
    Ok(doc.to_string().into_bytes())
}
pub fn codex_catalog_patch(
    original: Option<&[u8]>,
    base_url: &str,
    model: &str,
    helper: &Path,
    binding: &str,
    catalog_path: &Path,
    default_effort: &str,
) -> Result<Vec<u8>> {
    if !catalog_path.is_absolute()
        || default_effort.is_empty()
        || default_effort.len() > 128
        || !default_effort.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(fail("invalid_configuration"));
    }
    let after = codex_patch(original, base_url, model, helper, binding)?;
    let mut document = std::str::from_utf8(&after)
        .map_err(|_| fail("invalid_configuration"))?
        .parse::<DocumentMut>()
        .map_err(|_| fail("invalid_configuration"))?;
    let owned = [
        "model",
        "model_provider",
        "model_catalog_json",
        "model_reasoning_effort",
    ];
    if document.get("profile").is_some()
        || document
            .get("profiles")
            .and_then(Item::as_table_like)
            .is_some_and(|profiles| {
                profiles
                    .iter()
                    .any(|(_, profile)| owned.iter().any(|key| profile.get(key).is_some()))
            })
    {
        return Err(fail("codex_config_override_unverified"));
    }
    document["model_catalog_json"] =
        value(catalog_path.to_str().ok_or_else(|| fail("invalid_path"))?);
    document["model_reasoning_effort"] = value(default_effort);
    let bytes = document.to_string().into_bytes();
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(fail("invalid_configuration"));
    }
    Ok(bytes)
}
pub fn codex_configuration_revision(config: &Path, catalog: &Path) -> Result<Option<String>> {
    let files = [(config, read_file(config)?), (catalog, read_file(catalog)?)];
    if files.iter().all(|(_, bytes)| bytes.is_none()) {
        return Ok(None);
    }
    let framed: Vec<_> = files
        .iter()
        .map(|(path, bytes)| {
            (
                path.to_string_lossy(),
                bytes.is_some(),
                bytes.as_ref().map(|bytes| sha256(bytes)),
            )
        })
        .collect();
    Ok(Some(sha256(
        &serde_json::to_vec(&(1u32, framed)).map_err(|_| fail("serialization_failed"))?,
    )))
}
pub fn claude_import_diagnostic(
    base_url: &str,
    helper: &Path,
    binding: &str,
    model: &str,
) -> Result<serde_json::Value> {
    crate::identity::validate_uuid(binding)?;
    let command = helper.to_str().ok_or_else(|| fail("invalid_path"))?;
    if !helper.is_absolute()
        || command.contains('"')
        || command.contains('%')
        || command.chars().any(char::is_control)
    {
        return Err(fail("invalid_configuration"));
    }
    let url = url::Url::parse(base_url).map_err(|_| fail("invalid_remote_url"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || model.is_empty()
        || model.len() > 256
        || model.chars().any(char::is_control)
    {
        return Err(fail("invalid_configuration"));
    }
    let mut credential = serde_json::json!({"kind":"helper-script","args":["claude",binding],"ttlSec":3600,"timeoutSec":5,"silentRefreshEnabled":true,"authScheme":"bearer"});
    #[cfg(windows)]
    {
        credential["commandWindows"] = serde_json::json!(command);
    }
    #[cfg(not(windows))]
    {
        credential["command"] = serde_json::json!(command);
    }
    Ok(
        serde_json::json!({"$schemaVersion":2,"inference":{"provider":"gateway","baseUrl":base_url,"credential":credential},"models":{"list":[model],"discoveryEnabled":false}}),
    )
}
pub fn configuration_hash(bytes: Option<&[u8]>) -> Option<String> {
    bytes.map(sha256)
}
pub fn codex_redacted_diff(before: Option<&[u8]>, after: &[u8]) -> Result<String> {
    fn document(bytes: &[u8]) -> Result<DocumentMut> {
        std::str::from_utf8(bytes)
            .map_err(|_| fail("invalid_configuration"))?
            .parse::<DocumentMut>()
            .map_err(|_| fail("invalid_configuration"))
    }
    fn field(doc: &DocumentMut, path: &[&str]) -> Option<String> {
        let mut item = doc.as_item();
        for part in path {
            item = item.get(part)?;
        }
        Some(item.to_string().trim().to_string())
    }
    fn safe(path: &[&str], value: Option<String>, previous: bool) -> String {
        match value {
            None => "<absent>".into(),
            Some(value) => {
                if previous
                    && path.iter().any(|p| *p == "auth")
                    && path.last().is_some_and(|p| ["command", "args"].contains(p))
                {
                    return "<redacted previous helper value>".into();
                }
                if previous && path.last() == Some(&"model_catalog_json") {
                    return "<redacted previous catalog path>".into();
                }
                if previous && path.last() == Some(&"base_url") {
                    let wrapped = format!("value={value}");
                    let suspicious = wrapped
                        .parse::<DocumentMut>()
                        .ok()
                        .and_then(|d| d.get("value").and_then(Item::as_str).map(str::to_owned))
                        .and_then(|s| url::Url::parse(&s).ok())
                        .is_none_or(|u| {
                            !u.username().is_empty()
                                || u.password().is_some()
                                || u.query().is_some()
                                || u.fragment().is_some()
                        });
                    if suspicious {
                        return "<redacted previous URL value>".into();
                    }
                }
                if path
                    .iter()
                    .any(|p| ["env_key", "experimental_bearer_token"].contains(p))
                    || value.contains("orv_sk_")
                    || value.contains("sk-")
                    || value.to_ascii_lowercase().contains("bearer ")
                {
                    "<redacted>".into()
                } else {
                    value
                }
            }
        }
    }
    let before = document(before.unwrap_or_default())?;
    let after = document(after)?;
    let paths: Vec<Vec<&str>> = vec![
        vec!["model"],
        vec!["model_provider"],
        vec!["model_catalog_json"],
        vec!["model_reasoning_effort"],
        vec!["model_providers", "orivelle_orbit", "name"],
        vec!["model_providers", "orivelle_orbit", "base_url"],
        vec!["model_providers", "orivelle_orbit", "wire_api"],
        vec!["model_providers", "orivelle_orbit", "env_key"],
        vec![
            "model_providers",
            "orivelle_orbit",
            "experimental_bearer_token",
        ],
        vec!["model_providers", "orivelle_orbit", "requires_openai_auth"],
        vec!["model_providers", "orivelle_orbit", "auth", "command"],
        vec!["model_providers", "orivelle_orbit", "auth", "args"],
        vec!["model_providers", "orivelle_orbit", "auth", "timeout_ms"],
        vec![
            "model_providers",
            "orivelle_orbit",
            "auth",
            "refresh_interval_ms",
        ],
    ];
    let mut out = String::new();
    for path in paths {
        let old = field(&before, &path);
        let new = field(&after, &path);
        if old != new {
            out.push_str(&format!(
                "{}\n- {}\n+ {}\n",
                path.join("."),
                safe(&path, old, true),
                safe(&path, new, false)
            ));
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::FakeVault;
    #[test]
    fn legacy_backup_requires_new_guarded_preview_but_can_restore_exact_original_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("owned")).unwrap();
        let vault = FakeVault::default();
        let install = Installation::create("https://panel.test", &vault).unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, b"original").unwrap();
        let legacy = ConfigEngine {
            store: &store,
            vault: &vault,
            allowed_roots: vec![dir.path().to_path_buf()],
            policy_guard: None,
            context_guard: None,
        };
        let journal = legacy
            .prepare(
                ClientKind::Codex,
                "synthetic",
                &install,
                vec![FileChange {
                    path: path.display().to_string(),
                    before: Some(b"original".to_vec()),
                    after: Some(b"changed".to_vec()),
                }],
                None,
                vec![],
            )
            .unwrap();
        assert!(journal.target_policy_revision.is_none());
        legacy.apply(&journal.id).unwrap();
        let clear = || Ok(());
        let current = ConfigEngine {
            store: &store,
            vault: &vault,
            allowed_roots: vec![dir.path().to_path_buf()],
            policy_guard: Some(&clear),
            context_guard: None,
        };
        let mut restore = current.prepare_restore(&journal.id).unwrap();
        current
            .freeze_policy(&mut restore, &"a".repeat(64))
            .unwrap();
        assert_eq!(
            restore.target_policy_revision.as_deref(),
            Some("a".repeat(64).as_str())
        );
        current.apply(&restore.id).unwrap();
        assert_eq!(fs::read(path).unwrap(), b"original");
    }
    #[test]
    fn policy_and_identity_metadata_are_authenticated_and_v3_cannot_downgrade_to_v2() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("owned")).unwrap();
        let vault = FakeVault::default();
        let install = Installation::create("https://panel.test", &vault).unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, b"original").unwrap();
        let guard = || Ok(());
        let engine = ConfigEngine {
            store: &store,
            vault: &vault,
            allowed_roots: vec![dir.path().to_path_buf()],
            policy_guard: Some(&guard),
            context_guard: None,
        };
        let mut journal = engine
            .prepare(
                ClientKind::Codex,
                "synthetic",
                &install,
                vec![FileChange {
                    path: path.display().to_string(),
                    before: Some(b"original".to_vec()),
                    after: Some(b"changed".to_vec()),
                }],
                None,
                vec![],
            )
            .unwrap();
        engine
            .freeze_target(
                &mut journal,
                Some("synthetic-version".into()),
                Some("synthetic.bundle".into()),
            )
            .unwrap();
        engine.freeze_policy(&mut journal, &"a".repeat(64)).unwrap();
        assert!(engine.decrypt(&journal).is_ok());
        for field in ["policy", "downgrade", "version", "bundle"] {
            let mut changed = journal.clone();
            match field {
                "policy" => changed.target_policy_revision = Some("b".repeat(64)),
                "downgrade" => changed.target_policy_revision = None,
                "version" => changed.target_version = Some("other".into()),
                "bundle" => changed.target_bundle_id = Some("other".into()),
                _ => unreachable!(),
            }
            assert_eq!(
                engine.decrypt(&changed).err().unwrap().code,
                "backup_invalid"
            );
        }
        assert_eq!(fs::read(path).unwrap(), b"original");
    }
    #[test]
    fn encrypted_multi_file_recovery_stops_on_external_modification() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("owned")).unwrap();
        let vault = FakeVault::default();
        let installation = Installation::create("https://panel.test", &vault).unwrap();
        let a = dir.path().join("a.toml");
        let b = dir.path().join("b.json");
        fs::write(&a, b"secret-original").unwrap();
        fs::write(&b, b"original-b").unwrap();
        let engine = ConfigEngine {
            store: &store,
            vault: &vault,
            allowed_roots: vec![dir.path().to_path_buf()],
            policy_guard: None,
            context_guard: None,
        };
        let j = engine
            .prepare(
                ClientKind::Codex,
                "a",
                &installation,
                vec![
                    FileChange {
                        path: a.display().to_string(),
                        before: Some(b"secret-original".to_vec()),
                        after: Some(b"new-a".to_vec()),
                    },
                    FileChange {
                        path: b.display().to_string(),
                        before: Some(b"original-b".to_vec()),
                        after: Some(b"new-b".to_vec()),
                    },
                ],
                None,
                vec![],
            )
            .unwrap();
        assert!(
            !serde_json::to_string(&j)
                .unwrap()
                .contains("secret-original")
        );
        let mut authorized = j.clone();
        authorized.phase = "publishing".into();
        store
            .put(
                "journal",
                &authorized.id,
                &authorized.account_id,
                &authorized,
            )
            .unwrap();
        fs::write(&a, b"new-a").unwrap();
        fs::write(&b, b"external-b").unwrap();
        assert_eq!(
            engine.recover(&j.id).unwrap_err().code,
            "configuration_conflict"
        );
        assert_eq!(fs::read(&a).unwrap(), b"new-a");
        assert_eq!(fs::read(&b).unwrap(), b"external-b");
    }
    #[test]
    fn restore_preserves_exact_original_and_toml_unowned_fields() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("owned")).unwrap();
        let vault = FakeVault::default();
        let install = Installation::create("https://panel.test", &vault).unwrap();
        let path = dir.path().join("config.toml");
        let original = b"# user comment\nweb_search = 'disabled'\n";
        fs::write(&path, original).unwrap();
        let patched = codex_patch(
            Some(original),
            "https://edge.test/v1",
            "model",
            Path::new("/fixture/orbit-helper"),
            &uuid(),
        )
        .unwrap();
        assert!(
            std::str::from_utf8(&patched)
                .unwrap()
                .contains("# user comment")
        );
        assert!(!std::str::from_utf8(&patched).unwrap().contains("api_key"));
        let engine = ConfigEngine {
            store: &store,
            vault: &vault,
            allowed_roots: vec![dir.path().to_path_buf()],
            policy_guard: None,
            context_guard: None,
        };
        let j = engine
            .prepare(
                ClientKind::Codex,
                "a",
                &install,
                vec![FileChange {
                    path: path.display().to_string(),
                    before: Some(original.to_vec()),
                    after: Some(patched),
                }],
                None,
                vec![],
            )
            .unwrap();
        engine.apply(&j.id).unwrap();
        let restore = engine.prepare_restore(&j.id).unwrap();
        engine.apply(&restore.id).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    fn grant(
        vault: &FakeVault,
        install: &Installation,
        name: &str,
        enabled: bool,
    ) -> HelperBinding {
        let binding = HelperBinding {
            id: uuid(),
            client: ClientKind::Codex,
            origin: install.origin.clone(),
            account_id: "a".into(),
            installation_id: install.id.clone(),
            provider_id: "openai".into(),
            credential_revision: name.into(),
            secret_reference: format!("fixture-{name}"),
            enabled,
            expires_at: None,
            authorization_epoch: 1,
        };
        vault
            .put(
                &binding.secret_reference,
                &serde_json::to_vec(&crate::identity::InferenceSecret {
                    origin: install.origin.clone(),
                    account_id: "a".into(),
                    installation_id: install.id.clone(),
                    provider_id: "openai".into(),
                    credential_revision: name.into(),
                    token: "orv_sk_public_synthetic".into(),
                    expires_at: None,
                    authorization_epoch: 1,
                })
                .unwrap(),
            )
            .unwrap();
        binding
    }
    #[test]
    fn restores_original_grant_without_reviving_logged_out_authorization() {
        for revoked in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let store = Store::open(dir.path()).unwrap();
            let vault = FakeVault::default();
            let install = Installation::create("https://panel.test", &vault).unwrap();
            let a = grant(&vault, &install, "key-a", true);
            let b = grant(&vault, &install, "key-b", false);
            store
                .set_meta(
                    &crate::identity::authorization_epoch_reference(
                        &install.origin,
                        &install.id,
                        "a",
                    ),
                    "1",
                )
                .unwrap();
            for binding in [&a, &b] {
                store.put("binding", &binding.id, "a", binding).unwrap();
            }
            let path = dir.path().join("config.toml");
            fs::write(&path, b"model='a'\n").unwrap();
            let engine = ConfigEngine {
                store: &store,
                vault: &vault,
                allowed_roots: vec![dir.path().to_path_buf()],
                policy_guard: None,
                context_guard: None,
            };
            let applied = engine
                .prepare(
                    ClientKind::Codex,
                    "a",
                    &install,
                    vec![FileChange {
                        path: path.display().to_string(),
                        before: Some(b"model='a'\n".to_vec()),
                        after: Some(b"model='b'\n".to_vec()),
                    }],
                    Some(b.id.clone()),
                    vec![a.id.clone()],
                )
                .unwrap();
            engine.apply(&applied.id).unwrap();
            assert!(
                !store
                    .get::<HelperBinding>("binding", &a.id)
                    .unwrap()
                    .unwrap()
                    .enabled
            );
            if revoked {
                vault.delete(&a.secret_reference).unwrap();
            }
            let restore = engine.prepare_restore(&applied.id).unwrap();
            engine.apply(&restore.id).unwrap();
            assert_eq!(fs::read(&path).unwrap(), b"model='a'\n");
            assert_eq!(
                store
                    .get::<HelperBinding>("binding", &a.id)
                    .unwrap()
                    .unwrap()
                    .enabled,
                !revoked
            );
            assert!(
                !store
                    .get::<HelperBinding>("binding", &b.id)
                    .unwrap()
                    .unwrap()
                    .enabled
            );
        }
    }
    #[test]
    fn prepared_preview_is_never_auto_published_by_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let vault = FakeVault::default();
        let install = Installation::create("https://panel.test", &vault).unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, b"model='original'\n").unwrap();
        let engine = ConfigEngine {
            store: &store,
            vault: &vault,
            allowed_roots: vec![dir.path().to_path_buf()],
            policy_guard: None,
            context_guard: None,
        };
        let preview = engine
            .prepare(
                ClientKind::Codex,
                "a",
                &install,
                vec![FileChange {
                    path: path.display().to_string(),
                    before: Some(b"model='original'\n".to_vec()),
                    after: Some(b"model='new'\n".to_vec()),
                }],
                None,
                vec![],
            )
            .unwrap();
        assert_eq!(
            engine.recover(&preview.id).unwrap_err().code,
            "preview_expired"
        );
        assert_eq!(fs::read(path).unwrap(), b"model='original'\n");
    }
    #[test]
    fn preview_diff_contains_actual_owned_changes_and_redacts_old_secrets() {
        let original=b"model='old'\n[model_providers.orivelle_orbit]\ncustom_unowned='keep'\nexperimental_bearer_token='original-secret'\n[model_providers.orivelle_orbit.auth]\ncwd='/user/owned'\n";
        let after = codex_patch(
            Some(original),
            "https://edge.test/v1",
            "new",
            Path::new("/private/orbit/bin/orbit-credential-helper"),
            &uuid(),
        )
        .unwrap();
        let diff = codex_redacted_diff(Some(original), &after).unwrap();
        assert!(diff.contains("auth.command"));
        assert!(diff.contains("/private/orbit/bin/orbit-credential-helper"));
        assert!(diff.contains("auth.args"));
        assert!(diff.contains("experimental_bearer_token"));
        assert!(!diff.contains("original-secret"));
        let after = std::str::from_utf8(&after).unwrap();
        assert!(after.contains("custom_unowned"));
        assert!(after.contains("cwd"));
    }
}
