use super::*;
use crate::{identity::bound_helper_token, network::RemoteResponse};

fn key(letter: char, legacy: bool) -> ApiKeyView {
    serde_json::from_value(json!({
        "id":letter.to_string().repeat(26), "name":format!("Synthetic {letter}"), "prefix":"orv_sk_public",
        "status":"active","state":"active","key_revision":letter.to_ascii_lowercase().to_string().repeat(64),
        "management_revision":"f".repeat(64),"service_tier":"follow_client","force_fast":false,"service_tier_revision":1,
        "spending_limit_mode":"cumulative","spending_limit":null,"spending_used":"0.00000000",
        "spending_active_used":"0.00000000","spending_remaining":null,"spending_window_minutes":null,
        "spending_window_used":"0.00000000","spending_window_started_at":null,"spending_window_expires_at":null,
        "spending_window_resets_at":null,"spending_window_status":null,"expires_at":null,"last_used_at":null,
        "created_at":null,"revoked_at":null,"legacy_issued":legacy,"selectable":!legacy
    })).unwrap()
}
fn wire(keys: &[ApiKeyView]) -> Value {
    let fixture: Value =
        serde_json::from_str(include_str!("../tests/fixtures/public-wire.json")).unwrap();
    let mut data = fixture["bootstrap_reply"]["data"].clone();
    data["api_key_management"] = json!(true);
    data["inference_keys"] = json!(
        keys.iter()
            .map(|key| json!({
                "provider_id":"openai","api_key_id":key.id,"name":key.name,"prefix":key.prefix,
                "key_revision":key.key_revision,"state":key.state,"expires_at":key.expires_at
            }))
            .collect::<Vec<_>>()
    );
    data
}
fn profile(service: &Service, record: &ConnectionRecord, keys: &[ApiKeyView]) {
    let mut session = service.session(record).unwrap();
    session.scopes = [
        "api-keys:read",
        "api-keys:write",
        "api-keys:use",
        "api-keys:reveal",
    ]
    .map(String::from)
    .to_vec();
    service.save_session(record, session).unwrap();
    let cache = map_account_cache(&wire(keys), record).unwrap();
    service
        .store
        .put(
            "account-cache",
            &record.view.id,
            &record.view.account_id,
            &cache,
        )
        .unwrap();
}
fn reference(record: &ConnectionRecord, key: &ApiKeyView) -> KeyReference {
    KeyReference {
        connection_id: record.view.id.clone(),
        expected_revision: record.view.revision,
        api_key_id: key.id.clone(),
        key_revision: key.key_revision.clone(),
    }
}
fn page(legacy: Option<&ApiKeyView>) -> Value {
    json!({"items":[],"page":1,"last_page":1,"total":0,"currency":"USD","can_write":true,
        "can_reveal":true,"legacy_key":legacy,"legacy_cleanup_completed":false})
}
fn response(data: Value) -> Result<RemoteResponse> {
    Ok(RemoteResponse {
        status: 200,
        request_id: None,
        bytes: serde_json::to_vec(&json!({"data":data})).unwrap(),
    })
}
fn assert_authenticated(request: &crate::network::FixtureRequest) {
    assert!(
        request
            .headers
            .iter()
            .any(|(name, value)| name == "Authorization" && value.starts_with("Bearer "))
    );
    assert!(
        request
            .headers
            .iter()
            .any(|(name, _)| name == "X-Orbit-Proof-Signature")
    );
}
fn selection_transport(service: &Service, keys: Vec<ApiKeyView>) {
    service.http.scripted(Arc::new(move |request| {
        assert_authenticated(&request);
        let keys = keys.clone();
        Box::pin(async move {
            let path = url::Url::parse(&request.url).unwrap().path().to_owned();
            if path == "/api/v1/orbit/bootstrap" { return response(wire(&keys)); }
            if path == "/api/v1/orbit/api-keys" { return response(page(None)); }
            if path.ends_with("/ticket") {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                assert!(["use","reveal"].contains(&body["purpose"].as_str().unwrap()));
                return response(json!({"delivery_ticket":"t".repeat(43),
                    "expires_at":(chrono::Utc::now()+chrono::Duration::seconds(60)).to_rfc3339()}));
            }
            if path.ends_with("/delivery") {
                return response(json!({"provider_id":"openai","api_key_id":keys.last().unwrap().id,
                    "key_revision":keys.last().unwrap().key_revision,"plain_text_token":format!("orv_sk_synthetic_{}",keys.last().unwrap().id.chars().next().unwrap()),"expires_at":null}));
            }
            let chosen = keys.iter().find(|key| path.ends_with(&key.id)).unwrap();
            response(json!(chosen))
        })
    }));
}
fn binding(service: &Service, record: &ConnectionRecord) -> HelperBinding {
    let binding = HelperBinding {
        id: uuid(),
        client: ClientKind::Codex,
        origin: record.view.panel_origin.clone(),
        account_id: record.view.account_id.clone(),
        installation_id: record.installation_id.clone(),
        provider_id: "openai".into(),
        credential_revision: record.view.credential_revision.clone().unwrap(),
        secret_reference: record.credential_reference.clone().unwrap(),
        enabled: true,
        expires_at: None,
        authorization_epoch: record.authorization_epoch,
    };
    service
        .store
        .put("binding", &binding.id, &binding.account_id, &binding)
        .unwrap();
    binding
}

