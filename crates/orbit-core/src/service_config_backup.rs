use super::*;
use crate::config::ManualTarget;

impl Service {
    fn manual_target(observation: &ConfigObservation) -> Result<ManualTarget> {
        match observation.target.client {
            ClientKind::Codex => observation
                .configuration_target
                .clone()
                .map(ManualTarget::Codex),
            ClientKind::Claude => observation
                .claude_configuration_target
                .clone()
                .map(ManualTarget::Claude),
        }
        .ok_or_else(|| fail("configuration_unavailable"))
    }

    fn validate_manual_target(
        &self,
        client: ClientKind,
        policy: &crate::policy::Snapshot,
        codex: Option<&CodexConfigurationTarget>,
        claude: Option<&ClaudeConfigurationTarget>,
    ) -> Result<()> {
        match client {
            ClientKind::Codex => self.validate_configuration(client, policy, codex),
            ClientKind::Claude => self.validate_claude_configuration(policy, claude),
        }
    }

    fn manual_engine<'a>(&'a self, guard: &'a dyn Fn() -> Result<()>) -> ConfigEngine<'a> {
        ConfigEngine {
            policy_guard: Some(guard),
            ..self.config_engine()
        }
    }

    pub fn backup_client_config(&self, client: ClientKind) -> Result<BootstrapView> {
        let _operation = self.begin_mutation()?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        self.require_original_configuration(client)?;
        if self.pending_configuration_recovery(client)? {
            return Err(fail("configuration_recovery_required"));
        }
        let observation = self.observe_configuration(client)?;
        self.require_configuration(&observation, None, None)?;
        let record = self
            .active_record()?
            .ok_or_else(|| fail("connection_unavailable"))?;
        let installation = self.installation(&record)?;
        self.ensure_installation_available(&installation.id)?;
        let guard = || {
            self.validate_manual_target(
                client,
                &observation.policy,
                observation.configuration_target.as_ref(),
                observation.claude_configuration_target.as_ref(),
            )
        };
        self.manual_engine(&guard).capture_manual(
            &installation,
            &record.view.account_id,
            &record.view.label,
            Self::manual_target(&observation)?,
            &observation.policy.revision,
        )?;
        self.changed();
        self.bootstrap()
    }

    pub(super) fn preview_manual_restore(&self, backup: Journal) -> Result<ConfigPreview> {
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        self.require_original_configuration(backup.client)?;
        if self.pending_configuration_recovery(backup.client)? {
            return Err(fail("configuration_recovery_required"));
        }
        self.require_backup_installation(&backup)?;
        let observation = self.observe_configuration(backup.client)?;
        self.require_configuration(&observation, None, None)?;
        let guard = || {
            self.validate_manual_target(
                backup.client,
                &observation.policy,
                observation.configuration_target.as_ref(),
                observation.claude_configuration_target.as_ref(),
            )
        };
        let journal = self.manual_engine(&guard).prepare_manual_restore(
            &backup,
            &Self::manual_target(&observation)?,
            &observation.policy.revision,
        )?;
        let source_expiry = chrono::DateTime::parse_from_rfc3339(&backup.expires_at)
            .map_err(|_| fail("backup_invalid"))?;
        let view = ConfigPreview {
            id: uuid(),
            client: backup.client,
            operation: "restore".into(),
            target_version: observation.target.version.clone(),
            paths: backup.paths,
            model_id: None,
            context: None,
            restore_owner: Some(RestoreOwner {
                account_id: backup.account_id,
                account_label: backup.account_label,
                panel_origin: backup.installation.origin,
                installation_id: backup.installation.id,
            }),
            restore_source: Some(RestoreSource {
                backup_group_id: backup.id,
                expires_at: backup.expires_at,
                kind: Some(BackupKind::Manual),
            }),
            restore_coordination: None,
            impact: vec!["用手动备份替换预览中的配置文件。确认前的再次编辑会使预览失效；恢复后请重启目标工具。".into()],
            redacted_diff: "Restore the selected encrypted files.".into(),
            expires_at: (chrono::Utc::now() + chrono::Duration::minutes(5))
                .min(source_expiry.with_timezone(&chrono::Utc)).to_rfc3339(),
            can_apply: true,
            can_export_import: false,
            blocking_reason: None,
            codex_catalog: None,
            codex_runtime: observation.runtime_view.clone(),
            codex_configuration_target: configuration_target_view(&observation),
            claude_configuration_target: claude::target_view(&observation),
        };
        self.previews
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .insert(
                view.id.clone(),
                PreviewEntry {
                    view: view.clone(),
                    journal_id: Some(journal.id),
                    import_candidate: None,
                    policy: observation.policy,
                    configuration_target: observation.configuration_target,
                    claude_configuration_target: observation.claude_configuration_target,
                    claude_diagnostic_scope: None,
                    claude_context: None,
                    catalog_context: None,
                    _mapping_reservation: None,
                },
            );
        Ok(view)
    }

