use super::*;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ManualTarget {
    Codex(CodexConfigurationTarget),
    Claude(ClaudeConfigurationTarget),
}

impl ManualTarget {
    fn client(&self) -> ClientKind {
        match self {
            Self::Codex(_) => ClientKind::Codex,
            Self::Claude(_) => ClientKind::Claude,
        }
    }

    fn root(&self) -> &Path {
        match self {
            Self::Codex(target) => &target.config_root,
            Self::Claude(target) => &target.config_root,
        }
    }

    fn validate_root(&self) -> Result<()> {
        match self {
            Self::Codex(target) => target.validate_root(),
            Self::Claude(target) => target.validate_root(),
        }
    }

    fn index_path(&self) -> PathBuf {
        self.root().join("configLibrary/_meta.json")
    }

    fn paths(&self, index: Option<&[u8]>) -> Result<Vec<PathBuf>> {
        let root = self.root();
        Ok(match self {
            Self::Codex(_) => vec![
                root.join("orivelle-orbit.models.json"),
                root.join("config.toml"),
            ],
            Self::Claude(_) => {
                let mut paths = Vec::new();
                if let Some(index) = index {
                    #[derive(Deserialize)]
                    #[serde(rename_all = "camelCase")]
                    struct Index {
                        applied_id: String,
                    }
                    let index: Index = serde_json::from_slice(index)
                        .map_err(|_| fail("claude_configuration_source_unsupported"))?;
                    let id = uuid::Uuid::parse_str(&index.applied_id)
                        .map_err(|_| fail("claude_configuration_source_unsupported"))?;
                    if id.to_string() != index.applied_id {
                        return Err(fail("claude_configuration_source_unsupported"));
                    }
                    paths.push(root.join("configLibrary").join(format!("{id}.json")));
                }
                paths.extend([self.index_path(), root.join("claude_desktop_config.json")]);
                paths
            }
        })
    }

    pub(super) fn ensure_root(&self, boundary: &dyn Fn() -> Result<()>) -> Result<()> {
        match self {
            Self::Codex(target) => target.ensure_root(),
            Self::Claude(target) => target.ensure_root(boundary),
        }
    }
}

