use super::*;
use crate::config::{ClaudeActiveSource, ClaudeContext, ClaudeFileDigest, ClaudeModelInput};

const MAPPING_UNAVAILABLE_NOTE: &str = "本机模型映射未就绪；请保持 Orbit 运行，检查账户、系统安全存储及原端口占用后重新应用接入。保存状态不代表 Claude 已载入或连接通过。";
pub(super) const PREFERENCES_RESTORED_NOTE: &str = "上次协调恢复已还原接入配置并保留后续偏好；当前载入与连接仍待确认，失效的旧 helper 授权不会复活。";
const EXACT_RESTORED_NOTE: &str =
    "配置原字节已恢复；已退出、撤销、删除或失效的旧 helper 授权不会复活。";
const SAVED_NOTE: &str = "三文件配置已保存；需重启，尚未证明客户端加载或连通。";
const PENDING_NOTE: &str = "整组配置操作尚待恢复；请先协调外部修改，同客户端新应用已阻止。";

pub(super) struct Plan<'a> {
    pub record: &'a ConnectionRecord,
    pub cache: &'a AccountCache,
    pub compiled: &'a crate::claude_config::CompiledLibrary,
    pub observation: &'a ConfigObservation,
    pub binding_id: &'a str,
    pub selected_model_id: &'a str,
}

struct MappingAuthority {
    service: std::sync::Weak<Service>,
    record: crate::claude_mapping::MappingRecord,
}