    fn require_backup_installation(&self, backup: &Journal) -> Result<()> {
        if self
            .store
            .installation(&backup.installation.origin)?
            .is_none_or(|current| current.id != backup.installation.id)
        {
            return Err(fail("backup_invalid"));
        }
        self.ensure_installation_available(&backup.installation.id)
    }

    pub(super) fn apply_manual_restore(&self, entry: PreviewEntry) -> Result<ConfigResult> {
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let client = entry.view.client;
        self.require_original_configuration(client)?;
        if self.pending_configuration_recovery(client)? {
            return Err(fail("configuration_recovery_required"));
        }
        let guard = || {
            self.validate_manual_target(
                client,
                &entry.policy,
                entry.configuration_target.as_ref(),
                entry.claude_configuration_target.as_ref(),
            )
        };
        guard()?;
        let id = entry
            .journal_id
            .as_deref()
            .ok_or_else(|| fail("preview_expired"))?;
        let journal: Journal = self
            .store
            .get("journal", id)?
            .ok_or_else(|| fail("preview_expired"))?;
        self.require_backup_installation(&journal)?;
        self.manual_engine(&guard).apply(id)?;
        self.project_manual_restore(client, true)?;
        self.changed();
        Ok(ConfigResult {
            client,
            state: "restored".into(),
            loaded_state: "restart_required".into(),
            backup_group_id: None,
        })
    }

    pub(super) fn recover_manual_restore(&self, journal: &Journal) -> Result<()> {
        let observation = self.observe_configuration(journal.client)?;
        if self
            .require_configuration(&observation, None, None)
            .is_err()
        {
            return Ok(());
        }
        let guard = || {
            self.validate_manual_target(
                journal.client,
                &observation.policy,
                observation.configuration_target.as_ref(),
                observation.claude_configuration_target.as_ref(),
            )
        };
        let engine = self.manual_engine(&guard);
        let recovered = journal.phase != "conflict"
            && journal.target_policy_revision.as_deref() == Some(&observation.policy.revision)
            && self.require_backup_installation(journal).is_ok()
            && engine.manual_target(journal).is_ok_and(|target| {
                Self::manual_target(&observation).is_ok_and(|current| current == target)
            })
            && engine.recover(&journal.id).is_ok();
        self.project_manual_restore(journal.client, recovered)
    }

    fn project_manual_restore(&self, kind: ClientKind, recovered: bool) -> Result<()> {
        let revision = if recovered {
            match kind {
                ClientKind::Codex => codex_configuration_revision(
                    &self.paths.codex_config,
                    &self.paths.codex_catalog(),
                )?,
                ClientKind::Claude => self.claude_revision()?,
            }
        } else {
            None
        };
        let mut clients = self.clients.lock().map_err(|_| fail("state_unavailable"))?;
        if let Some(client) = clients.iter_mut().find(|client| client.client == kind) {
            client.configuration_state = if recovered {
                "not_configured"
            } else {
                "conflict"
            }
            .into();
            client.configuration_revision = revision;
            client.loaded_state = if recovered {
                "restart_required"
            } else {
                "unconfirmed"
            }
            .into();
            client.loaded_evidence = None;
            client.connection_evidence = None;
            client.connection_state = "not_tested".into();
            self.store
                .put("client-state", kind.as_str(), "local", client)?;
        }
        Ok(())
    }
}
