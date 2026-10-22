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
            || cache.catalog_revision != context.catalog_revision
            || self.claude_inputs(&record, &cache) != context.models
            || !context.models.iter().any(|model| {
                model.id == context.selected_model_id
                    && model.revision == context.selected_model_revision
            })
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