impl crate::claude_mapping::MappingAuthority for MappingAuthority {
    fn authenticate(&self, bearer: &[u8]) -> Result<crate::claude_mapping::MappingAccess> {
        let service = self
            .service
            .upgrade()
            .ok_or_else(|| fail("binding_disabled"))?;
        let lease = service.begin_mutation()?;
        service.validate_running_claude_mapping(&self.record)?;
        crate::claude_mapping::authenticate_local(
            &self.record,
            bearer,
            &service.store,
            service.vault.as_ref(),
        )?;
        service.validate_running_claude_mapping(&self.record)?;
        Ok(crate::claude_mapping::MappingAccess {
            binding_id: self.record.context.binding_id.clone(),
            route_revision: self.record.plan()?.revision()?,
            _lease: Box::new(lease),
        })
    }
    fn authorize_messages(
        &self,
        access: crate::claude_mapping::MappingAccess,
        alias: String,
    ) -> Result<crate::claude_mapping::ForwardPermit> {
        let service = self
            .service
            .upgrade()
            .ok_or_else(|| fail("binding_disabled"))?;
        if access.binding_id != self.record.context.binding_id
            || access.route_revision != self.record.plan()?.revision()?
        {
            return Err(fail("binding_disabled"));
        }
        service.validate_running_claude_mapping(&self.record)?;
        let binding = crate::claude_mapping::mapping_binding(&self.record, &service.store)?;
        let key = bound_helper_token(
            &binding,
            ClientKind::Claude,
            &binding.id,
            &service.store,
            service.vault.as_ref(),
            chrono::Utc::now().timestamp(),
        )?;
        service.validate_running_claude_mapping(&self.record)?;
        let messages_url = service.http.validate_url(&format!(
            "{}/v1/messages",
            self.record.context.claude_base_url.trim_end_matches('/')
        ))?;
        Ok(crate::claude_mapping::ForwardPermit {
            access,
            messages_url,
            upstream_model_id: self
                .record
                .plan()?
                .upstream_model(&alias)
                .ok_or_else(|| fail("binding_disabled"))?
                .into(),
            requested_alias: alias,
            api_key: key,
        })
    }
    fn before_send(
        &self,
        permit: crate::claude_mapping::ForwardPermit,
    ) -> Result<crate::claude_mapping::ForwardPermit> {
        let service = self
            .service
            .upgrade()
            .ok_or_else(|| fail("binding_disabled"))?;
        let expected_url = service.http.validate_url(&format!(
            "{}/v1/messages",
            self.record.context.claude_base_url.trim_end_matches('/')
        ))?;
        if permit.access.binding_id != self.record.context.binding_id
            || permit.access.route_revision != self.record.plan()?.revision()?
            || self.record.plan()?.upstream_model(&permit.requested_alias)
                != Some(permit.upstream_model_id.as_str())
            || permit.messages_url != expected_url
        {
            return Err(fail("binding_disabled"));
        }
        service.validate_running_claude_mapping(&self.record)?;
        Ok(permit)
    }
    fn observe(
        &self,
        permit: crate::claude_mapping::ForwardPermit,
        outcome: crate::claude_mapping::ForwardOutcome,
    ) -> Result<()> {
        let service = self
            .service
            .upgrade()
            .ok_or_else(|| fail("binding_disabled"))?;
        let (state, status) = match outcome {
            crate::claude_mapping::ForwardOutcome::Complete { status } => {
                ("complete", Some(status))
            }
            crate::claude_mapping::ForwardOutcome::Unconfirmed => ("unconfirmed", None),
            crate::claude_mapping::ForwardOutcome::NotSent => ("not_sent", None),
        };
        // A transport receipt is not proof of the caller process, Desktop load,
        // model equivalence, usage or settlement. Do not project client success.
        service.store.put("claude-mapping-outcome",&permit.access.binding_id,&self.record.context.account_id,
            &json!({"state":state,"http_status":status,"observed_at":chrono::Utc::now().to_rfc3339()}))
    }
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
    fn validate_running_claude_mapping(
        &self,
        expected: &crate::claude_mapping::MappingRecord,
    ) -> Result<()> {
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let current = self
            .store
            .get::<crate::claude_mapping::MappingRecord>(
                "claude-mapping",
                &expected.context.binding_id,
            )?
            .ok_or_else(|| fail("binding_disabled"))?;
        if current.context != expected.context || current.journal_id != expected.journal_id {
            return Err(fail("binding_disabled"));
        }
        crate::claude_mapping::mapping_binding(&current, &self.store)?;
        let (client, policy) = self.client_policy(ClientKind::Claude)?;
        let revision = access_revision(
            &expected.context.configuration_target.revision,
            &policy.local_revision,
        );
        if client.configuration_access != ConfigurationAccess::Allowed
            || client.configuration_access_revision.as_deref() != Some(revision.as_str())
        {
            return Err(fail("claude_configuration_context_changed"));
        }
        self.validate_claude_authority_state(&expected.context, false, true, &policy)?;
        self.validate_claude_mapping_selection(&expected.context)
    }

    fn validate_claude_mapping_selection(&self, context: &ClaudeContext) -> Result<()> {
        // Startup authenticates this context before calling; a running
        // authority holds the already authenticated immutable snapshot.
        let plan = context
            .mapping
            .as_ref()
            .ok_or_else(|| fail("binding_disabled"))?;
        plan.validate()?;
        let root = self
            .paths
            .claude_library
            .parent()
            .ok_or_else(|| fail("invalid_path"))?;
        let paths = [
            self.paths
                .claude_library
                .join(format!("{}.json", plan.profile_id)),
            self.paths.claude_library.join("_meta.json"),
            root.join("claude_desktop_config.json"),
        ];
        if root != context.configuration_target.config_root
            || context.files.len() != 3
            || context
                .files
                .iter()
                .zip(&paths)
                .any(|(file, path)| Path::new(&file.path) != path)
        {
            return Err(fail("configuration_conflict"));
        }
        let (metadata, active) = self.claude_source()?;
        let (id, profile) = active.ok_or_else(|| fail("configuration_conflict"))?;
        if id != plan.profile_id
            || sha256(&profile) != context.files[0].after_sha256
            || profile.len() as u64 != context.files[0].after_size
        {
            return Err(fail("configuration_conflict"));
        }
        let metadata: Value =
            serde_json::from_slice(&metadata.ok_or_else(|| fail("configuration_conflict"))?)
                .map_err(|_| fail("configuration_conflict"))?;
        if metadata
            .as_object()
            .is_none_or(|object| object.contains_key("hybridPointer"))
            || metadata["appliedId"].as_str() != Some(plan.profile_id.as_str())
            || metadata["entries"].as_array().is_none_or(|entries| {
                entries
                    .iter()
                    .filter(|entry| entry["id"].as_str() == Some(plan.profile_id.as_str()))
                    .count()
                    != 1
            })
        {
            return Err(fail("configuration_conflict"));
        }
        let main = read_file(&paths[2])?.ok_or_else(|| fail("configuration_conflict"))?;
        let main: Value =
            serde_json::from_slice(&main).map_err(|_| fail("configuration_conflict"))?;
        if main
            .as_object()
            .is_none_or(|object| object.contains_key("enterpriseConfig"))
            || main["deploymentMode"].as_str() != Some("3p")
        {
            return Err(fail("configuration_conflict"));
        }
        // Desktop-owned preferences and index labels are not inference
        // authority. Engine apply/restore still requires exact original bytes.
        Ok(())
    }