impl ConfigEngine<'_> {
    pub(crate) fn capture_manual(
        &self,
        installation: &Installation,
        account_id: &str,
        account_label: &str,
        target: ManualTarget,
        policy_revision: &str,
    ) -> Result<()> {
        let _lock = self.lock(target.client())?;
        self.check_policy()?;
        target.validate_root()?;
        let index = if matches!(target, ManualTarget::Claude(_)) {
            read_file(&target.index_path())?
        } else {
            None
        };
        let files = target
            .paths(index.as_deref())?
            .into_iter()
            .map(|path| {
                let after = read_file(&path)?;
                Ok(FileChange {
                    path: path.to_str().ok_or_else(|| fail("invalid_path"))?.into(),
                    before: None,
                    after,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if files.iter().all(|file| file.after.is_none()) {
            return Err(fail("configuration_unavailable"));
        }
        let now = chrono::Utc::now();
        let retention = self.store.settings()?.backup_retention_days.min(30);
        let mut journal = Journal {
            id: uuid(),
            client: target.client(),
            account_id: account_id.into(),
            account_label: account_label.into(),
            installation: installation.clone(),
            created_at: now.to_rfc3339(),
            expires_at: (now + chrono::Duration::days(i64::from(retention))).to_rfc3339(),
            phase: "committed".into(),
            purpose: JournalPurpose::ManualBackup,
            paths: files.iter().map(|file| file.path.clone()).collect(),
            target_version: None,
            target_bundle_id: None,
            target_policy_revision: Some(policy_revision.into()),
            nonce: String::new(),
            ciphertext: String::new(),
        };
        let operation = Operation {
            files,
            enable_binding: None,
            disable_bindings: Vec::new(),
            grants_before: Vec::new(),
            grants_after: Vec::new(),
            restore: false,
            source_journal_id: None,
            source_journal_sha256: None,
            source_journal_phase: None,
            codex_context: None,
            claude_context: None,
            claude_main_preferences_restore_v1: None,
            manual_target: Some(target),
        };
        self.check_context(&journal, &operation)?;
        self.seal_operation(&mut journal, &operation)?;
        for file in &operation.files {
            if !equal_current(file, &file.after)? {
                return Err(fail("configuration_conflict"));
            }
        }
        self.store
            .put("backup", &journal.id, &journal.account_id, &journal)?;
        self.prune_backups_locked(journal.client)
    }

    pub(crate) fn prepare_manual_restore(
        &self,
        backup: &Journal,
        target: &ManualTarget,
        policy_revision: &str,
    ) -> Result<Journal> {
        self.check_policy()?;
        backup.require_unexpired()?;
        let mut operation = self.decrypt(backup)?;
        self.check_context(backup, &operation)?;
        if backup.purpose != JournalPurpose::ManualBackup
            || operation.manual_target.as_ref() != Some(target)
        {
            return Err(fail("configuration_conflict"));
        }
        for file in &mut operation.files {
            file.before = read_file(Path::new(&file.path))?;
        }
        operation.restore = true;
        operation.source_journal_id = Some(backup.id.clone());
        operation.source_journal_sha256 = Some(Self::source_journal_sha256(backup)?);
        let mut journal = backup.clone();
        journal.id = uuid();
        journal.phase = "prepared".into();
        journal.purpose = JournalPurpose::ManualRestore;
        journal.target_policy_revision = Some(policy_revision.into());
        self.check_context(&journal, &operation)?;
        self.persist_operation(&mut journal, &operation)?;
        Ok(journal)
    }

    pub(crate) fn manual_target(&self, journal: &Journal) -> Result<ManualTarget> {
        self.decrypt(journal)?
            .manual_target
            .clone()
            .ok_or_else(|| fail("backup_invalid"))
    }

    pub(super) fn check_manual_context(
        &self,
        journal: &Journal,
        operation: &Operation,
        target: &ManualTarget,
    ) -> Result<()> {
        target.validate_root()?;
        let restore = journal.purpose == JournalPurpose::ManualRestore;
        let index = operation
            .files
            .iter()
            .find(|file| Path::new(&file.path) == target.index_path())
            .and_then(|file| file.after.as_deref());
        let paths = target.paths(index)?;
        if (!restore && journal.purpose != JournalPurpose::ManualBackup)
            || operation.restore != restore
            || journal.client != target.client()
            || operation.codex_context.is_some()
            || operation.claude_context.is_some()
            || operation.enable_binding.is_some()
            || !operation.disable_bindings.is_empty()
            || !operation.grants_before.is_empty()
            || !operation.grants_after.is_empty()
            || operation.claude_main_preferences_restore_v1.is_some()
            || operation.files.len() != paths.len()
            || journal
                .paths
                .iter()
                .map(Path::new)
                .ne(paths.iter().map(PathBuf::as_path))
            || !self.allowed_roots.iter().any(|root| {
                root == target.root()
                    || root
                        .canonicalize()
                        .is_ok_and(|root| target.root().starts_with(root))
            })
        {
            return Err(fail("backup_invalid"));
        }
        for (file, path) in operation.files.iter().zip(paths) {
            if Path::new(&file.path) != path
                || fs::symlink_metadata(&path)
                    .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err(fail("configuration_conflict"));
            }
        }
        if index.is_some()
            && operation
                .files
                .first()
                .is_none_or(|file| file.after.is_none())
        {
            return Err(fail("backup_invalid"));
        }
        if restore && journal.phase == "prepared" {
            let id = operation
                .source_journal_id
                .as_deref()
                .ok_or_else(|| fail("backup_invalid"))?;
            let source = self
                .store
                .get::<Journal>("backup", id)?
                .ok_or_else(|| fail("backup_invalid"))?;
            source.require_unexpired()?;
            if source.purpose != JournalPurpose::ManualBackup
                || source.phase != "committed"
                || operation.source_journal_sha256.as_deref()
                    != Some(Self::source_journal_sha256(&source)?.as_str())
            {
                return Err(fail("backup_invalid"));
            }
        }
        Ok(())
    }
}
