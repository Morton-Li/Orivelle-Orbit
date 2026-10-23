use super::*;
use crate::config::{ClaudeActiveSource, ClaudeContext, ClaudeFileDigest, ClaudeModelInput};

pub(super) struct Plan<'a> {
    pub record: &'a ConnectionRecord,
    pub cache: &'a AccountCache,
    pub compiled: &'a crate::claude_config::CompiledLibrary,
    pub observation: &'a ConfigObservation,
    pub binding_id: &'a str,
    pub selected_model_id: &'a str,
}

pub(super) fn access_revision(target: &str, policy: &str) -> String {
    sha256(format!("claude-configuration-access-v1\n{target}\n{policy}\n").as_bytes())
}

pub(super) fn target_view(
    observation: &ConfigObservation,
) -> Option<ClaudeConfigurationTargetView> {
    observation.policy.require_user_config_clear().ok()?;
    let target = observation.claude_configuration_target.as_ref()?;
    Some(ClaudeConfigurationTargetView {
        app_version: target.app_version.clone(),
        revision: access_revision(&target.revision, &observation.policy.local_revision),
    })
}

fn scope_revision(context: &ClaudeContext) -> Result<String> {
    let mut frozen = context.clone();
    frozen.scope_revision.clear();
    Ok(sha256(
        &serde_json::to_vec(&frozen).map_err(|_| fail("serialization_failed"))?,
    ))
}

impl Service {
    pub(super) fn ensure_no_other_claude_recovery(&self, own_journal_id: &str) -> Result<()> {
        for journal in self.store.list::<Journal>("journal", None)? {
            if journal.client == ClientKind::Claude
                && journal.id != own_journal_id
                && (["publishing", "conflict"].contains(&journal.phase.as_str())
                    || journal.phase == "committed"
                        && self.store.get::<Journal>("backup", &journal.id)?.is_none())
            {
                return Err(fail("configuration_recovery_required"));
            }
        }
        Ok(())
    }