    pub(super) fn claude_mapping_authority(
        &self,
        record: crate::claude_mapping::MappingRecord,
    ) -> Arc<dyn crate::claude_mapping::MappingAuthority> {
        Arc::new(MappingAuthority {
            service: self.self_reference.clone(),
            record,
        })
    }

    pub(super) fn stop_claude_mappings_for(&self, account: &str, origin: &str) -> Result<()> {
        for record in self
            .store
            .list::<crate::claude_mapping::MappingRecord>("claude-mapping", None)?
        {
            if record.context.account_id == account && record.context.panel_origin == origin {
                self.claude_mappings.stop(&record.context.binding_id);
            }
        }
        Ok(())
    }

    pub fn stop_claude_mappings(&self) {
        self.claude_mappings.stop_running_except(None);
    }

    pub fn note_claude_mapping_unavailable(&self) -> Result<()> {
        self.set_claude_mapping_notice(true)
    }
    fn set_claude_mapping_notice(&self, unavailable: bool) -> Result<()> {
        let mut changed = false;
        {
            let mut clients = self.clients.lock().map_err(|_| fail("state_unavailable"))?;
            for client in clients
                .iter_mut()
                .filter(|client| client.client == ClientKind::Claude)
            {
                let had = client
                    .manual_steps
                    .iter()
                    .any(|note| note == MAPPING_UNAVAILABLE_NOTE);
                if had == unavailable {
                    continue;
                }
                if unavailable {
                    client.manual_steps.push(MAPPING_UNAVAILABLE_NOTE.into());
                } else {
                    client
                        .manual_steps
                        .retain(|note| note != MAPPING_UNAVAILABLE_NOTE);
                }
                self.store
                    .put("client-state", client.client.as_str(), "local", client)?;
                changed = true;
            }
        }
        if changed {
            self.changed();
        }
        Ok(())
    }