#[tokio::test]
async fn selection_keeps_bound_credentials_and_frozen_image_authority_then_fails_closed_on_retirement()
 {
    let (_dir, service, vault, record, _) = super::super::tests::setup();
    let a = key('A', false);
    let b = key('B', false);
    profile(&service, &record, &[a.clone(), b.clone()]);
    selection_transport(&service, vec![a.clone()]);
    service
        .select_api_key(&reference(&record, &a))
        .await
        .unwrap();
    let old = service.record(&record.view.id).unwrap();
    let grant = binding(&service, &old);
    let mut snapshot = super::super::tests::task_snapshot(&old);
    snapshot.task.state = "queued".into();
    service
        .store
        .put(
            "task-snapshot",
            &snapshot.task.id,
            &old.view.account_id,
            &snapshot,
        )
        .unwrap();
    selection_transport(&service, vec![a.clone(), b.clone()]);
    let bootstrap = service.select_api_key(&reference(&old, &b)).await.unwrap();
    let current = service.record(&record.view.id).unwrap();
    assert_eq!(current.api_key_id.as_deref(), Some(b.id.as_str()));
    assert_eq!(current.authorization_epoch, old.authorization_epoch);
    let token = bound_helper_token(
        &grant,
        ClientKind::Codex,
        &grant.id,
        &service.store,
        vault.as_ref(),
        chrono::Utc::now().timestamp(),
    )
    .unwrap();
    assert_eq!(token.as_str(), "orv_sk_synthetic_A");
    let frozen = service.image_key_record(&snapshot).unwrap();
    assert_eq!(frozen.api_key_id, old.api_key_id);
    assert_eq!(frozen.view.revision, snapshot.task.connection_revision);
    assert!(
        !serde_json::to_string(&bootstrap)
            .unwrap()
            .contains(token.as_str())
    );
    let mut cache = service.account_cache(&current).unwrap();
    cache
        .keys
        .retain(|key| key.api_key_id.as_deref() != Some(a.id.as_str()));
    service
        .synchronize_key_credentials(&current, &cache)
        .unwrap();
    assert_eq!(
        bound_helper_token(
            &grant,
            ClientKind::Codex,
            &grant.id,
            &service.store,
            vault.as_ref(),
            chrono::Utc::now().timestamp()
        )
        .err()
        .unwrap()
        .code,
        "binding_disabled"
    );
    assert!(
        service
            .store
            .get::<KeyCredential>("api-key-credential", &grant.secret_reference)
            .unwrap()
            .is_some_and(|key| !key.enabled)
    );
}

#[tokio::test]
async fn a_delivery_finishing_after_an_account_switch_never_stores_or_returns_the_secret() {
    let (_dir, service, _vault, record, _) = super::super::tests::setup();
    let selected = key('A', false);
    profile(&service, &record, &[selected.clone()]);
    let store = service.store.clone();
    let sent = selected.clone();
    service.http.scripted(Arc::new(move |request| {
        assert_authenticated(&request);
        let store = store.clone(); let key = sent.clone();
        Box::pin(async move {
            if request.url.ends_with("/ticket") { return response(json!({"delivery_ticket":"t".repeat(43),"expires_at":(chrono::Utc::now()+chrono::Duration::seconds(60)).to_rfc3339()})); }
            if request.url.ends_with("/delivery") {
                store.set_meta("active_connection","other-actor-connection").unwrap();
                return response(json!({"provider_id":"openai","api_key_id":key.id,"key_revision":key.key_revision,"plain_text_token":"orv_sk_late_secret","expires_at":null}));
            }
            response(json!(key))
        })
    }));
    assert_eq!(
        service
            .select_api_key(&reference(&record, &selected))
            .await
            .err()
            .unwrap()
            .code,
        "account_scope_mismatch"
    );
    assert!(
        service
            .store
            .list::<KeyCredential>("api-key-credential", None)
            .unwrap()
            .is_empty()
    );
    assert!(
        service
            .record(&record.view.id)
            .unwrap()
            .credential_reference
            .is_none()
    );
}