    fn claude_engine<'a>(
        &'a self,
        policy: &'a dyn Fn() -> Result<()>,
        context: &'a dyn Fn(&ClaudeContext, bool) -> Result<()>,
    ) -> ConfigEngine<'a> {
        ConfigEngine {
            store: &self.store,
            vault: self.vault.as_ref(),
            allowed_roots: self.paths.configuration_roots.clone(),
            policy_guard: Some(policy),
            context_guard: None,
            claude_context_guard: Some(context),
        }
    }

    fn claude_source(&self) -> Result<(Option<Vec<u8>>, Option<(String, Vec<u8>)>)> {
        let metadata = read_file(&self.paths.claude_library.join("_meta.json"))?;
        let active = metadata
            .as_ref()
            .map(|bytes| {
                let value: Value = serde_json::from_slice(bytes)
                    .map_err(|_| fail("claude_configuration_source_unsupported"))?;
                let id = value
                    .get("appliedId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| fail("claude_configuration_source_unsupported"))?;
                validate_uuid(id).map_err(|_| fail("claude_configuration_source_unsupported"))?;
                if uuid::Uuid::parse_str(id)
                    .map_err(|_| fail("claude_configuration_source_unsupported"))?
                    .to_string()
                    != id
                {
                    return Err(fail("claude_configuration_source_unsupported"));
                }
                let bytes = read_file(&self.paths.claude_library.join(format!("{id}.json")))?
                    .ok_or_else(|| fail("claude_configuration_source_unsupported"))?;
                Ok((id.to_owned(), bytes))
            })
            .transpose()?;
        Ok((metadata, active))
    }

    pub(super) fn claude_revision(&self) -> Result<Option<String>> {
        let (metadata, active) = self.claude_source()?;
        let main = read_file(
            &self
                .paths
                .claude_library
                .parent()
                .ok_or_else(|| fail("invalid_path"))?
                .join("claude_desktop_config.json"),
        )?;
        if metadata.is_none() && main.is_none() {
            return Ok(None);
        }
        Ok(Some(sha256(
            &serde_json::to_vec(&(
                metadata.as_ref().map(|bytes| sha256(bytes)),
                active.as_ref().map(|(id, bytes)| (id, sha256(bytes))),
                main.as_ref().map(|bytes| sha256(bytes)),
            ))
            .map_err(|_| fail("serialization_failed"))?,
        )))
    }

    pub(super) fn preview_claude_config(
        &self,
        id: &str,
        revision: u64,
        model_id: &str,
    ) -> Result<ConfigPreview> {
        let _operation = self.begin_mutation()?;
        let observation = self.observe_configuration(ClientKind::Claude)?;
        let gate_error = self
            .require_claude_configuration(&observation, None, None)
            .err();
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let (record, cache) = self.current_connection(id, revision)?;
        let model = cache
            .models
            .iter()
            .find(|model| {
                model.id == model_id
                    && model.provider_id == record.view.provider_id
                    && model.category == "text"
            })
            .ok_or_else(|| fail("model_unavailable"))?;
        let mut view = ConfigPreview {
            id: uuid(), client: ClientKind::Claude, operation: "apply".into(),
            target_version: observation.target.version.clone(), paths: Vec::new(),
            model_id: Some(model_id.into()), context: Some(self.context(&record, &cache, model)?), restore_owner: None, restore_source: None,
            impact: vec![
                "创建新的 Orivelle 配置，依次保存 profile、配置库索引和第三方模式；影响 Claude 的 Chat、Code 与 Cowork。".into(),
                "保留原配置与未知索引字段；保存后需重启，尚未证明客户端加载或连通。".into(),
                "首次目录仅在确认后创建为用户私有目录；原字节可整组恢复，空目录可保留。".into(),
            ],
            redacted_diff: "No file mutation is permitted for this preview.".into(),
            expires_at: (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339(),
            can_apply: false, can_export_import: false, blocking_reason: None,
            codex_catalog: None, codex_runtime: None, codex_configuration_target: None,
            claude_configuration_target: target_view(&observation),
        };
        let mut journal_id = None;
        let mut frozen = None;
        if let Some(error) = gate_error {
            view.blocking_reason = Some(error.code);
        } else if self.pending_configuration_recovery(ClientKind::Claude)? {
            view.blocking_reason = Some("configuration_recovery_required".into());
        } else if !self.paths.helper_available() {
            view.blocking_reason = Some("helper_unavailable".into());
        } else if !model
            .capabilities
            .iter()
            .any(|cap| cap == "anthropic.messages")
        {
            view.blocking_reason = Some("model_unsupported".into());
        } else {
            let target = observation
                .claude_configuration_target
                .as_ref()
                .ok_or_else(|| fail("claude_configuration_target_unverified"))?;
            let (metadata, active) = self.claude_source()?;
            let main = read_file(&target.config_root.join("claude_desktop_config.json"))?;
            let profile_id = uuid();
            let binding_id = uuid();
            // Compile the complete, bounded source before credentials or grants.
            let compiled = crate::claude_config::compile(&crate::claude_config::LibraryPlan {
                app_version: &target.app_version,
                source_member_sha256: &target.source_member_sha256,
                user_root: &target.config_root,
                profile_id: &profile_id,
                gateway_base_url: cache
                    .claude_endpoints
                    .get(
                        record
                            .view
                            .node_id
                            .as_ref()
                            .ok_or_else(|| fail("node_unavailable"))?,
                    )
                    .ok_or_else(|| fail("node_unavailable"))?,
                helper: &self.paths.helper,
                binding_id: &binding_id,
                model_id,
                profile_before: read_file(
                    &self.paths.claude_library.join(format!("{profile_id}.json")),
                )?
                .as_deref(),
                metadata_before: metadata.as_deref(),
                main_before: main.as_deref(),
                active_profile: active
                    .as_ref()
                    .map(|(id, bytes)| crate::claude_config::ActiveProfile { id, bytes }),
            })?;
            let context = self.freeze_claude(Plan {
                record: &record,
                cache: &cache,
                compiled: &compiled,
                observation: &observation,
                binding_id: &binding_id,
                selected_model_id: model_id,
            })?;
            view.paths = compiled
                .files()
                .iter()
                .map(|file| file.path.clone())
                .collect();
            // Only owned values enter the preview; unknown metadata/main/profile
            // fields and every original credential remain solely in the journal.
            view.redacted_diff = serde_json::to_string_pretty(&json!({
                "profile": {"id": profile_id, "inferenceProvider": "gateway",
                    "inferenceGatewayBaseUrl": context.claude_base_url,
                    "inferenceCredentialKind": "helper-script",
                    "inferenceGatewayAuthScheme": "bearer",
                    "inferenceCredentialHelper": context.helper_path,
                    "inferenceCredentialHelperArgs": ["claude", binding_id],
                    "inferenceModels": [{"name": model_id}], "modelDiscoveryEnabled": false},
                "metadata": {"appliedId": profile_id, "appendEntry": "Orivelle Orbit"},
                "main": {"deploymentMode": "3p"},
            }))
            .map_err(|_| fail("serialization_failed"))?;
            let installation = self.installation(&record)?;
            let binding = HelperBinding {
                id: binding_id.clone(),
                client: ClientKind::Claude,
                origin: context.panel_origin.clone(),
                account_id: context.account_id.clone(),
                installation_id: context.installation_id.clone(),
                provider_id: context.provider_id.clone(),
                credential_revision: context.credential_revision.clone(),
                secret_reference: context.credential_reference.clone(),
                enabled: false,
                expires_at: context.credential_expires_at,
                authorization_epoch: context.authorization_epoch,
            };
            let guard = || self.validate_claude_configuration(&observation.policy, Some(target));
            let context_guard = |current: &ClaudeContext, restoring: bool| {
                if current != &context || restoring {
                    return Err(fail("claude_configuration_context_unverified"));
                }
                self.validate_claude_context(current, false)
            };
            let mut checked = binding.clone();
            checked.enabled = true;
            self.validate_claude_context(&context, false)?;
            let _ = bound_helper_token(
                &checked,
                ClientKind::Claude,
                &binding_id,
                &self.store,
                self.vault.as_ref(),
                chrono::Utc::now().timestamp(),
            )?;
            self.validate_claude_context(&context, false)?;
            self.store
                .put("binding", &binding_id, &context.account_id, &binding)?;
            let previous = self.claude_previous_bindings(&context)?;
            let mut journal = self.claude_engine(&guard, &context_guard).prepare_claude(
                &context.account_id,
                &installation,
                compiled.into_files(),
                previous,
                context.clone(),
            )?;
            journal.account_label = cache.account.label.clone();
            self.validate_claude_context(&context, false)?;
            self.store
                .put("journal", &journal.id, &journal.account_id, &journal)?;
            journal_id = Some(journal.id);
            view.can_apply = true;
            frozen = Some(context);
        }
        self.previews
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .insert(
                view.id.clone(),
                PreviewEntry {
                    view: view.clone(),
                    journal_id,
                    import_candidate: None,
                    policy: observation.policy,
                    configuration_target: None,
                    claude_configuration_target: observation.claude_configuration_target,
                    claude_diagnostic_scope: None,
                    claude_context: frozen,
                    catalog_context: None,
                },
            );
        Ok(view)
    }

    pub(super) fn apply_claude_entry(
        &self,
        entry: PreviewEntry,
        restore: bool,
    ) -> Result<ConfigResult> {
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let frozen = entry
            .claude_context
            .as_ref()
            .ok_or_else(|| fail("claude_configuration_context_unverified"))?;
        let guard = || {
            self.validate_claude_configuration(
                &entry.policy,
                entry.claude_configuration_target.as_ref(),
            )?;
            if !restore {
                self.ensure_no_other_claude_recovery(
                    entry
                        .journal_id
                        .as_deref()
                        .ok_or_else(|| fail("preview_expired"))?,
                )?;
            }
            Ok(())
        };
        let context_guard = |current: &ClaudeContext, restoring: bool| {
            if current != frozen || restoring != restore {
                return Err(fail("claude_configuration_context_unverified"));
            }
            self.validate_claude_context(current, restoring)
        };
        guard()?;
        self.validate_claude_context(frozen, restore)?;
        let journal = self.claude_engine(&guard, &context_guard).apply(
            entry
                .journal_id
                .as_deref()
                .ok_or_else(|| fail("preview_expired"))?,
        )?;
        self.project_claude_operation(restore, true)?;
        self.changed();
        Ok(ConfigResult {
            client: ClientKind::Claude,
            state: if restore { "restored" } else { "saved" }.into(),
            loaded_state: "restart_required".into(),
            backup_group_id: Some(journal.id),
        })
    }

    fn project_claude_operation(&self, restoring: bool, recovered: bool) -> Result<()> {
        let revision = if recovered {
            self.claude_revision()?
        } else {
            None
        };
        for client in self
            .clients
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .iter_mut()
            .filter(|client| client.client == ClientKind::Claude)
        {
            client.configuration_state = if !recovered {
                "conflict"
            } else if restoring {
                "not_configured"
            } else {
                "saved"
            }
            .into();
            if recovered {
                client.configuration_revision = revision.clone();
            }
            client.loaded_state = if recovered {
                "restart_required"
            } else {
                "unconfirmed"
            }
            .into();
            client.loaded_evidence = None;
            client.connection_evidence = None;
            client.connection_state = "not_tested".into();
            client.manual_steps.push(
                if !recovered {
                    "整组配置操作尚待恢复；请先协调外部修改，同客户端新应用已阻止。"
                } else if restoring {
                    "配置原字节已恢复；已退出、撤销、删除或失效的旧 helper 授权不会复活。"
                } else {
                    "三文件配置已保存；需重启，尚未证明客户端加载或连通。"
                }
                .into(),
            );
            self.store
                .put("client-state", client.client.as_str(), "local", client)?;
        }
        Ok(())
    }

    pub(super) fn preview_claude_restore(
        &self,
        backup_id: &str,
        backup: Journal,
    ) -> Result<ConfigPreview> {
        let observation = self.observe_configuration(ClientKind::Claude)?;
        let blocked = self
            .require_claude_configuration(&observation, None, None)
            .err()
            .map(|error| error.code);
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let guard = || {
            self.validate_claude_configuration(
                &observation.policy,
                observation.claude_configuration_target.as_ref(),
            )
        };
        let context_guard =
            |context: &ClaudeContext, _: bool| self.validate_claude_context(context, true);
        let engine = self.claude_engine(&guard, &context_guard);
        let source_expiry = chrono::DateTime::parse_from_rfc3339(&backup.expires_at)
            .map_err(|_| fail("backup_invalid"))?;
        let expires_at = (chrono::Utc::now() + chrono::Duration::minutes(5))
            .min(source_expiry.with_timezone(&chrono::Utc))
            .to_rfc3339();
        let mut frozen = None;
        let mut paths = backup.paths.clone();
        paths.reverse();
        let journal_id = if blocked.is_none() {
            let (context, restoring) = engine.claude_operation(&backup)?;
            if restoring {
                return Err(fail("backup_invalid"));
            }
            let journal = engine.prepare_restore(backup_id)?;
            paths = journal.paths.clone();
            frozen = Some(context);
            Some(journal.id)
        } else {
            None
        };
        let view = ConfigPreview {
            id: uuid(), client: ClientKind::Claude, operation: "restore".into(),
            target_version: observation.target.version.clone(), paths, model_id: None, context: None,
            restore_source: Some(RestoreSource { backup_group_id: backup_id.into(), expires_at: backup.expires_at.clone() }),
            restore_owner: Some(RestoreOwner { account_id: backup.account_id.clone(),
                account_label: if backup.account_label.is_empty() { backup.account_id } else { backup.account_label },
                panel_origin: backup.installation.origin, installation_id: backup.installation.id }),
            impact: vec!["依次恢复主配置、配置库索引和新 profile 的原字节；客户端仍需重启。".into(),
                "恢复不依赖当前账户会话、key、节点或 helper；外部修改时整组停止，失效授权不会复活。".into()],
            redacted_diff: "Restore encrypted original configuration; original secrets stay redacted.".into(),
            expires_at,
            can_apply: blocked.is_none(), can_export_import: false, blocking_reason: blocked,
            codex_catalog: None, codex_runtime: None, codex_configuration_target: None,
            claude_configuration_target: target_view(&observation),
        };
        self.previews
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .insert(
                view.id.clone(),
                PreviewEntry {
                    view: view.clone(),
                    journal_id,
                    import_candidate: None,
                    policy: observation.policy,
                    configuration_target: None,
                    claude_configuration_target: observation.claude_configuration_target,
                    claude_diagnostic_scope: None,
                    claude_context: frozen,
                    catalog_context: None,
                },
            );
        Ok(view)
    }

    pub(super) fn coordinate_claude_journal(&self, journal: &Journal) -> Result<()> {
        // A newer restore can settle its older source while iterating the
        // startup list. Never recover or project status from that stale row.
        let Some(current) = self.store.get::<Journal>("journal", &journal.id)? else {
            return Ok(());
        };
        let journal = &current;
        if journal.client != ClientKind::Claude
            || !["publishing", "conflict", "committed"].contains(&journal.phase.as_str())
            || journal.phase == "committed"
                && self.store.get::<Journal>("backup", &journal.id)?.is_some()
        {
            return Ok(());
        }
        let observation = self.observe_configuration(ClientKind::Claude)?;
        // Preflight is read-only and precedes the authenticated journal's Vault access.
        if self
            .require_claude_configuration(&observation, None, None)
            .is_err()
        {
            return Ok(());
        }
        let qualified = observation.target.version == journal.target_version
            && observation.target.bundle_id == journal.target_bundle_id
            && journal.target_policy_revision.as_deref()
                == Some(observation.policy.revision.as_str())
            && self
                .store
                .installation(&journal.installation.origin)?
                .is_some_and(|i| i.id == journal.installation.id)
            && self
                .ensure_installation_available(&journal.installation.id)
                .is_ok();
        let guard = || {
            self.validate_claude_configuration(
                &observation.policy,
                observation.claude_configuration_target.as_ref(),
            )
        };
        let context_guard = |context: &ClaudeContext, restoring: bool| {
            self.validate_claude_context(context, restoring)
        };
        let engine = self.claude_engine(&guard, &context_guard);
        let operation = if qualified {
            engine.claude_operation(journal).ok()
        } else {
            None
        };
        let restored = operation.as_ref().is_some_and(|(_, restored)| *restored);
        let recovered = operation.is_some()
            && journal.phase != "conflict"
            && engine.recover(&journal.id).is_ok();
        guard()?;
        self.project_claude_operation(restored, recovered)
    }

    /// Diagnostic export has its own destination fence. Resolve the existing
    /// parent without creating directories, then keep that fixed destination.
    pub(super) fn claude_diagnostic_destination(&self, path: &Path) -> Result<PathBuf> {
        if !path.is_absolute()
            || !path.components().all(|part| {
                matches!(
                    part,
                    std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                        | std::path::Component::Normal(_)
                )
            })
        {
            return Err(fail("invalid_path"));
        }
        let parent_leaf = |path: &Path| {
            path.parent()?
                .canonicalize()
                .ok()
                .map(|parent| parent.join(path.file_name().unwrap_or_default()))
        };
        let destination = parent_leaf(path).ok_or_else(|| fail("invalid_path"))?;
        let same_physical = |left: &Path, right: &Path| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                match (std::fs::metadata(left), std::fs::metadata(right)) {
                    (Ok(left), Ok(right)) => left.dev() == right.dev() && left.ino() == right.ino(),
                    _ => false,
                }
            }
            #[cfg(not(unix))]
            {
                left.canonicalize()
                    .ok()
                    .zip(right.canonicalize().ok())
                    .is_some_and(|(left, right)| left == right)
            }
        };
        if !destination.parent().is_some_and(|parent| parent.is_dir())
            || path.file_name().is_none()
            || std::fs::symlink_metadata(&destination)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(fail("invalid_path"));
        }
        let claude_root = self
            .paths
            .claude_library
            .parent()
            .ok_or_else(|| fail("invalid_path"))?;
        let actual_claude_root = claude_root
            .canonicalize()
            .ok()
            .or_else(|| parent_leaf(claude_root))
            .ok_or_else(|| fail("invalid_path"))?;
        if destination.starts_with(actual_claude_root)
            || destination.parent().is_some_and(|parent| {
                parent
                    .ancestors()
                    .any(|ancestor| same_physical(ancestor, claude_root))
            })
            || [self.paths.codex_config.clone(), self.paths.codex_catalog()]
                .iter()
                .any(|protected| {
                    path == protected
                        || parent_leaf(protected).as_ref() == Some(&destination)
                        || protected.canonicalize().ok().as_ref() == Some(&destination)
                        || same_physical(protected, &destination)
                        || protected.parent().zip(destination.parent()).is_some_and(
                            |(protected_parent, destination_parent)| {
                                same_physical(protected_parent, destination_parent)
                                    && protected
                                        .file_name()
                                        .and_then(|name| name.to_str())
                                        .zip(destination.file_name().and_then(|name| name.to_str()))
                                        .is_some_and(|(protected_name, destination_name)| {
                                            protected_name.eq_ignore_ascii_case(destination_name)
                                        })
                            },
                        )
                })
            || self.paths.isolated
                && !destination.parent().is_some_and(|parent| {
                    parent
                        .ancestors()
                        .any(|ancestor| same_physical(ancestor, &self.store.root))
                })
        {
            return Err(fail("invalid_path"));
        }
        Ok(destination)
    }

    pub(super) fn require_claude_configuration(
        &self,
        observation: &ConfigObservation,
        expected_policy: Option<&crate::policy::Snapshot>,
        expected_target: Option<&ClaudeConfigurationTarget>,
    ) -> Result<()> {
        if !self.paths.writes_authorized {
            return Err(fail("integration_authorization_required"));
        }
        let target = observation
            .claude_configuration_target
            .as_ref()
            .ok_or_else(|| fail("claude_configuration_target_unverified"))?;
        target.validate_root()?;
        if observation.target.client != ClientKind::Claude
            || !observation.target.installed
            || !observation.target.platform_supported
            || observation.target.version.as_deref() != Some(target.app_version.as_str())
            || observation.target.bundle_id.as_deref() != Some(target.bundle_id.as_str())
            || observation.target.app_path.as_deref() != Some(target.app_path.as_str())
            || self.paths.claude_library != target.config_root.join("configLibrary")
        {
            return Err(fail("claude_configuration_target_unverified"));
        }
        if expected_target.is_some_and(|expected| expected != target) {
            return Err(fail("claude_configuration_target_changed"));
        }
        if let Some(expected) = expected_policy {
            expected.matches_user_config(&observation.policy)
        } else {
            observation.policy.require_user_config_clear()
        }
    }

    pub(super) fn validate_claude_configuration(
        &self,
        policy: &crate::policy::Snapshot,
        target: Option<&ClaudeConfigurationTarget>,
    ) -> Result<()> {
        // observe_configuration calls the full producer afresh. Its signature
        // result is never inferred from a display hint or validate_root alone.
        self.require_claude_configuration(
            &self.observe_configuration(ClientKind::Claude)?,
            Some(policy),
            target,
        )
    }

    fn claude_inputs(
        &self,
        record: &ConnectionRecord,
        cache: &AccountCache,
    ) -> Vec<ClaudeModelInput> {
        cache
            .models
            .iter()
            .filter(|model| {
                model.provider_id == record.view.provider_id
                    && model.category == "text"
                    && model
                        .capabilities
                        .iter()
                        .any(|capability| capability == "anthropic.messages")
            })
            .map(|model| ClaudeModelInput {
                id: model.id.clone(),
                provider_id: model.provider_id.clone(),
                revision: model.revision.clone(),
                category: model.category.clone(),
                capabilities: model.capabilities.clone(),
                support_status: model.support_status.clone(),
                reasoning: model.reasoning.clone(),
            })
            .collect()
    }

    pub(super) fn claude_diagnostic_scope(
        &self,
        id: &str,
        revision: u64,
        selected: &str,
    ) -> Result<String> {
        let (record, cache) = self.current_connection(id, revision)?;
        let epoch = self
            .store
            .meta(&authorization_epoch_reference(
                &record.view.panel_origin,
                &record.installation_id,
                &record.view.account_id,
            ))?
            .and_then(|value| value.parse::<u64>().ok());
        let node = cache
            .nodes
            .iter()
            .find(|node| Some(&node.id) == record.view.node_id.as_ref() && node.available)
            .ok_or_else(|| fail("node_unavailable"))?;
        let endpoint = cache
            .claude_endpoints
            .get(&node.id)
            .ok_or_else(|| fail("node_unavailable"))?;
        let models = self.claude_inputs(&record, &cache);
        let model = models
            .iter()
            .find(|model| model.id == selected)
            .ok_or_else(|| fail("model_unavailable"))?;
        let key = cache
            .keys
            .iter()
            .find(|key| {
                key.provider_id == record.view.provider_id
                    && key.api_key_id == record.api_key_id
                    && key.key_revision == record.view.credential_revision
            })
            .ok_or_else(|| fail("credential_missing"))?;
        if epoch != Some(record.authorization_epoch)
            || key.state != "active"
            || key.expires_at != record.credential_expires_at
            || key
                .expires_at
                .is_some_and(|at| at <= chrono::Utc::now().timestamp())
        {
            return Err(fail("claude_configuration_context_changed"));
        }
        Ok(sha256(
            &serde_json::to_vec(&(
                &record,
                epoch,
                node,
                endpoint,
                &cache.catalog_revision,
                &models,
                &model.id,
                &model.revision,
                key,
            ))
            .map_err(|_| fail("serialization_failed"))?,
        ))
    }

    pub(super) fn freeze_claude(&self, plan: Plan<'_>) -> Result<ClaudeContext> {
        let Plan {
            record,
            cache,
            compiled,
            observation,
            binding_id,
            selected_model_id,
        } = plan;
        let target = observation
            .claude_configuration_target
            .as_ref()
            .ok_or_else(|| fail("claude_configuration_target_unverified"))?;
        self.validate_claude_configuration(&observation.policy, Some(target))?;
        let node = cache
            .nodes
            .iter()
            .find(|node| Some(&node.id) == record.view.node_id.as_ref() && node.available)
            .ok_or_else(|| fail("node_unavailable"))?;
        let endpoint = cache
            .claude_endpoints
            .get(&node.id)
            .ok_or_else(|| fail("node_unavailable"))?;
        let models = self.claude_inputs(record, cache);
        let model = models
            .iter()
            .find(|model| model.id == selected_model_id)
            .ok_or_else(|| fail("model_unsupported"))?;
        let profile_id = compiled
            .files()
            .first()
            .and_then(|file| Path::new(&file.path).file_stem())
            .and_then(|id| id.to_str())
            .ok_or_else(|| fail("invalid_configuration"))?
            .to_owned();
        let active_source = compiled
            .active_profile_id()
            .map(|id| {
                let path = target
                    .config_root
                    .join("configLibrary")
                    .join(format!("{id}.json"));
                let bytes = read_file(&path)?.ok_or_else(|| fail("configuration_conflict"))?;
                let hash = sha256(&bytes);
                if compiled.active_profile_sha256() != Some(hash.as_str()) {
                    return Err(fail("configuration_conflict"));
                }
                Ok(ClaudeActiveSource {
                    profile_id: id.into(),
                    path: path.to_str().ok_or_else(|| fail("invalid_path"))?.into(),
                    sha256: hash,
                    size: bytes.len() as u64,
                })
            })
            .transpose()?;
        let mut context = ClaudeContext {
            contract_version: 1,
            panel_origin: record.view.panel_origin.clone(),
            account_id: record.view.account_id.clone(),
            installation_id: record.installation_id.clone(),
            connection_id: record.view.id.clone(),
            connection_revision: record.view.revision,
            authorization_epoch: record.authorization_epoch,
            credential_revision: record
                .view
                .credential_revision
                .clone()
                .ok_or_else(|| fail("credential_missing"))?,
            credential_reference: record
                .credential_reference
                .clone()
                .ok_or_else(|| fail("credential_missing"))?,
            credential_expires_at: record.credential_expires_at,
            api_key_id: record.api_key_id.clone(),
            node_id: node.id.clone(),
            node_revision: node.revision.clone(),
            claude_base_url: endpoint.clone(),
            provider_id: record.view.provider_id.clone(),
            catalog_revision: cache.catalog_revision.clone(),
            selected_model_id: model.id.clone(),
            selected_model_revision: model.revision.clone(),
            models,
            scope_revision: String::new(),
            configuration_target: target.clone(),
            policy_revision: observation.policy.revision.clone(),
            local_policy_revision: observation.policy.local_revision.clone(),
            profile_id,
            binding_id: binding_id.into(),
            helper_path: self
                .paths
                .helper
                .to_str()
                .ok_or_else(|| fail("invalid_path"))?
                .into(),
            files: compiled
                .files()
                .iter()
                .map(ClaudeFileDigest::from_change)
                .collect::<Result<_>>()?,
            active_source,
        };
        context.scope_revision = scope_revision(&context)?;
        self.validate_claude_context(&context, false)?;
        Ok(context)
    }

    pub(super) fn validate_claude_context(
        &self,
        context: &ClaudeContext,
        restoring: bool,
    ) -> Result<()> {
        self.validate_claude_authority(context, restoring, true)
    }

    pub(super) fn validate_claude_refresh_context(&self, context: &ClaudeContext) -> Result<()> {
        // Bootstrap and models have independent catalog revisions. The live
        // actor/key/node must already match; compare the complete model scope
        // only after the models endpoint has refreshed it, before any apply.
        self.validate_claude_authority(context, false, false)
    }

    fn validate_claude_authority(
        &self,
        context: &ClaudeContext,
        restoring: bool,
        compare_catalog: bool,
    ) -> Result<()> {
        let observation = self.observe_configuration(ClientKind::Claude)?;
        self.require_claude_configuration(&observation, None, Some(&context.configuration_target))?;
        if context.contract_version != 1
            || scope_revision(context)? != context.scope_revision
            || self.paths.claude_library
                != context
                    .configuration_target
                    .config_root
                    .join("configLibrary")
            || observation.policy.revision != context.policy_revision
            || observation.policy.local_revision != context.local_policy_revision
        {
            return Err(fail("claude_configuration_context_changed"));
        }
        let installation = self
            .store
            .installation(&context.panel_origin)?
            .ok_or_else(|| fail("account_scope_mismatch"))?;
        if installation.id != context.installation_id || installation.origin != context.panel_origin
        {
            return Err(fail("account_scope_mismatch"));
        }
        self.ensure_installation_available(&installation.id)?;
        // Original-byte restoration deliberately has no live actor, session,
        // credential, node, model or current-helper prerequisite.
        if restoring {
            return Ok(());
        }
        if Path::new(&context.helper_path) != self.paths.helper || !self.paths.helper_available() {
            return Err(fail("helper_unavailable"));
        }
        let (record, cache) = self
            .current_connection(&context.connection_id, context.connection_revision)
            .map_err(|_| fail("claude_configuration_context_changed"))?;
        let epoch = self
            .store
            .meta(&authorization_epoch_reference(
                &context.panel_origin,
                &context.installation_id,
                &context.account_id,
            ))?
            .and_then(|value| value.parse::<u64>().ok());
        let node = cache
            .nodes
            .iter()
            .find(|node| node.id == context.node_id && node.available);
        let key = cache.keys.iter().find(|key| {
            key.provider_id == context.provider_id
                && key.api_key_id == context.api_key_id
                && key.key_revision.as_ref() == Some(&context.credential_revision)
        });
        if record.view.panel_origin != context.panel_origin
            || record.view.account_id != context.account_id
            || record.installation_id != context.installation_id
            || record.authorization_epoch != context.authorization_epoch
            || epoch != Some(context.authorization_epoch)
            || cache.account.id != context.account_id
            || record.view.credential_revision.as_ref() != Some(&context.credential_revision)
            || record.credential_reference.as_ref() != Some(&context.credential_reference)
            || record.credential_expires_at != context.credential_expires_at
            || record.api_key_id != context.api_key_id
            || record.view.node_id.as_ref() != Some(&context.node_id)
            || record.view.provider_id != context.provider_id
            || node.is_none_or(|node| node.revision != context.node_revision)
            || cache.claude_endpoints.get(&context.node_id) != Some(&context.claude_base_url)
            || compare_catalog
                && (cache.catalog_revision != context.catalog_revision
                    || self.claude_inputs(&record, &cache) != context.models
                    || !context.models.iter().any(|model| {
                        model.id == context.selected_model_id
                            && model.revision == context.selected_model_revision
                    }))
            || key.is_none_or(|key| {
                key.state != "active"
                    || key.expires_at != context.credential_expires_at
                    || key
                        .expires_at
                        .is_some_and(|at| at <= chrono::Utc::now().timestamp())
            })
        {
            return Err(fail("claude_configuration_context_changed"));
        }
        Ok(())
    }

    pub(super) fn claude_previous_bindings(&self, context: &ClaudeContext) -> Result<Vec<String>> {
        Ok(self
            .store
            .list::<HelperBinding>("binding", Some(&context.account_id))?
            .into_iter()
            .filter(|binding| {
                binding.client == ClientKind::Claude
                    && binding.enabled
                    && binding.origin == context.panel_origin
                    && binding.account_id == context.account_id
                    && binding.installation_id == context.installation_id
            })
            .map(|binding| binding.id)
            .collect())
    }
}