    pub fn reconcile_claude_mappings(&self) -> Result<()> {
        let _operation = self.begin_mutation()?;
        let result = (|| {
            let records = self
                .store
                .list::<crate::claude_mapping::MappingRecord>("claude-mapping", None)?;
            let mut enabled = false;
            for record in &records {
                if self
                    .store
                    .get::<HelperBinding>("binding", &record.context.binding_id)?
                    .is_some_and(|binding| binding.client == ClientKind::Claude && binding.enabled)
                {
                    enabled = true;
                    break;
                }
            }
            if !enabled {
                self.stop_claude_mappings();
                return Ok(true);
            }
            // Fixed user roots are qualified before reading their active index.
            // Ordinary mapping rows cannot supply a path-reading authority.
            let observation = self.observe_configuration(ClientKind::Claude)?;
            self.require_claude_configuration(&observation, None, None)?;
            let (_, active) = self.claude_source()?;
            let Some((active_id, _)) = active else {
                self.stop_claude_mappings();
                return Ok(true);
            };
            let record = records
                .into_iter()
                .find(|record| record.context.profile_id == active_id);
            let Some(mut record) = record else {
                self.stop_claude_mappings();
                return Ok(true);
            };
            self.claude_mappings
                .stop_running_except(Some(&record.context.binding_id));
            match crate::claude_mapping::mapping_binding(&record, &self.store) {
                Ok(_) => {}
                Err(error) if error.code == "binding_disabled" => {
                    self.claude_mappings.stop(&record.context.binding_id);
                    return Ok(false);
                }
                Err(error) => return Err(error),
            }
            self.validate_claude_context(&record.context, false)?;
            // The Keychain pin authenticates the complete frozen context, even
            // after its expired backup has legitimately been pruned.
            let _local = crate::claude_mapping::mapped_helper_token(
                &record.context.binding_id,
                &self.store,
                self.vault.as_ref(),
                chrono::Utc::now().timestamp(),
            )?;
            if let Some(journal) = self.store.get::<Journal>("journal", &record.journal_id)? {
                let observation = self.observe_configuration(ClientKind::Claude)?;
                let guard = || {
                    self.validate_claude_engine_policy(
                        &observation.policy,
                        observation.claude_configuration_target.as_ref(),
                    )
                };
                let scope = |context: &ClaudeContext, restoring: bool| {
                    self.validate_claude_context(context, restoring)
                };
                let (authenticated, restoring) = self
                    .claude_engine(&guard, &scope)
                    .claude_operation(&journal)?;
                if restoring || authenticated != record.context {
                    return Err(fail("backup_invalid"));
                }
            }
            self.validate_running_claude_mapping(&record)?;
            record.committed = true;
            self.store.put(
                "claude-mapping",
                &record.context.binding_id,
                &record.context.account_id,
                &record,
            )?;
            let authority = self.claude_mapping_authority(record.clone());
            self.claude_mappings.activate(
                record.plan()?,
                authority,
                self.http.clone(),
                crate::claude_mapping_http::start,
            )?;
            Ok(true)
        })();
        match result {
            Err(error) => {
                self.stop_claude_mappings();
                let _ = self.note_claude_mapping_unavailable();
                Err(error)
            }
            Ok(ready) => {
                self.cleanup_claude_mapping_records()?;
                self.set_claude_mapping_notice(!ready)?;
                Ok(())
            }
        }
    }

