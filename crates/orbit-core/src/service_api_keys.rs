use super::*;
use crate::api_keys::*;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct KeyCredential {
    pub origin: String,
    pub account_id: String,
    pub installation_id: String,
    pub api_key_id: String,
    pub key_revision: String,
    pub secret_reference: String,
    pub expires_at: Option<i64>,
    pub authorization_epoch: u64,
    pub enabled: bool,
}
#[derive(Clone, Serialize, Deserialize)]
struct PendingKeyMutation {
    origin: String,
    account_id: String,
    installation_id: String,
    target: String,
    method: String,
    body: Value,
}
#[derive(Deserialize)]
struct DeliveryTicket {
    delivery_ticket: String,
    expires_at: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct LocalKeySelection {
    origin: String,
    account_id: String,
    installation_id: String,
    api_key_id: String,
}

impl Service {
    pub(super) fn frozen_api_key_record(
        &self,
        record: &ConnectionRecord,
        reference: &str,
        revision: &str,
    ) -> Result<ConnectionRecord> {
        let credential = self
            .store
            .get::<KeyCredential>("api-key-credential", reference)?
            .ok_or_else(|| fail("binding_disabled"))?;
        let cache = self.account_cache(record)?;
        if !credential.enabled
            || record.session_status != "authenticated"
            || !cache.inference_allowed
            || self.store.meta("active_connection")?.as_deref() != Some(record.view.id.as_str())
            || record.view.account_id != credential.account_id
            || record.view.panel_origin != credential.origin
            || record.installation_id != credential.installation_id
            || record.authorization_epoch != credential.authorization_epoch
            || credential.secret_reference != reference
            || credential.key_revision != revision
            || record
                .session_expires_at
                .is_none_or(|at| at <= chrono::Utc::now().timestamp())
            || credential
                .expires_at
                .is_some_and(|at| at <= chrono::Utc::now().timestamp())
            || self
                .store
                .meta(&crate::identity::credential_disabled_reference(reference))?
                .as_deref()
                == Some("1")
            || !cache.keys.iter().any(|key| {
                key.api_key_id.as_ref() == Some(&credential.api_key_id)
                    && key.key_revision.as_ref() == Some(&credential.key_revision)
                    && key.state == "active"
            })
        {
            return Err(fail("binding_disabled"));
        }
        let mut frozen = record.clone();
        frozen.api_key_id = Some(credential.api_key_id);
        frozen.credential_reference = Some(credential.secret_reference);
        frozen.view.credential_revision = Some(credential.key_revision);
        frozen.credential_expires_at = credential.expires_at;
        frozen.credential_state = "active".into();
        frozen.view.permission_state = "allowed".into();
        Ok(frozen)
    }
    pub(super) fn image_key_record(&self, snapshot: &TaskSnapshot) -> Result<ConnectionRecord> {
        let record = self.record(&snapshot.task.connection_id)?;
        if !self.has_key_access(&record, "api-keys:read") {
            return Ok(record);
        }
        if record.view.node_id.as_ref() != Some(&snapshot.task.node_id)
            || record.view.edge_base_url.as_ref() != Some(&snapshot.edge_base_url)
        {
            return Err(fail("connection_conflict"));
        }
        let mut frozen = self.frozen_api_key_record(
            &record,
            &snapshot.secret_reference,
            &snapshot.task.credential_revision,
        )?;
        frozen.view.revision = snapshot.task.connection_revision;
        Ok(frozen)
    }
    pub(super) fn validate_api_key_store(store: &Store) -> Result<()> {
        let _ = store.list::<KeyCredential>("api-key-credential", None)?;
        let _ = store.list::<PendingKeyMutation>("api-key-pending", None)?;
        for selection in store.list::<LocalKeySelection>("api-key-selection", None)? {
            validate_id(&selection.api_key_id)?;
        }
        Ok(())
    }
    pub(super) fn retire_api_key_credentials(&self, record: &mut ConnectionRecord) -> Result<()> {
        for mut credential in self
            .store
            .list::<KeyCredential>("api-key-credential", Some(&record.view.account_id))?
        {
            if credential.origin != record.view.panel_origin
                || credential.installation_id != record.installation_id
            {
                continue;
            }
            credential.enabled = false;
            self.store.set_meta(
                &crate::identity::credential_disabled_reference(&credential.secret_reference),
                "1",
            )?;
            self.store.put(
                "api-key-credential",
                &credential.secret_reference,
                &credential.account_id,
                &credential,
            )?;
            if !record
                .cleanup_references
                .contains(&credential.secret_reference)
            {
                record.cleanup_references.push(credential.secret_reference);
            }
        }
        Ok(())
    }
    fn key_scope(
        &self,
        scope: &KeyScope,
        expected: Option<&ConnectionRecord>,
    ) -> Result<ConnectionRecord> {
        let current = self.record(&scope.connection_id)?;
        if current.view.revision != scope.expected_revision
            || current.session_status != "authenticated"
            || current
                .session_expires_at
                .is_none_or(|at| at <= chrono::Utc::now().timestamp())
            || self.store.meta("active_connection")?.as_deref()
                != Some(scope.connection_id.as_str())
            || expected.is_some_and(|old| {
                old.view.account_id != current.view.account_id
                    || old.view.panel_origin != current.view.panel_origin
                    || old.installation_id != current.installation_id
                    || old.authorization_epoch != current.authorization_epoch
                    || old.session_reference != current.session_reference
            })
        {
            return Err(fail("account_scope_mismatch"));
        }
        self.ensure_installation_available(&current.installation_id)?;
        Ok(current)
    }
    pub(super) fn has_key_access(&self, record: &ConnectionRecord, scope: &str) -> bool {
        self.session(record)
            .is_ok_and(|s| s.scopes.iter().any(|item| item == scope))
    }
    pub(super) async fn restore_local_key_selection(&self, id: &str) -> Result<()> {
        let record = self.record(id)?;
        if !self.has_key_access(&record, "api-keys:use") {
            return Ok(());
        }
        let Some(selection) = self
            .store
            .get::<LocalKeySelection>("api-key-selection", id)?
        else {
            return Ok(());
        };
        if selection.origin != record.view.panel_origin
            || selection.account_id != record.view.account_id
            || selection.installation_id != record.installation_id
        {
            return Err(fail("account_scope_mismatch"));
        }
        let cache = self.account_cache(&record)?;
        let Some(key) = cache.keys.iter().find(|key| {
            key.api_key_id.as_ref() == Some(&selection.api_key_id) && key.state == "active"
        }) else {
            return Ok(());
        };
        let Some(revision) = &key.key_revision else {
            return Ok(());
        };
        self.select_api_key(&KeyReference {
            connection_id: id.into(),
            expected_revision: record.view.revision,
            api_key_id: selection.api_key_id,
            key_revision: revision.clone(),
        })
        .await?;
        Ok(())
    }
    fn require_key_access(&self, record: &ConnectionRecord, scope: &str) -> Result<()> {
        if !self.account_cache(record)?.api_keys_supported {
            return Err(fail("api_key_management_unavailable"));
        }
        if !self.has_key_access(record, scope) {
            return Err(fail("api_key_authorization_required"));
        }
        Ok(())
    }
    async fn remote_key(
        &self,
        record: &ConnectionRecord,
        installation: &Installation,
        session: &ProofSession,
        id: &str,
    ) -> Result<ApiKeyView> {
        validate_id(id)?;
        let key: ApiKeyView = self
            .http
            .panel(
                &record.view.panel_origin,
                &format!("/api/v1/orbit/api-keys/{id}"),
                Method::GET,
                vec![],
                Some((installation, session, self.vault.as_ref())),
                true,
            )
            .await?;
        key.validate()?;
        if key.id != id {
            return Err(fail("invalid_panel_response"));
        }
        Ok(key)
    }
    pub async fn list_api_keys(&self, request: &ApiKeyQuery) -> Result<ApiKeyPage> {
        let _operation = self.begin_mutation()?;
        let target = request.target()?;
        let scope = request.scope();
        let original = self.key_scope(&scope, None)?;
        self.require_key_access(&original, "api-keys:read")?;
        let (record, installation, session) = self.fresh_session(&request.connection_id).await?;
        self.key_scope(&scope, Some(&original))?;
        let mut page: ApiKeyPage = self
            .http
            .panel(
                &record.view.panel_origin,
                &target,
                Method::GET,
                vec![],
                Some((&installation, &session, self.vault.as_ref())),
                true,
            )
            .await?;
        if page.items.len() > 15
            || page.page == 0
            || page.last_page == 0
            || page.currency.len() != 3
        {
            return Err(fail("invalid_panel_response"));
        }
        for key in &page.items {
            key.validate()?;
        }
        if let Some(key) = &page.legacy_key {
            key.validate()?;
        }
        page.selected = match record.api_key_id.as_deref() {
            None => None,
            Some(id)
                if !self
                    .account_cache(&record)?
                    .keys
                    .iter()
                    .any(|key| key.api_key_id.as_deref() == Some(id)) =>
            {
                None
            }
            Some(id) => {
                if let Some(key) = page.items.iter().find(|key| key.id == id) {
                    Some(key.clone())
                } else {
                    match self.remote_key(&record, &installation, &session, id).await {
                        Ok(key) => Some(key),
                        Err(error)
                            if [
                                "not_found",
                                "http_404",
                                "resource_not_found",
                                "endpoint_unavailable",
                            ]
                            .contains(&error.code.as_str()) =>
                        {
                            None
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
        };
        self.key_scope(&scope, Some(&record))?;
        page.bindings = self.api_key_bindings(&record)?;
        page.selected_ready = record.credential_reference.is_some()
            && record.credential_state == "active"
            && record.view.permission_state == "allowed";
        page.pending_operation = self
            .store
            .get::<PendingKeyMutation>("api-key-pending", &record.view.id)?
            .is_some();
        page.cleanup_status = self
            .store
            .get::<String>("api-key-cleanup", &record.view.id)?
            .unwrap_or_else(|| {
                if page.legacy_cleanup_completed {
                    "completed"
                } else if page.legacy_key.is_some() {
                    "pending"
                } else {
                    "not_needed"
                }
                .into()
            });
        Ok(page)
    }
    pub async fn get_api_key(&self, request: &KeyReference) -> Result<ApiKeyView> {
        let _operation = self.begin_mutation()?;
        request.validate()?;
        let scope = request.scope();
        let original = self.key_scope(&scope, None)?;
        self.require_key_access(&original, "api-keys:read")?;
        let (record, installation, session) = self.fresh_session(&request.connection_id).await?;
        let key = self
            .remote_key(&record, &installation, &session, &request.api_key_id)
            .await?;
        self.key_scope(&scope, Some(&original))?;
        Ok(key)
    }
    pub async fn mutate_api_key(&self, request: &ApiKeyMutation) -> Result<KeyMutationResult> {
        let _operation = self.begin_mutation()?;
        request.validate()?;
        let serial = self.network_lock(&format!("api-keys/{}", request.connection_id))?;
        let _serial = serial.lock().await;
        let record = self.key_scope(&request.scope(), None)?;
        self.require_key_access(&record, "api-keys:write")?;
        if self
            .store
            .get::<PendingKeyMutation>("api-key-pending", &record.view.id)?
            .is_some()
        {
            return Err(fail("api_key_operation_pending"));
        }
        let mut body = request
            .changes
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|_| fail("serialization_failed"))?
            .unwrap_or_else(|| json!({}));
        body["operation_id"] = json!(uuid());
        body["operation_created_at"] = json!(chrono::Utc::now().to_rfc3339());
        if let Some(revision) = &request.management_revision {
            body["expected_revision"] = json!(revision);
        }
        let target = match request.action {
            KeyAction::Create => "/api/v1/orbit/api-keys".into(),
            KeyAction::Update => format!(
                "/api/v1/orbit/api-keys/{}",
                request.api_key_id.as_deref().unwrap()
            ),
            _ => format!(
                "/api/v1/orbit/api-keys/{}/{}",
                request.api_key_id.as_deref().unwrap(),
                request.action.name()
            ),
        };
        let pending = PendingKeyMutation {
            origin: record.view.panel_origin.clone(),
            account_id: record.view.account_id.clone(),
            installation_id: record.installation_id.clone(),
            target,
            method: if request.action == KeyAction::Update {
                "PATCH"
            } else {
                "POST"
            }
            .into(),
            body,
        };
        self.store.put(
            "api-key-pending",
            &record.view.id,
            &record.view.account_id,
            &pending,
        )?;
        self.perform_key_mutation(&request.scope(), &pending).await
    }
    pub async fn recover_api_key_operation(&self, scope: &KeyScope) -> Result<KeyMutationResult> {
        let _operation = self.begin_mutation()?;
        let serial = self.network_lock(&format!("api-keys/{}", scope.connection_id))?;
        let _serial = serial.lock().await;
        let record = self.key_scope(scope, None)?;
        self.require_key_access(&record, "api-keys:write")?;
        let pending = self
            .store
            .get::<PendingKeyMutation>("api-key-pending", &record.view.id)?
            .ok_or_else(|| fail("api_key_operation_missing"))?;
        self.perform_key_mutation(scope, &pending).await
    }
    async fn perform_key_mutation(
        &self,
        scope: &KeyScope,
        pending: &PendingKeyMutation,
    ) -> Result<KeyMutationResult> {
        let original = self.key_scope(scope, None)?;
        if original.view.panel_origin != pending.origin
            || original.view.account_id != pending.account_id
            || original.installation_id != pending.installation_id
        {
            return Err(fail("account_scope_mismatch"));
        }
        let (record, installation, session) = self.fresh_session(&scope.connection_id).await?;
        self.key_scope(scope, Some(&original))?;
        let result: Result<KeyMutationResult> = self
            .http
            .panel(
                &record.view.panel_origin,
                &pending.target,
                if pending.method == "PATCH" {
                    Method::PATCH
                } else {
                    Method::POST
                },
                serde_json::to_vec(&pending.body).map_err(|_| fail("serialization_failed"))?,
                Some((&installation, &session, self.vault.as_ref())),
                true,
            )
            .await;
        self.key_scope(scope, Some(&record))?;
        match result {
            Ok(result) => {
                if !result.completed
                    || result.operation_id != pending.body["operation_id"].as_str().unwrap_or("")
                {
                    return Err(fail("invalid_panel_response"));
                }
                if let Some(key) = &result.api_key {
                    key.validate()?;
                }
                self.store.delete("api-key-pending", &record.view.id)?;
                // The server acknowledged the action. A failed metadata refresh must
                // not turn a completed create/rotate into an invitation to repeat it.
                let _ = self.refresh_account_cache(&record.view.id, false).await;
                let current = self.record(&record.view.id)?;
                if let Some(key) = &result.api_key {
                    if current.api_key_id.as_deref() == Some(&key.id)
                        && current.credential_reference.is_none()
                        && key.selectable
                        && !key.legacy_issued
                    {
                        let _ = self
                            .select_api_key(&KeyReference {
                                connection_id: current.view.id.clone(),
                                expected_revision: current.view.revision,
                                api_key_id: key.id.clone(),
                                key_revision: key.key_revision.clone(),
                            })
                            .await;
                    }
                }
                Ok(result)
            }
            Err(error) => {
                if [
                    "api_key_changed",
                    "api_key_operation_conflict",
                    "validation_error",
                    "validation_failed",
                    "orbit_scope_denied",
                    "subaccount_read_only",
                    "subaccount_write_denied",
                    "forbidden",
                    "not_found",
                    "api_key_expired",
                    "api_key_unavailable",
                    "connection_conflict",
                    "permission_denied",
                    "endpoint_unavailable",
                    "invalid_api_key_request",
                    "api_key_operation_expired",
                ]
                .contains(&error.code.as_str())
                {
                    self.store.delete("api-key-pending", &record.view.id)?;
                }
                if self
                    .store
                    .get::<PendingKeyMutation>("api-key-pending", &record.view.id)?
                    .is_some()
                {
                    Err(fail("api_key_operation_pending"))
                } else {
                    Err(error)
                }
            }
        }
    }
    async fn deliver_selected_key(
        &self,
        record: &ConnectionRecord,
        installation: &Installation,
        session: &ProofSession,
        key: &KeyReference,
        purpose: &str,
    ) -> Result<CredentialDelivery> {
        let body = serde_json::to_vec(
            &json!({"expected_key_revision":key.key_revision,"purpose":purpose}),
        )
        .map_err(|_| fail("serialization_failed"))?;
        let ticket: DeliveryTicket = self
            .http
            .panel(
                &record.view.panel_origin,
                &format!("/api/v1/orbit/api-keys/{}/ticket", key.api_key_id),
                Method::POST,
                body,
                Some((installation, session, self.vault.as_ref())),
                true,
            )
            .await?;
        if ticket.delivery_ticket.len() != 43
            || parse_timestamp(&ticket.expires_at)? <= chrono::Utc::now().timestamp()
        {
            return Err(fail("invalid_panel_response"));
        }
        let body = Zeroizing::new(
            serde_json::to_vec(
                &json!({"delivery_ticket":ticket.delivery_ticket,"purpose":purpose}),
            )
            .map_err(|_| fail("serialization_failed"))?,
        );
        let delivery: CredentialDelivery = self
            .http
            .panel(
                &record.view.panel_origin,
                "/api/v1/orbit/api-keys/delivery",
                Method::POST,
                body.to_vec(),
                Some((installation, session, self.vault.as_ref())),
                true,
            )
            .await?;
        if delivery.api_key_id != key.api_key_id
            || delivery.key_revision != key.key_revision
            || delivery.provider_id != "openai"
            || !delivery.plain_text_token.starts_with("orv_sk_")
            || delivery.plain_text_token.len() > 4096
            || delivery.plain_text_token.chars().any(char::is_whitespace)
        {
            return Err(fail("invalid_panel_response"));
        }
        Ok(delivery)
    }
    pub async fn reveal_api_key(&self, request: &KeyReference) -> Result<RevealedApiKey> {
        let _operation = self.begin_mutation()?;
        request.validate()?;
        let serial = self.network_lock(&format!("api-key-delivery/{}", request.connection_id))?;
        let _serial = serial.lock().await;
        let record = self.key_scope(&request.scope(), None)?;
        self.require_key_access(&record, "api-keys:reveal")?;
        let (record, installation, session) = self.fresh_session(&request.connection_id).await?;
        let delivery = self
            .deliver_selected_key(&record, &installation, &session, request, "reveal")
            .await?;
        self.key_scope(&request.scope(), Some(&record))?;
        Ok(RevealedApiKey {
            api_key_id: delivery.api_key_id,
            plain_text_token: delivery.plain_text_token,
        })
    }
    /// Serialize the final native clipboard write with account changes and logout.
    pub fn commit_api_key_copy(
        &self,
        request: &KeyReference,
        secret: &RevealedApiKey,
        write: impl FnOnce(&str) -> Result<()>,
    ) -> Result<()> {
        let _operation = self.begin_mutation()?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let record = self.key_scope(&request.scope(), None)?;
        self.require_key_access(&record, "api-keys:reveal")?;
        if secret.api_key_id != request.api_key_id {
            return Err(fail("account_scope_mismatch"));
        }
        write(&secret.plain_text_token)
    }
    pub async fn select_api_key(&self, request: &KeyReference) -> Result<BootstrapView> {
        let _operation = self.begin_mutation()?;
        request.validate()?;
        let serial = self.network_lock(&format!("api-key-delivery/{}", request.connection_id))?;
        let _serial = serial.lock().await;
        let original = self.key_scope(&request.scope(), None)?;
        self.require_key_access(&original, "api-keys:use")?;
        let (record, installation, session) = self.fresh_session(&request.connection_id).await?;
        let view = self
            .remote_key(&record, &installation, &session, &request.api_key_id)
            .await?;
        if !view.selectable || view.legacy_issued || view.key_revision != request.key_revision {
            return Err(fail("api_key_unavailable"));
        }
        let delivery = self
            .deliver_selected_key(&record, &installation, &session, request, "use")
            .await?;
        self.key_scope(&request.scope(), Some(&original))?;
        let expires_at = delivery
            .expires_at
            .as_deref()
            .map(parse_timestamp)
            .transpose()?;
        if expires_at.is_some_and(|at| at <= chrono::Utc::now().timestamp()) {
            return Err(fail("credential_expired"));
        }
        let reference = format!(
            "inference/{}/{}/{}/{}",
            installation.id,
            sha256(record.view.account_id.as_bytes()),
            delivery.key_revision,
            uuid()
        );
        let secret = InferenceSecret {
            origin: installation.origin.clone(),
            account_id: record.view.account_id.clone(),
            installation_id: installation.id.clone(),
            provider_id: "openai".into(),
            credential_revision: delivery.key_revision.clone(),
            token: delivery.plain_text_token,
            expires_at,
            authorization_epoch: record.authorization_epoch,
        };
        let bytes =
            Zeroizing::new(serde_json::to_vec(&secret).map_err(|_| fail("serialization_failed"))?);
        self.vault.put(&reference, &bytes)?;
        let committed = (|| -> Result<()> {
            let _gate = self
                .account_gate
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            let mut current = self.key_scope(&request.scope(), Some(&record))?;
            self.retain_current_key(&current)?;
            let credential = KeyCredential {
                origin: current.view.panel_origin.clone(),
                account_id: current.view.account_id.clone(),
                installation_id: current.installation_id.clone(),
                api_key_id: request.api_key_id.clone(),
                key_revision: request.key_revision.clone(),
                secret_reference: reference.clone(),
                expires_at,
                authorization_epoch: current.authorization_epoch,
                enabled: true,
            };
            self.store.put(
                "api-key-credential",
                &reference,
                &current.view.account_id,
                &credential,
            )?;
            self.store.put(
                "api-key-selection",
                &current.view.id,
                &current.view.account_id,
                &LocalKeySelection {
                    origin: current.view.panel_origin.clone(),
                    account_id: current.view.account_id.clone(),
                    installation_id: current.installation_id.clone(),
                    api_key_id: request.api_key_id.clone(),
                },
            )?;
            current.credential_reference = Some(reference.clone());
            current.api_key_id = Some(request.api_key_id.clone());
            current.view.credential_revision = Some(request.key_revision.clone());
            current.credential_state = "active".into();
            current.credential_expires_at = expires_at;
            current.view.permission_state = "allowed".into();
            current.view.revision += 1;
            self.store.put(
                "connection",
                &current.view.id,
                &current.view.account_id,
                &current,
            )?;
            self.invalidate_evidence()?;
            Ok(())
        })();
        if let Err(error) = committed {
            self.store.set_meta(
                &crate::identity::credential_disabled_reference(&reference),
                "1",
            )?;
            let _ = self.vault.delete(&reference);
            return Err(error);
        }
        let _ = self.refresh_account_cache(&record.view.id, false).await;
        let _ = self.cleanup_legacy_api_key(&record.view.id).await;
        self.prune_api_key_credentials(&record.view.id)?;
        self.changed();
        self.bootstrap()
    }
    pub(super) fn prune_api_key_credentials(&self, id: &str) -> Result<()> {
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let record = self.record(id)?;
        let bindings = self
            .store
            .list::<HelperBinding>("binding", Some(&record.view.account_id))?;
        let tasks = self
            .store
            .list::<TaskSnapshot>("task-snapshot", Some(&record.view.account_id))?;
        for mut credential in self
            .store
            .list::<KeyCredential>("api-key-credential", Some(&record.view.account_id))?
        {
            if credential.origin != record.view.panel_origin
                || credential.installation_id != record.installation_id
                || record.credential_reference.as_ref() == Some(&credential.secret_reference)
                || bindings.iter().any(|binding| {
                    binding.enabled && binding.secret_reference == credential.secret_reference
                })
                || tasks.iter().any(|task| {
                    task.secret_reference == credential.secret_reference
                        && ["queued", "sending", "waiting", "saving"]
                            .contains(&task.task.state.as_str())
                })
            {
                continue;
            }
            credential.enabled = false;
            self.store.set_meta(
                &crate::identity::credential_disabled_reference(&credential.secret_reference),
                "1",
            )?;
            self.store.put(
                "api-key-credential",
                &credential.secret_reference,
                &credential.account_id,
                &credential,
            )?;
            if self.vault.delete(&credential.secret_reference).is_ok() {
                self.store
                    .delete("api-key-credential", &credential.secret_reference)?;
            }
        }
        Ok(())
    }
    pub(super) fn retain_current_key(&self, record: &ConnectionRecord) -> Result<()> {
        if let (Some(reference), Some(id), Some(revision)) = (
            &record.credential_reference,
            &record.api_key_id,
            &record.view.credential_revision,
        ) {
            if self
                .store
                .get::<KeyCredential>("api-key-credential", reference)?
                .is_none()
            {
                self.store.put(
                    "api-key-credential",
                    reference,
                    &record.view.account_id,
                    &KeyCredential {
                        origin: record.view.panel_origin.clone(),
                        account_id: record.view.account_id.clone(),
                        installation_id: record.installation_id.clone(),
                        api_key_id: id.clone(),
                        key_revision: revision.clone(),
                        secret_reference: reference.clone(),
                        expires_at: record.credential_expires_at,
                        authorization_epoch: record.authorization_epoch,
                        enabled: record.credential_state == "active",
                    },
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn synchronize_key_credentials(
        &self,
        record: &ConnectionRecord,
        cache: &AccountCache,
    ) -> Result<()> {
        self.retain_current_key(record)?;
        for mut credential in self
            .store
            .list::<KeyCredential>("api-key-credential", Some(&record.view.account_id))?
        {
            if credential.origin != record.view.panel_origin
                || credential.installation_id != record.installation_id
            {
                continue;
            }
            let valid = cache.inference_allowed
                && credential.authorization_epoch == record.authorization_epoch
                && cache.keys.iter().any(|key| {
                    key.api_key_id.as_ref() == Some(&credential.api_key_id)
                        && key.key_revision.as_ref() == Some(&credential.key_revision)
                        && key.state == "active"
                        && key.expires_at == credential.expires_at
                });
            if credential.enabled && !valid {
                credential.enabled = false;
                self.store.set_meta(
                    &crate::identity::credential_disabled_reference(&credential.secret_reference),
                    "1",
                )?;
                self.store.put(
                    "api-key-credential",
                    &credential.secret_reference,
                    &credential.account_id,
                    &credential,
                )?;
                for mut binding in self
                    .store
                    .list::<HelperBinding>("binding", Some(&credential.account_id))?
                {
                    if binding.origin == credential.origin
                        && binding.installation_id == credential.installation_id
                        && binding.secret_reference == credential.secret_reference
                    {
                        binding.enabled = false;
                        self.store
                            .put("binding", &binding.id, &binding.account_id, &binding)?;
                        self.claude_mappings.stop(&binding.id);
                    }
                }
                let _ = self.vault.delete(&credential.secret_reference);
            }
        }
        Ok(())
    }
    fn api_key_bindings(&self, record: &ConnectionRecord) -> Result<Vec<KeyBindingView>> {
        let mut bindings = vec![];
        let engine = self.config_engine();
        let cache = self.account_cache(record)?;
        for journal in self
            .store
            .list::<Journal>("journal", Some(&record.view.account_id))?
        {
            if journal.purpose != JournalPurpose::Switch
                || journal.phase != "committed"
                || journal.installation.id != record.installation_id
                || journal.installation.origin != record.view.panel_origin
            {
                continue;
            }
            let (id, revision, claude_mapping) = engine.key_binding(&journal)?;
            let name = cache
                .keys
                .iter()
                .find(|key| key.api_key_id == id)
                .map(|key| key.name.clone())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "API Key".into());
            bindings.push(KeyBindingView {
                client: journal.client,
                current: id == record.api_key_id
                    && Some(&revision) == record.view.credential_revision.as_ref(),
                api_key_id: id,
                name,
                key_revision: revision,
                claude_mapping,
            });
        }
        Ok(bindings)
    }
    pub(super) async fn cleanup_legacy_api_key(&self, id: &str) -> Result<()> {
        let serial = self.network_lock(&format!("api-key-cleanup/{id}"))?;
        let _serial = serial.lock().await;
        let record = self.record(id)?;
        if !self.has_key_access(&record, "api-keys:write") || record.credential_state != "active" {
            return Ok(());
        }
        let (record, installation, session) = self.fresh_session(id).await?;
        let page: ApiKeyPage = self
            .http
            .panel(
                &record.view.panel_origin,
                "/api/v1/orbit/api-keys?page=1",
                Method::GET,
                vec![],
                Some((&installation, &session, self.vault.as_ref())),
                true,
            )
            .await?;
        let Some(legacy) = page.legacy_key else {
            self.store.put(
                "api-key-cleanup",
                id,
                &record.view.account_id,
                &if page.legacy_cleanup_completed {
                    "completed"
                } else {
                    "not_needed"
                },
            )?;
            return Ok(());
        };
        legacy.validate()?;
        if record.api_key_id.as_deref() == Some(&legacy.id)
            || self
                .api_key_bindings(&record)?
                .iter()
                .any(|binding| binding.api_key_id.as_deref() == Some(&legacy.id))
            || self
                .store
                .list::<HelperBinding>("binding", Some(&record.view.account_id))?
                .iter()
                .any(|binding| {
                    binding.enabled
                        && binding.origin == record.view.panel_origin
                        && binding.installation_id == record.installation_id
                        && binding.credential_revision == legacy.key_revision
                })
            || self
                .store
                .list::<TaskSnapshot>("task-snapshot", Some(&record.view.account_id))?
                .iter()
                .any(|task| {
                    task.panel_origin == record.view.panel_origin
                        && task.installation_id == record.installation_id
                        && task.task.credential_revision == legacy.key_revision
                        && (["queued", "sending", "waiting", "saving"]
                            .contains(&task.task.state.as_str())
                            || task.task.state == "unconfirmed"
                                && task.task.request_id.is_some()
                                && task.task.result_state == "unconfirmed")
                })
        {
            self.store
                .put("api-key-cleanup", id, &record.view.account_id, &"pending")?;
            return Ok(());
        }
        let Some(replacement) = record.api_key_id.as_deref() else {
            return Ok(());
        };
        let scope = KeyScope {
            connection_id: id.into(),
            expected_revision: record.view.revision,
        };
        self.key_scope(&scope, Some(&record))?;
        let body = serde_json::to_vec(&json!({"legacy_key_id":legacy.id,"expected_key_revision":legacy.key_revision,
            "replacement_key_id":replacement,"replacement_key_revision":record.view.credential_revision})).map_err(|_| fail("serialization_failed"))?;
        let result: Value = self
            .http
            .panel(
                &record.view.panel_origin,
                "/api/v1/orbit/api-keys/legacy-cleanup",
                Method::POST,
                body,
                Some((&installation, &session, self.vault.as_ref())),
                true,
            )
            .await?;
        self.key_scope(&scope, Some(&record))?;
        if result["completed"] != true {
            return Err(fail("invalid_panel_response"));
        }
        self.store
            .put("api-key-cleanup", id, &record.view.account_id, &"completed")?;
        self.refresh_account_cache(id, false).await
    }
}

#[cfg(test)]
#[path = "service_api_keys_tests.rs"]
mod tests;