#[tokio::test]
async fn ambiguous_mutations_are_recovered_with_the_same_body_and_block_new_mutations() {
    let (_dir, service, _vault, record, _) = super::super::tests::setup();
    profile(&service, &record, &[]);
    let bodies = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let observed = bodies.clone();
    service.http.scripted(Arc::new(move |request| {
        assert_authenticated(&request);
        let bodies = observed.clone();
        Box::pin(async move {
            if request.url.ends_with("/bootstrap") { return Err(fail("panel_unavailable")); }
            let mut bodies = bodies.lock().unwrap();
            bodies.push(request.body.clone());
            if bodies.len()==1 { return Err(fail("result_unconfirmed")); }
            let body:Value = serde_json::from_slice(&request.body).unwrap();
            response(json!({"completed":true,"operation_id":body["operation_id"],"api_key":key('A',false)}))
        })
    }));
    let mutation = ApiKeyMutation {
        connection_id: record.view.id.clone(),
        expected_revision: record.view.revision,
        action: KeyAction::Create,
        api_key_id: None,
        management_revision: None,
        changes: Some(ApiKeyChanges {
            name: "Synthetic".into(),
            expires_at: None,
            service_tier: "follow_client".into(),
            spending_limit: None,
            spending_limit_mode: "cumulative".into(),
            spending_window_minutes: None,
        }),
    };
    assert_eq!(
        service.mutate_api_key(&mutation).await.err().unwrap().code,
        "api_key_operation_pending"
    );
    assert_eq!(
        service.mutate_api_key(&mutation).await.err().unwrap().code,
        "api_key_operation_pending"
    );
    let result = service
        .recover_api_key_operation(&mutation.scope())
        .await
        .unwrap();
    assert!(result.completed);
    let bodies = bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0], bodies[1]);
    assert!(
        service
            .store
            .get::<PendingKeyMutation>("api-key-pending", &record.view.id)
            .unwrap()
            .is_none()
    );
    assert!(!String::from_utf8_lossy(&bodies[0]).contains("plain_text_token"));
}

#[test]
fn clipboard_final_write_rechecks_scope_and_per_reference_fences_survive_vault_deletion_failure() {
    let (_dir, service, vault, record, _) = super::super::tests::setup();
    let key = key('A', false);
    profile(&service, &record, &[key.clone()]);
    let request = reference(&record, &key);
    let secret = RevealedApiKey {
        api_key_id: key.id,
        plain_text_token: "orv_sk_copy_synthetic".into(),
    };
    service
        .store
        .set_meta("active_connection", "other-actor")
        .unwrap();
    let mut written = false;
    assert!(
        service
            .commit_api_key_copy(&request, &secret, |_| {
                written = true;
                Ok(())
            })
            .is_err()
    );
    assert!(!written);
    // A denied OS-vault delete leaves bytes present; the durable fence still wins.
    vault.put("denied-delete", b"still-present").unwrap();
    service
        .store
        .set_meta(
            &crate::identity::credential_disabled_reference("denied-delete"),
            "1",
        )
        .unwrap();
    let grant = HelperBinding {
        id: uuid(),
        client: ClientKind::Codex,
        origin: record.view.panel_origin.clone(),
        account_id: record.view.account_id.clone(),
        installation_id: record.installation_id.clone(),
        provider_id: "openai".into(),
        credential_revision: "a".repeat(64),
        secret_reference: "denied-delete".into(),
        enabled: true,
        expires_at: None,
        authorization_epoch: record.authorization_epoch,
    };
    assert_eq!(
        bound_helper_token(
            &grant,
            ClientKind::Codex,
            &grant.id,
            &service.store,
            vault.as_ref(),
            chrono::Utc::now().timestamp()
        )
        .err()
        .unwrap()
        .code,
        "binding_disabled"
    );
    assert!(vault.get("denied-delete").is_ok());
}

#[tokio::test]
async fn remembered_default_requires_the_same_actor_installation_and_a_new_explicit_use_scope() {
    let (_dir, service, _vault, mut record, _) = super::super::tests::setup();
    let key = key('A', false);
    profile(&service, &record, &[key.clone()]);
    selection_transport(&service, vec![key.clone()]);
    service
        .select_api_key(&reference(&record, &key))
        .await
        .unwrap();
    record = service.record(&record.view.id).unwrap();
    let selection = service
        .store
        .get::<LocalKeySelection>("api-key-selection", &record.view.id)
        .unwrap()
        .unwrap();
    assert_eq!(selection.api_key_id, key.id);
    assert!(
        !serde_json::to_string(&selection)
            .unwrap()
            .contains("orv_sk_")
    );
    record.credential_reference = None;
    record.view.credential_revision = None;
    record.credential_state = "unavailable".into();
    record.view.revision += 1;
    service
        .store
        .put(
            "connection",
            &record.view.id,
            &record.view.account_id,
            &record,
        )
        .unwrap();
    let mut session = service.session(&record).unwrap();
    session.scopes.retain(|scope| scope != "api-keys:use");
    service.save_session(&record, session).unwrap();
    service
        .restore_local_key_selection(&record.view.id)
        .await
        .unwrap();
    assert!(
        service
            .record(&record.view.id)
            .unwrap()
            .credential_reference
            .is_none()
    );
    profile(&service, &record, &[key.clone()]);
    service
        .restore_local_key_selection(&record.view.id)
        .await
        .unwrap();
    assert_eq!(
        service
            .record(&record.view.id)
            .unwrap()
            .api_key_id
            .as_deref(),
        Some(key.id.as_str())
    );
    let foreign = LocalKeySelection {
        account_id: "another-actor".into(),
        ..selection
    };
    service
        .store
        .put(
            "api-key-selection",
            &record.view.id,
            &record.view.account_id,
            &foreign,
        )
        .unwrap();
    assert_eq!(
        service
            .restore_local_key_selection(&record.view.id)
            .await
            .err()
            .unwrap()
            .code,
        "account_scope_mismatch"
    );
}
