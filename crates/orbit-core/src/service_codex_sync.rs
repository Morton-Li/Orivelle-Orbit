use super::*;

impl Service {
    /// Native background polling also runs while the main window is hidden.
    /// Only an existing confirmed Codex switch authorizes configuration writes.
    pub async fn synchronize_models(&self) -> Result<()> {
        let _operation = self.begin_mutation()?;
        let serial = self.network_lock("model-sync")?;
        let Ok(_serial) = serial.try_lock() else {
            return Ok(());
        };
        let Some(id) = self.store.meta("active_connection")? else {
            return Ok(());
        };
        let record = self.record(&id)?;
        if record.session_status != "authenticated" || record.view.node_id.is_none() {
            return Ok(());
        }
        let result: Result<()> = async {
            self.refresh_account_cache(&id, false).await?;
            self.list_models(&id).await?;
            Ok(())
        }
        .await;
        if let Err(error) = &result {
            self.note_catalog_sync("blocked", 0, Some(error.code.clone()));
        }
        result
    }

    fn note_catalog_sync(&self, state: &str, count: usize, error: Option<String>) {
        if let Ok(mut clients) = self.clients.lock() {
            if let Some(client) = clients
                .iter_mut()
                .find(|client| client.client == ClientKind::Codex)
            {
                client.catalog_sync = Some(CatalogSyncView {
                    state: state.into(),
                    checked_at: chrono::Utc::now().to_rfc3339(),
                    model_count: count,
                    error,
                });
                if state == "updated" {
                    client.configuration_state = "saved".into();
                    client.configuration_revision = codex_configuration_revision(
                        &self.paths.codex_config,
                        &self.paths.codex_catalog(),
                    )
                    .ok()
                    .flatten();
                    client.loaded_state = "restart_required".into();
                    client.loaded_evidence = None;
                    client.connection_evidence = None;
                    client.connection_state = "not_tested".into();
                }
                let _ = self.store.put("client-state", "codex", "local", client);
            }
        }
        self.changed();
    }

    pub(super) fn sync_codex_models_if_bound(&self, id: &str) {
        let result = self.sync_codex_models(id);
        match result {
            Ok(Some((updated, count))) => {
                self.note_catalog_sync(if updated { "updated" } else { "current" }, count, None)
            }
            Err(error) => self.note_catalog_sync("blocked", 0, Some(error.code)),
            Ok(None) => {}
        }
    }

    pub(super) fn sync_codex_models(&self, id: &str) -> Result<Option<(bool, usize)>> {
        let _operation = self.begin_mutation()?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        if self.store.meta("active_connection")?.as_deref() != Some(id)
            || !self.paths.writes_authorized
        {
            return Ok(None);
        }
        let sources: Vec<_> = self
            .store
            .list::<Journal>("journal", None)?
            .into_iter()
            .filter(|journal| {
                journal.client == ClientKind::Codex
                    && journal.purpose == JournalPurpose::Switch
                    && journal.phase == "committed"
            })
            .collect();
        let Some(source) = sources.first() else {
            return Ok(None);
        };
        if sources.len() != 1 || self.pending_configuration_recovery(ClientKind::Codex)? {
            return Err(fail("configuration_recovery_required"));
        }
        let record = self.record(id)?;
        if source.account_id != record.view.account_id
            || source.installation.id != record.installation_id
            || source.installation.origin != record.view.panel_origin
        {
            return Ok(None);
        }
        let observation = self.observe_configuration(ClientKind::Codex)?;
        self.require_configuration(&observation, None, None)?;
        let guard = || {
            self.validate_configuration(
                ClientKind::Codex,
                &observation.policy,
                observation.configuration_target.as_ref(),
            )
        };
        let context_guard =
            |context: Option<&CodexContext>, restore| self.validate_codex_context(context, restore);
        let engine = self.codex_engine(&guard, &context_guard);
        let (previous, binding_id) = engine.codex_switch_context(source)?;
        if previous.connection_id != id
            || record.view.node_id.as_ref() != Some(&previous.node_id)
            || record.view.edge_base_url.as_ref() != Some(&previous.edge_base_url)
        {
            return Err(fail("codex_catalog_scope_changed"));
        }
        // A later default Key selection must never silently rebind a client.
        let record = if self
            .store
            .get::<api_keys::KeyCredential>("api-key-credential", &previous.credential_reference)?
            .is_some()
        {
            self.frozen_api_key_record(
                &record,
                &previous.credential_reference,
                &previous.credential_revision,
            )?
        } else {
            record
        };
        let cache = self.account_cache(&record)?;
        let binding: HelperBinding = self
            .store
            .get("binding", &binding_id)?
            .ok_or_else(|| fail("binding_disabled"))?;
        if !binding.enabled
            || binding.secret_reference != previous.credential_reference
            || binding.credential_revision != previous.credential_revision
        {
            return Err(fail("binding_disabled"));
        }
        let _ = bound_helper_token(
            &binding,
            ClientKind::Codex,
            &binding_id,
            &self.store,
            self.vault.as_ref(),
            chrono::Utc::now().timestamp(),
        )?;
        let planned = match self.plan_codex(&record, &cache, &previous.default_alias) {
            Ok(planned) => planned,
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "codex_binding_unavailable"
                        | "codex_reasoning_unavailable"
                        | "codex_catalog_reasoning_invalid"
                        | "codex_models_unavailable"
                        | "codex_model_id_unsupported"
                ) =>
            {
                self.plan_codex(&record, &cache, "")?
            }
            Err(error) => return Err(error),
        };
        let target = observation
            .configuration_target
            .as_ref()
            .ok_or_else(|| fail("codex_configuration_target_unverified"))?;
        let context = self.freeze_codex(&record, &cache, &planned, target)?;
        let before = read_file(&self.paths.codex_config)?;
        let config = codex_catalog_patch(
            before.as_deref(),
            &previous.edge_base_url,
            planned.catalog().default_alias(),
            &self.paths.helper,
            &binding_id,
            &self.paths.codex_catalog(),
            planned.default_reasoning_effort(),
        )?;
        let files = vec![
            FileChange {
                path: self.paths.codex_catalog().display().to_string(),
                before: read_file(&self.paths.codex_catalog())?,
                after: Some(planned.catalog().bytes().to_vec()),
            },
            FileChange {
                path: self.paths.codex_config.display().to_string(),
                before,
                after: Some(config),
            },
        ];
        // Validate source bytes even on a no-op: an external platform switch or
        // edit must be reported without being overwritten or adopted as owned.
        let changed = files.iter().any(|file| file.before != file.after);
        if !changed
            && previous.models == context.models
            && previous.catalog_revision == context.catalog_revision
        {
            // prepare_codex_sync also checks the authenticated source bytes;
            // the lightweight source check avoids creating journals on a poll.
            engine.verify_codex_switch_files(source)?;
            return Ok(Some((false, planned.catalog().main_aliases().len())));
        }
        let mut journal = engine.prepare_codex_sync(source, files, context)?;
        engine.freeze_target(
            &mut journal,
            observation.target.version.clone(),
            observation.target.bundle_id.clone(),
        )?;
        engine.freeze_policy(&mut journal, &observation.policy.revision)?;
        engine.apply(&journal.id)?;
        Ok(Some((changed, planned.catalog().main_aliases().len())))
    }
}