    fn cleanup_claude_mapping_records(&self) -> Result<()> {
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let journals = self.store.list::<Journal>("journal", None)?;
        let mut pending = false;
        for record in self
            .store
            .list::<crate::claude_mapping::MappingRecord>("claude-mapping", None)?
        {
            let Ok(deadline) = chrono::DateTime::parse_from_rfc3339(&record.retained_until) else {
                continue;
            };
            if deadline > chrono::Utc::now()
                || journals.iter().any(|journal| {
                    journal.client == ClientKind::Claude
                        && journal.account_id == record.context.account_id
                        && journal.installation.origin == record.context.panel_origin
                        && ["publishing", "conflict"].contains(&journal.phase.as_str())
                })
            {
                continue;
            }
            let mut binding = self
                .store
                .get::<HelperBinding>("binding", &record.context.binding_id)?;
            if binding.as_ref().is_some_and(|binding| binding.enabled) {
                continue;
            }
            // Keep the legacy-helper bypass closed while deleting a record.
            if let Some(binding) = binding.as_mut() {
                binding.enabled = false;
                self.store
                    .put("binding", &binding.id, &binding.account_id, binding)?;
            }
            let plan = record.plan()?;
            self.claude_mappings.stop(&plan.binding_id);
            match crate::claude_mapping::verify_context_secret(&record, self.vault.as_ref()) {
                Ok(crate::claude_mapping::LocalSecretPresence::Absent) => {}
                Ok(crate::claude_mapping::LocalSecretPresence::Authenticated) => {
                    if self.vault.delete(&plan.token_reference()).is_err() {
                        pending = true;
                        continue;
                    }
                }
                Err(_) => {
                    pending = true;
                    continue;
                }
            }
            self.store.delete("claude-mapping", &plan.binding_id)?;
            self.store
                .delete("claude-mapping-outcome", &plan.binding_id)?;
        }
        self.store.set_meta(
            "claude_mapping_cleanup_pending",
            if pending { "1" } else { "0" },
        )
    }
    pub(super) fn ensure_no_other_claude_recovery(&self, own_journal_id: &str) -> Result<()> {
        for journal in self.store.list::<Journal>("journal", None)? {
            if journal.client == ClientKind::Claude
                && journal.id != own_journal_id
                && journal.requires_recovery(&self.store)?
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
        roles: Option<&ClaudeRoleModels>,
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
        if let Some(roles) = roles {
            self.require_original_configuration(ClientKind::Claude)?;
            let eligible = self.claude_inputs(&record, &cache);
            if !roles.ids().into_iter().all(|id| {
                eligible
                    .iter()
                    .any(|model| model.id == id && model.support_status != "unsupported")
            }) {
                return Err(fail("model_unavailable"));
            }
        }
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
            model_id: Some(model_id.into()), context: Some(self.context(&record, &cache, model)?), restore_owner: None, restore_source: None, restore_coordination: None,
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
        let mut mapping_reservation = None;
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
            let mapping = if roles.is_some() || crate::claude_mapping::requires_mapping(model_id) {
                let plan =
                    self.claude_mappings
                        .reserve(&binding_id, &profile_id, model_id, roles)?;
                mapping_reservation = Some(self.claude_mappings.ticket(&plan));
                Some(plan)
            } else {
                None
            };
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
                mapping: mapping.as_ref(),
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
                    "inferenceGatewayBaseUrl": mapping.as_ref().map(|plan|plan.base_url()).unwrap_or_else(||context.claude_base_url.clone()),
                    "inferenceCredentialKind": "helper-script",
                    "inferenceGatewayAuthScheme": "bearer",
                    "inferenceCredentialHelper": context.helper_path,
                    "inferenceCredentialHelperArgs": [if mapping.is_some(){"claude-map"}else{"claude"}, binding_id],
                    "inferenceModels": mapping.as_ref().map(|plan|json!(plan.aliases)).unwrap_or_else(||json!([{"name": model_id}])), "modelDiscoveryEnabled": false},
                "localModelMapping": mapping.as_ref().map(|plan|json!({"roles":plan.aliases,"loopback":plan.base_url(),"requiresOrbitRunning":true})),
                "metadata": {"appliedId": profile_id, "appendEntry": "Orivelle Orbit"},
                "main": {"deploymentMode": "3p"},
            }))
            .map_err(|_| fail("serialization_failed"))?;
            if mapping.is_some() {
                if let Some(roles) = roles {
                    view.impact.push(format!(
                        "主模型 {}，备用主模型 {}，辅助模型 {}。",
                        roles.primary, roles.fallback, roles.auxiliary
                    ));
                } else {
                    view.impact.push(format!(
                        "Claude 主模型、备用主模型与辅助角色均映射至同一个获准模型 {model_id}。"
                    ));
                }
                view.impact.push(
                    "Claude Desktop 通过本机 Orbit 转发请求，使用时请保持 Orbit 运行。".into(),
                );
                view.impact.push("真正退出 Orbit 会中断本机转发；已发出的请求可能仍在服务端执行并计费，结果与费用需由 Panel 确认。".into());
            }
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
            let guard = || self.validate_claude_engine_policy(&observation.policy, Some(target));
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
            let previous_mapping_ids = previous.clone();
            let engine = self.claude_engine(&guard, &context_guard);
            let mut journal = engine.prepare_claude(
                &context.account_id,
                &installation,
                compiled.into_files(),
                previous,
                context.clone(),
            )?;
            if roles.is_some() {
                engine.mark_switch(&mut journal)?;
            }
            journal.account_label = cache.account.label.clone();
            self.validate_claude_context(&context, false)?;
            self.store
                .put("journal", &journal.id, &journal.account_id, &journal)?;
            if context.mapping.is_some() {
                let record = crate::claude_mapping::MappingRecord {
                    journal_id: journal.id.clone(),
                    context: context.clone(),
                    committed: false,
                    retained_until: journal.expires_at.clone(),
                };
                self.store
                    .put("claude-mapping", &binding_id, &context.account_id, &record)?;
            }
            // A newer restore source can reference an older mapping's helper.
            // Retain its local token at least as long as that encrypted source.
            for previous_id in previous_mapping_ids {
                if let Some(mut record) = self
                    .store
                    .get::<crate::claude_mapping::MappingRecord>("claude-mapping", &previous_id)?
                {
                    let current = chrono::DateTime::parse_from_rfc3339(&record.retained_until).ok();
                    let next = chrono::DateTime::parse_from_rfc3339(&journal.expires_at)
                        .map_err(|_| fail("backup_invalid"))?;
                    if current.is_none_or(|current| current < next) {
                        record.retained_until = journal.expires_at.clone();
                        self.store.put(
                            "claude-mapping",
                            &previous_id,
                            &record.context.account_id,
                            &record,
                        )?;
                    }
                }
            }
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
                    _mapping_reservation: mapping_reservation,
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
        self.check_switch_confirmation(&entry, restore)?;
        let frozen = entry
            .claude_context
            .as_ref()
            .ok_or_else(|| fail("claude_configuration_context_unverified"))?;
        // take_preview already accepted the explicit confirmation deadline.
        // Claim only the socket before slow full checks; it grants no write or
        // forwarding authority, and the entry ticket releases it on failure.
        if !restore && let Some(plan) = frozen.mapping.as_ref() {
            self.claude_mappings.claim(plan)?;
        }
        let guard = || {
            self.validate_claude_engine_policy(
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
        self.validate_claude_configuration(
            &entry.policy,
            entry.claude_configuration_target.as_ref(),
        )?;
        guard()?;
        self.validate_claude_context(frozen, restore)?;
        if !restore && let Some(plan) = frozen.mapping.as_ref() {
            plan.validate()?;
            let record = self
                .store
                .get::<crate::claude_mapping::MappingRecord>("claude-mapping", &frozen.binding_id)?
                .ok_or_else(|| fail("binding_disabled"))?;
            if record.context != *frozen
                || Some(record.journal_id.as_str()) != entry.journal_id.as_deref()
            {
                return Err(fail("binding_disabled"));
            }
            let binding = self
                .store
                .get::<HelperBinding>("binding", &frozen.binding_id)?
                .ok_or_else(|| fail("binding_disabled"))?;
            crate::claude_mapping::ensure_local_secret(&record, &binding, self.vault.as_ref())?;
            self.validate_claude_context(frozen, false)?;
            guard()?;
        }
        let (journal, preserved_preferences) = self
            .claude_engine(&guard, &context_guard)
            .apply_claude_with_restore_mode(
                entry
                    .journal_id
                    .as_deref()
                    .ok_or_else(|| fail("preview_expired"))?,
                entry.view.restore_coordination.is_some(),
            )?;
        self.project_claude_operation(restore, true, preserved_preferences)?;
        if !restore && frozen.mapping.is_some() {
            let mut record = self
                .store
                .get::<crate::claude_mapping::MappingRecord>("claude-mapping", &frozen.binding_id)?
                .ok_or_else(|| fail("binding_disabled"))?;
            record.committed = true;
            self.store.put(
                "claude-mapping",
                &frozen.binding_id,
                &frozen.account_id,
                &record,
            )?;
        }
        drop(_gate);
        let mapping_result = self.reconcile_claude_mappings();
        self.changed();
        // Restoring authenticated original bytes deliberately has no live
        // actor, model or listener prerequisite. Its runtime status is separate.
        if !restore {
            mapping_result?;
        }
        Ok(ConfigResult {
            client: ClientKind::Claude,
            state: if restore { "restored" } else { "saved" }.into(),
            loaded_state: "restart_required".into(),
            backup_group_id: Some(journal.id),
        })
    }

    fn project_claude_operation(
        &self,
        restoring: bool,
        recovered: bool,
        preserved_preferences: bool,
    ) -> Result<()> {
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
            client.manual_steps.retain(|note| {
                ![
                    PENDING_NOTE,
                    EXACT_RESTORED_NOTE,
                    PREFERENCES_RESTORED_NOTE,
                    SAVED_NOTE,
                ]
                .contains(&note.as_str())
            });
            client.manual_steps.push(
                if !recovered {
                    PENDING_NOTE
                } else if restoring && preserved_preferences {
                    PREFERENCES_RESTORED_NOTE
                } else if restoring {
                    EXACT_RESTORED_NOTE
                } else {
                    SAVED_NOTE
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
        coordination: Option<ClaudeRestoreCoordination>,
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
            self.validate_claude_engine_policy(
                &observation.policy,
                observation.claude_configuration_target.as_ref(),
            )
        };
        let context_guard =
            |context: &ClaudeContext, _: bool| self.validate_claude_context(context, true);
        let engine = self.claude_engine(&guard, &context_guard);
        let source_expiry = chrono::DateTime::parse_from_rfc3339(&backup.expires_at)
            .map_err(|_| fail("backup_invalid"))?;
        let preview_deadline = chrono::Utc::now() + chrono::Duration::minutes(5);
        let expires_at = if backup.purpose == JournalPurpose::Switch {
            preview_deadline
        } else {
            preview_deadline.min(source_expiry.with_timezone(&chrono::Utc))
        }
        .to_rfc3339();
        let mut frozen = None;
        let mut original_mode = None;
        let mut paths = backup.paths.clone();
        paths.reverse();
        let journal_id = if blocked.is_none() {
            let (journal, context) = if coordination.is_some() {
                let (journal, context, mode) =
                    engine.prepare_claude_preferences_restore_details(backup_id)?;
                original_mode = Some(mode);
                (journal, context)
            } else {
                let (context, restoring) = engine.claude_operation(&backup)?;
                if restoring {
                    return Err(fail("backup_invalid"));
                }
                (engine.prepare_restore(backup_id)?, context)
            };
            // Preparing the encrypted journal can prompt for its backup key.
            // A display hint cannot authorize returning an applicable preview.
            self.validate_claude_context(&context, true)?;
            paths = journal.paths.clone();
            frozen = Some(context);
            Some(journal.id)
        } else {
            None
        };
        let view = ConfigPreview {
            id: uuid(),
            client: ClientKind::Claude,
            operation: "restore".into(),
            target_version: observation.target.version.clone(),
            paths,
            model_id: None,
            context: None,
            restore_source: Some(RestoreSource {
                backup_group_id: backup_id.into(),
                expires_at: backup.expires_at.clone(),
                kind: backup.backup_kind(),
            }),
            restore_coordination: coordination,
            restore_owner: Some(RestoreOwner {
                account_id: backup.account_id.clone(),
                account_label: if backup.account_label.is_empty() {
                    backup.account_id
                } else {
                    backup.account_label
                },
                panel_origin: backup.installation.origin,
                installation_id: backup.installation.id,
            }),
            impact: if coordination.is_some() {
                vec!["显式保留当前主配置 preferences 的完整原始内容；仅还原备份中的 deploymentMode，并恢复原索引与 profile。".into(),
                    "确认后任何文件或源备份变化都会停止；不依赖当前账户会话、key、节点或 helper，失效授权不会复活。客户端仍需重启。".into()]
            } else {
                vec!["依次恢复主配置、配置库索引和新 profile 的原字节；客户端仍需重启。".into(),
                    "恢复不依赖当前账户会话、key、节点或 helper；外部修改时整组停止，失效授权不会复活。".into()]
            },
            redacted_diff: if coordination.is_some() {
                serde_json::to_string_pretty(&json!({"coordination":"claude_main_preferences_v1","claude_desktop_config.json":{"deploymentMode":{"from":"3p","to":original_mode.map(|mode| mode.unwrap_or("absent".into())).unwrap_or("unverified".into())},"preferences":"preserve current raw object or absence"},"configLibrary":"restore authenticated original profile and metadata bytes","secrets":"redacted"})).map_err(|_| fail("serialization_failed"))?
            } else {
                "Restore encrypted original configuration; original secrets stay redacted.".into()
            },
            expires_at,
            can_apply: blocked.is_none(),
            can_export_import: false,
            blocking_reason: blocked,
            codex_catalog: None,
            codex_runtime: None,
            codex_configuration_target: None,
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
                    _mapping_reservation: None,
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
        if journal.client != ClientKind::Claude || !journal.requires_recovery(&self.store)? {
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
            self.validate_claude_engine_policy(
                &observation.policy,
                observation.claude_configuration_target.as_ref(),
            )
        };
        let context_guard = |context: &ClaudeContext, restoring: bool| {
            self.validate_claude_context(context, restoring)
        };
        let engine = self.claude_engine(&guard, &context_guard);
        let operation = if qualified {
            engine.claude_operation_details(journal).ok()
        } else {
            None
        };
        let restored = operation.as_ref().is_some_and(|(_, restored, _)| *restored);
        let preserved_preferences = operation
            .as_ref()
            .is_some_and(|(_, _, preserved)| *preserved);
        let recovered = operation.is_some()
            && journal.phase != "conflict"
            && engine.recover(&journal.id).is_ok();
        self.validate_claude_configuration(
            &observation.policy,
            observation.claude_configuration_target.as_ref(),
        )?;
        self.project_claude_operation(restored, recovered, preserved_preferences)
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
            .ok_or_else(|| {
                observation
                    .claude_configuration_error
                    .clone()
                    .unwrap_or_else(|| fail("claude_configuration_target_unverified"))
            })?;
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

    /// Engine policy checks only reject changes to the frozen local policy and
    /// target hint. They never produce a full receipt or replace the fresh full
    /// context qualification at file, grant and post-Vault boundaries.
    fn validate_claude_engine_policy(
        &self,
        expected_policy: &crate::policy::Snapshot,
        expected_target: Option<&ClaudeConfigurationTarget>,
    ) -> Result<()> {
        if !self.paths.writes_authorized {
            return Err(fail("integration_authorization_required"));
        }
        let expected_target =
            expected_target.ok_or_else(|| fail("claude_configuration_target_unverified"))?;
        expected_target.validate_root()?;
        let (current, policy) = self.client_policy(ClientKind::Claude)?;
        expected_policy.matches_user_config(&policy)?;
        let expected_revision = access_revision(&expected_target.revision, &policy.local_revision);
        if current.configuration_access != ConfigurationAccess::Allowed
            || current.configuration_access_revision.as_deref() != Some(&expected_revision)
            || self.paths.claude_library != expected_target.config_root.join("configLibrary")
        {
            return Err(fail("claude_configuration_target_changed"));
        }
        Ok(())
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
            mapping: compiled.mapping().cloned(),
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
        self.validate_claude_authority_state(
            context,
            restoring,
            compare_catalog,
            &observation.policy,
        )
    }
    fn validate_claude_authority_state(
        &self,
        context: &ClaudeContext,
        restoring: bool,
        compare_catalog: bool,
        policy: &crate::policy::Snapshot,
    ) -> Result<()> {
        if let Some(plan) = context.mapping.as_ref() {
            plan.validate()?;
        }
        if context.contract_version != 1
            || scope_revision(context)? != context.scope_revision
            || self.paths.claude_library
                != context
                    .configuration_target
                    .config_root
                    .join("configLibrary")
            || policy.revision != context.policy_revision
            || policy.local_revision != context.local_policy_revision
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
