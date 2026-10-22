use super::*;
use crate::config::ClaudeContext;

fn automatic_preview(service: &Service, fixture: &tests::PolicyFixture) -> ConfigPreview {
    service
        .preview_client_config(
            &fixture.record.view.id,
            fixture.record.view.revision,
            ClientKind::Claude,
            &fixture.model,
        )
        .unwrap()
}

fn assert_unobserved(service: &Service, state: &str) {
    let client = service
        .bootstrap()
        .unwrap()
        .clients
        .into_iter()
        .find(|client| client.client == ClientKind::Claude)
        .unwrap();
    assert_eq!(client.configuration_state, state);
    assert!(client.loaded_evidence.is_none() && client.connection_evidence.is_none());
    assert_eq!(client.connection_state, "not_tested");
    if state == "saved" {
        assert_eq!(client.loaded_state, "restart_required");
    }
}

#[test]
fn claude_b3b_service_saves_reopens_and_restores_exact_three_files_without_preview_writes() {
    for existing in [false, true] {
        let fixture = tests::policy_fixture(ClientKind::Claude);
        let root = fixture.service.paths.claude_library.parent().unwrap();
        let old_id = uuid();
        let old_profile = fixture
            .service
            .paths
            .claude_library
            .join(format!("{old_id}.json"));
        let metadata = fixture.service.paths.claude_library.join("_meta.json");
        let main = root.join("claude_desktop_config.json");
        if existing {
            std::fs::create_dir_all(&fixture.service.paths.claude_library).unwrap();
            std::fs::write(&old_profile, br#"{"inferenceProvider":"gateway","inferenceGatewayApiKey":"SYNTHETIC-OLD-SECRET","keepAwakeEnabled":true}"#).unwrap();
            std::fs::write(
                &metadata,
                serde_json::to_vec(&json!({"appliedId":old_id,
                "entries":[{"id":old_id,"name":"Original","unknown":"META-SECRET"}],
                "unknownMetadata":"UNKNOWN-META-SECRET"}))
                .unwrap(),
            )
            .unwrap();
            std::fs::write(
                &main,
                br#"{"deploymentMode":"1p","unowned":"UNKNOWN-MAIN-SECRET"}"#,
            )
            .unwrap();
        }
        let before_meta = read_file(&metadata).unwrap();
        let before_main = read_file(&main).unwrap();
        let before_profile = read_file(&old_profile).unwrap();
        let preview = automatic_preview(&fixture.service, &fixture);
        assert!(
            preview.can_apply && !preview.can_export_import && preview.blocking_reason.is_none()
        );
        assert_eq!(preview.paths.len(), 3);
        let new_profile = PathBuf::from(&preview.paths[0]);
        assert_ne!(new_profile, old_profile);
        assert_eq!(
            new_profile.parent().unwrap(),
            fixture.service.paths.claude_library
        );
        assert_eq!(Path::new(&preview.paths[1]), metadata);
        assert_eq!(Path::new(&preview.paths[2]), main);
        assert!(validate_uuid(new_profile.file_stem().unwrap().to_str().unwrap()).is_ok());
        assert!(!new_profile.exists());
        assert_eq!(read_file(&metadata).unwrap(), before_meta);
        assert_eq!(read_file(&main).unwrap(), before_main);
        assert_eq!(read_file(&old_profile).unwrap(), before_profile);
        assert_eq!(root.exists(), existing);
        for hidden in [
            "SYNTHETIC-OLD-SECRET",
            "META-SECRET",
            "UNKNOWN-META-SECRET",
            "UNKNOWN-MAIN-SECRET",
            "orv_sk_",
        ] {
            assert!(!serde_json::to_string(&preview).unwrap().contains(hidden));
        }
        assert!(
            fixture
                .service
                .store
                .list::<HelperBinding>("binding", None)
                .unwrap()
                .iter()
                .all(|binding| !binding.enabled)
        );
        assert_eq!(
            fixture
                .service
                .apply_client_config(&preview.id, false, false)
                .unwrap_err()
                .code,
            "confirmation_required"
        );
        assert!(!new_profile.exists());
        let saved = fixture
            .service
            .apply_client_config(&preview.id, true, false)
            .unwrap();
        assert_eq!(saved.state, "saved");
        assert_eq!(saved.loaded_state, "restart_required");
        let profile: Value = serde_json::from_slice(&std::fs::read(&new_profile).unwrap()).unwrap();
        let summary: Value = serde_json::from_str(&preview.redacted_diff).unwrap();
        for field in [
            "inferenceGatewayBaseUrl",
            "inferenceCredentialKind",
            "inferenceGatewayAuthScheme",
            "inferenceCredentialHelper",
            "inferenceCredentialHelperArgs",
            "inferenceModels",
            "modelDiscoveryEnabled",
        ] {
            assert_eq!(summary["profile"][field], profile[field]);
        }
        let cache = fixture.service.account_cache(&fixture.record).unwrap();
        assert_eq!(
            profile["inferenceGatewayBaseUrl"],
            cache.claude_endpoints[fixture.record.view.node_id.as_ref().unwrap()]
        );
        assert_ne!(
            profile["inferenceGatewayBaseUrl"],
            json!(fixture.record.view.edge_base_url)
        );
        let binding_id = profile["inferenceCredentialHelperArgs"][1]
            .as_str()
            .unwrap();
        let binding: HelperBinding = fixture
            .service
            .store
            .get("binding", binding_id)
            .unwrap()
            .unwrap();
        assert!(binding.enabled);
        assert!(
            bound_helper_token(
                &binding,
                ClientKind::Claude,
                binding_id,
                &fixture.service.store,
                fixture.service.vault.as_ref(),
                chrono::Utc::now().timestamp()
            )
            .is_ok()
        );
        assert_unobserved(&fixture.service, "saved");
        // Recreate the Service from the same durable Store and Vault. Preview
        // entries disappear; the authenticated backup remains restorable.
        let store_root = fixture.service.store.root.clone();
        let vault = fixture.service.vault.clone();
        let paths = fixture.service.paths.clone();
        drop(fixture.service);
        let reopened =
            Service::new(Arc::new(Store::open(&store_root).unwrap()), vault, paths).unwrap();
        assert_unobserved(&reopened, "saved");
        assert_eq!(
            reopened
                .apply_client_config(&preview.id, true, false)
                .unwrap_err()
                .code,
            "preview_expired"
        );
        let restore = reopened
            .preview_restore(saved.backup_group_id.as_ref().unwrap())
            .unwrap();
        assert!(restore.can_apply && restore.context.is_none());
        assert_eq!(
            restore.paths,
            preview.paths.iter().rev().cloned().collect::<Vec<_>>()
        );
        assert_eq!(
            reopened
                .apply_client_config(&restore.id, true, true)
                .unwrap()
                .state,
            "restored"
        );
        assert_eq!(read_file(&metadata).unwrap(), before_meta);
        assert_eq!(read_file(&main).unwrap(), before_main);
        assert_eq!(read_file(&old_profile).unwrap(), before_profile);
        assert!(!new_profile.exists());
        assert!(
            !reopened
                .store
                .get::<HelperBinding>("binding", binding_id)
                .unwrap()
                .unwrap()
                .enabled
        );
        assert_unobserved(&reopened, "not_configured");
    }
}

#[test]
fn claude_b3b_logout_restore_needs_no_live_actor_key_node_or_helper_and_never_revives_grants() {
    let fixture = tests::policy_fixture(ClientKind::Claude);
    let first = automatic_preview(&fixture.service, &fixture);
    fixture
        .service
        .apply_client_config(&first.id, true, false)
        .unwrap();
    let before: Vec<_> = first
        .paths
        .iter()
        .map(|path| read_file(Path::new(path)).unwrap())
        .collect();
    let second = automatic_preview(&fixture.service, &fixture);
    let saved = fixture
        .service
        .apply_client_config(&second.id, true, false)
        .unwrap();
    fixture
        .service
        .commit_logout(&fixture.record.view.id)
        .unwrap();
    let mut paths = fixture.service.paths.clone();
    std::fs::remove_file(&paths.helper).unwrap();
    paths.helper_expected_sha256 = None;
    fixture
        .service
        .store
        .delete("account-cache", &fixture.record.view.id)
        .unwrap();
    fixture
        .service
        .store
        .set_meta("active_connection", "")
        .unwrap();
    let store_root = fixture.service.store.root.clone();
    let vault = fixture.service.vault.clone();
    drop(fixture.service);
    let reopened = Service::new(Arc::new(Store::open(&store_root).unwrap()), vault, paths).unwrap();
    let restore = reopened
        .preview_restore(saved.backup_group_id.as_ref().unwrap())
        .unwrap();
    assert!(restore.can_apply);
    reopened
        .apply_client_config(&restore.id, true, true)
        .unwrap();
    for (path, original) in first.paths.iter().zip(before) {
        assert_eq!(read_file(Path::new(path)).unwrap(), original);
    }
    assert!(!Path::new(&second.paths[0]).exists());
    assert!(
        reopened
            .store
            .list::<HelperBinding>("binding", None)
            .unwrap()
            .iter()
            .all(|grant| !grant.enabled)
    );
    assert_unobserved(&reopened, "not_configured");
}

#[test]
fn claude_b3b_startup_coordinates_publishing_apply_and_restore_with_honest_operation_status() {
    let fixture = tests::policy_fixture(ClientKind::Claude);
    let preview = automatic_preview(&fixture.service, &fixture);
    let saved = fixture
        .service
        .apply_client_config(&preview.id, true, false)
        .unwrap();
    let id = saved.backup_group_id.as_ref().unwrap();
    let mut journal: Journal = fixture.service.store.get("journal", id).unwrap().unwrap();
    // Synthetic crash at the all-written/grants-persisted publishing boundary.
    journal.phase = "publishing".into();
    fixture
        .service
        .store
        .put("journal", id, &journal.account_id, &journal)
        .unwrap();
    fixture.service.store.delete("backup", id).unwrap();
    let reopened = Service::new(
        fixture.service.store.clone(),
        fixture.service.vault.clone(),
        fixture.service.paths.clone(),
    )
    .unwrap();
    assert_eq!(
        reopened
            .store
            .get::<Journal>("journal", id)
            .unwrap()
            .unwrap()
            .phase,
        "committed"
    );
    assert!(
        reopened
            .store
            .get::<Journal>("backup", id)
            .unwrap()
            .is_some()
    );
    assert_unobserved(&reopened, "saved");
    let restore = reopened.preview_restore(id).unwrap();
    let mut restoring: Journal = reopened
        .store
        .get(
            "journal",
            reopened
                .previews
                .lock()
                .unwrap()
                .get(&restore.id)
                .unwrap()
                .journal_id
                .as_ref()
                .unwrap(),
        )
        .unwrap()
        .unwrap();
    restoring.phase = "publishing".into();
    reopened
        .store
        .put("journal", &restoring.id, &restoring.account_id, &restoring)
        .unwrap();
    // Synthetic crash after main/meta/profile restoration, before grants/commit.
    for path in &restore.paths {
        std::fs::remove_file(path).unwrap();
    }
    let recovered = Service::new(
        reopened.store.clone(),
        reopened.vault.clone(),
        reopened.paths.clone(),
    )
    .unwrap();
    assert_eq!(
        recovered
            .store
            .get::<Journal>("journal", &restoring.id)
            .unwrap()
            .unwrap()
            .phase,
        "committed"
    );
    assert_unobserved(&recovered, "not_configured");
    assert!(
        recovered
            .store
            .list::<HelperBinding>("binding", None)
            .unwrap()
            .iter()
            .all(|binding| !binding.enabled)
    );
    assert_eq!(
        recovered
            .store
            .get::<Journal>("journal", id)
            .unwrap()
            .unwrap()
            .phase,
        "restored"
    );
}

#[test]
fn claude_b3b_startup_skips_stale_apply_snapshot_after_newer_restore_commits() {
    let fixture = tests::policy_fixture(ClientKind::Claude);
    let preview = automatic_preview(&fixture.service, &fixture);
    let saved = fixture
        .service
        .apply_client_config(&preview.id, true, false)
        .unwrap();
    let source_id = saved.backup_group_id.as_ref().unwrap();
    let mut source: Journal = fixture
        .service
        .store
        .get("journal", source_id)
        .unwrap()
        .unwrap();
    // The original confirmed apply was interrupted after all three writes.
    source.phase = "publishing".into();
    fixture
        .service
        .store
        .put("journal", source_id, &source.account_id, &source)
        .unwrap();
    fixture.service.store.delete("backup", source_id).unwrap();
    let restore = fixture.service.preview_restore(source_id).unwrap();
    let restore_id = fixture
        .service
        .previews
        .lock()
        .unwrap()
        .get(&restore.id)
        .unwrap()
        .journal_id
        .clone()
        .unwrap();
    let mut restoring: Journal = fixture
        .service
        .store
        .get("journal", &restore_id)
        .unwrap()
        .unwrap();
    restoring.phase = "publishing".into();
    fixture
        .service
        .store
        .put("journal", &restore_id, &restoring.account_id, &restoring)
        .unwrap();
    // Newer confirmed restore also reached its all-after boundary. Startup
    // visits it first; committing it changes the older source to restored.
    for path in &restore.paths {
        std::fs::remove_file(path).unwrap();
    }
    let store_root = fixture.service.store.root.clone();
    let vault = fixture.service.vault.clone();
    let paths = fixture.service.paths.clone();
    drop(fixture.service);
    let reopened = Service::new(Arc::new(Store::open(&store_root).unwrap()), vault, paths).unwrap();
    assert_eq!(
        reopened
            .store
            .get::<Journal>("journal", source_id)
            .unwrap()
            .unwrap()
            .phase,
        "restored"
    );
    assert_eq!(
        reopened
            .store
            .get::<Journal>("journal", &restore_id)
            .unwrap()
            .unwrap()
            .phase,
        "committed"
    );
    assert!(
        !reopened
            .pending_configuration_recovery(ClientKind::Claude)
            .unwrap()
    );
    assert_unobserved(&reopened, "not_configured");
    assert!(preview.paths.iter().all(|path| !Path::new(path).exists()));
    assert!(
        reopened
            .store
            .list::<HelperBinding>("binding", None)
            .unwrap()
            .iter()
            .all(|binding| !binding.enabled)
    );
}

fn frozen(fixture: &tests::PolicyFixture) -> ClaudeContext {
    let observation = fixture
        .service
        .observe_configuration(ClientKind::Claude)
        .unwrap();
    let target = observation.claude_configuration_target.as_ref().unwrap();
    let cache = fixture.service.account_cache(&fixture.record).unwrap();
    let node = fixture.record.view.node_id.as_ref().unwrap();
    let profile = uuid();
    let binding = uuid();
    let compiled = crate::claude_config::compile(&crate::claude_config::LibraryPlan {
        app_version: &target.app_version,
        source_member_sha256: &target.source_member_sha256,
        user_root: &target.config_root,
        profile_id: &profile,
        gateway_base_url: &cache.claude_endpoints[node],
        helper: &fixture.service.paths.helper,
        binding_id: &binding,
        model_id: &fixture.model,
        profile_before: None,
        metadata_before: None,
        main_before: None,
        active_profile: None,
    })
    .unwrap();
    fixture
        .service
        .freeze_claude(claude::Plan {
            record: &fixture.record,
            cache: &cache,
            compiled: &compiled,
            observation: &observation,
            binding_id: &binding,
            selected_model_id: &fixture.model,
        })
        .unwrap()
}

#[test]
fn claude_b3a_full_producer_qualifies_local_clear_remote_unknown_and_native_receipt() {
    let fixture = tests::policy_fixture(ClientKind::Claude);
    let mut paths = fixture.service.paths.clone();
    let adapter = paths.fixture_claude_configuration_target.clone().unwrap();
    let full = Arc::new(AtomicU64::new(0));
    let count = full.clone();
    paths.fixture_claude_configuration_target = Some(Arc::new(move |client, is_full| {
        if is_full {
            count.fetch_add(1, Ordering::SeqCst);
        }
        adapter(client, is_full)
    }));
    let service = Service::new(
        fixture.service.store.clone(),
        fixture.service.vault.clone(),
        paths,
    )
    .unwrap();
    full.store(0, Ordering::SeqCst);
    service.configuration_preflight(ClientKind::Claude).unwrap();
    service.configuration_preflight(ClientKind::Claude).unwrap();
    assert_eq!(full.load(Ordering::SeqCst), 2);
    let observed = service.observe_configuration(ClientKind::Claude).unwrap();
    assert_eq!(observed.target.managed, "unknown");
    assert!(observed.policy.user_config_state() == crate::policy::State::No);
    let preview = service
        .preview_client_config(
            &fixture.record.view.id,
            fixture.record.view.revision,
            ClientKind::Claude,
            &fixture.model,
        )
        .unwrap();
    assert!(preview.can_apply && !preview.can_export_import);
    let json = serde_json::to_value(&preview).unwrap();
    let receipt = &json["claude_configuration_target"];
    assert_eq!(receipt.as_object().unwrap().len(), 2);
    assert_eq!(
        receipt["app_version"],
        crate::claude_config::SUPPORTED_APP_VERSION
    );
    assert_eq!(receipt["revision"].as_str().unwrap().len(), 64);
    let mut legacy = json;
    legacy
        .as_object_mut()
        .unwrap()
        .remove("claude_configuration_target");
    assert!(
        serde_json::from_value::<ConfigPreview>(legacy)
            .unwrap()
            .claude_configuration_target
            .is_none()
    );
    assert!(!service.paths.claude_library.exists());
    assert!(
        service
            .store
            .list::<HelperBinding>("binding", None)
            .unwrap()
            .iter()
            .all(|grant| !grant.enabled)
    );
}

#[test]
fn claude_b3a_display_hint_never_replaces_full_authority_and_export_consumes_late_failure() {
    let fixture = tests::policy_fixture(ClientKind::Claude);
    let mut paths = fixture.service.paths.clone();
    let adapter = paths.fixture_claude_configuration_target.clone().unwrap();
    paths.fixture_claude_configuration_target = Some(Arc::new(move |client, full| {
        if full {
            Err(fail("claude_configuration_target_unverified"))
        } else {
            adapter(client, false)
        }
    }));
    let service = Service::new(
        fixture.service.store.clone(),
        fixture.service.vault.clone(),
        paths,
    )
    .unwrap();
    assert!(
        service
            .client_policy(ClientKind::Claude)
            .unwrap()
            .0
            .configuration_access
            == ConfigurationAccess::Allowed
    );
    assert_eq!(
        service
            .configuration_preflight(ClientKind::Claude)
            .unwrap_err()
            .code,
        "claude_configuration_target_unverified"
    );
    let blocked = service
        .preview_client_config(
            &fixture.record.view.id,
            fixture.record.view.revision,
            ClientKind::Claude,
            &fixture.model,
        )
        .unwrap();
    assert!(
        !blocked.can_apply
            && !blocked.can_export_import
            && blocked.claude_configuration_target.is_none()
    );
    assert!(
        service
            .store
            .list::<HelperBinding>("binding", None)
            .unwrap()
            .is_empty()
    );

    let preview = fixture
        .service
        .preview_claude_diagnostic(
            &fixture.record.view.id,
            fixture.record.view.revision,
            &fixture.model,
        )
        .unwrap();
    assert!(preview.can_export_import && !preview.can_apply);
    let path = fixture.service.store.root.join("late-diagnostic.json");
    fixture.target.store(1, Ordering::SeqCst);
    assert_eq!(
        fixture
            .service
            .export_claude_import(&preview.id, true, &path)
            .unwrap_err()
            .code,
        "claude_configuration_target_changed"
    );
    fixture.target.store(0, Ordering::SeqCst);
    assert_eq!(
        fixture
            .service
            .export_claude_import(&preview.id, true, &path)
            .unwrap_err()
            .code,
        "preview_expired"
    );
    assert!(!path.exists() && !fixture.service.paths.claude_library.exists());
}

#[test]
fn claude_b3a_late_scope_changes_refuse_apply_while_restore_ignores_live_authority() {
    for fault in [
        "target",
        "policy",
        "helper",
        "epoch",
        "actor",
        "session",
        "account",
        "credential",
        "key",
        "expiry",
        "node",
        "node_revision",
        "endpoint",
        "model",
        "catalog",
    ] {
        let fixture = tests::policy_fixture(ClientKind::Claude);
        let context = frozen(&fixture);
        assert_eq!(context.claude_base_url, "https://edge.example.test");
        assert_eq!(
            fixture.record.view.edge_base_url.as_deref(),
            Some("https://edge.example.test/v1")
        );
        let mut record = fixture.record.clone();
        let mut cache = fixture.service.account_cache(&record).unwrap();
        match fault {
            "target" => fixture.target.store(1, Ordering::SeqCst),
            "policy" => fixture.epoch.store(1, Ordering::SeqCst),
            "helper" => std::fs::write(&fixture.service.paths.helper, b"replaced helper").unwrap(),
            "epoch" => fixture
                .service
                .store
                .set_meta(
                    &authorization_epoch_reference(
                        &context.panel_origin,
                        &context.installation_id,
                        &context.account_id,
                    ),
                    "2",
                )
                .unwrap(),
            "actor" => fixture
                .service
                .store
                .set_meta("active_connection", "")
                .unwrap(),
            "session" => record.session_status = "logged_out".into(),
            "account" => record.view.account_id = "different-actor".into(),
            "credential" => record.credential_reference = Some("different-secret-reference".into()),
            "key" => {
                assert!(!cache.keys.is_empty());
                cache.keys[0].state = "revoked".into();
            }
            "expiry" => record.credential_expires_at = Some(chrono::Utc::now().timestamp() - 1),
            "node" => cache.nodes[0].available = false,
            "node_revision" => cache.nodes[0].revision.push_str("-changed"),
            "endpoint" => {
                cache.claude_endpoints.insert(
                    context.node_id.clone(),
                    "https://different.claude.test".into(),
                );
            }
            "model" => cache.models[0].revision.push_str("-changed"),
            "catalog" => cache.catalog_revision.push_str("-changed"),
            _ => unreachable!(),
        }
        fixture
            .service
            .store
            .put(
                "connection",
                &record.view.id,
                &record.view.account_id,
                &record,
            )
            .unwrap();
        fixture
            .service
            .store
            .put(
                "account-cache",
                &record.view.id,
                &record.view.account_id,
                &cache,
            )
            .unwrap();
        assert!(
            fixture
                .service
                .validate_claude_context(&context, false)
                .is_err(),
            "fault {fault}"
        );
        if fault == "model" {
            assert_eq!(
                fixture
                    .service
                    .validate_claude_context(&context, false)
                    .unwrap_err()
                    .code,
                "claude_configuration_context_changed"
            );
        }
        let restoration = fixture.service.validate_claude_context(&context, true);
        if ["target", "policy"].contains(&fault) {
            assert!(restoration.is_err());
        } else {
            assert!(
                restoration.is_ok(),
                "restore depends on {fault}: {restoration:?}"
            );
        }
        assert!(!fixture.service.paths.claude_library.exists());
        assert!(
            fixture
                .service
                .store
                .list::<HelperBinding>("binding", None)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn claude_b3a_previous_grants_are_only_same_client_origin_actor_and_installation() {
    let fixture = tests::policy_fixture(ClientKind::Claude);
    let context = frozen(&fixture);
    let mut expected = Vec::new();
    for fault in [
        "same",
        "provider",
        "client",
        "origin",
        "account",
        "installation",
        "disabled",
    ] {
        let mut grant = HelperBinding {
            id: uuid(),
            client: ClientKind::Claude,
            origin: context.panel_origin.clone(),
            account_id: context.account_id.clone(),
            installation_id: context.installation_id.clone(),
            provider_id: context.provider_id.clone(),
            credential_revision: context.credential_revision.clone(),
            secret_reference: context.credential_reference.clone(),
            enabled: true,
            expires_at: None,
            authorization_epoch: context.authorization_epoch,
        };
        match fault {
            "same" => {}
            "provider" => grant.provider_id = "old-provider".into(),
            "client" => grant.client = ClientKind::Codex,
            "origin" => grant.origin = "https://different.panel.test".into(),
            "account" => grant.account_id = "different-actor".into(),
            "installation" => grant.installation_id = uuid(),
            "disabled" => grant.enabled = false,
            _ => unreachable!(),
        }
        if ["same", "provider"].contains(&fault) {
            expected.push(grant.id.clone());
        }
        fixture
            .service
            .store
            .put("binding", &grant.id, &grant.account_id, &grant)
            .unwrap();
    }
    let mut actual = fixture.service.claude_previous_bindings(&context).unwrap();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
}

#[test]
fn claude_b3a_policy_changed_inside_full_producer_fails_before_vault_or_refresh() {
    let fixture = tests::policy_fixture(ClientKind::Claude);
    let mut paths = fixture.service.paths.clone();
    let adapter = paths.fixture_claude_configuration_target.clone().unwrap();
    let policy_signal = fixture.epoch.clone();
    paths.fixture_claude_configuration_target = Some(Arc::new(move |client, full| {
        let receipt = adapter(client, full)?;
        if full {
            policy_signal.store(1, Ordering::SeqCst);
        }
        Ok(receipt)
    }));
    let service = Service::new(
        fixture.service.store.clone(),
        fixture.service.vault.clone(),
        paths,
    )
    .unwrap();
    fixture.vault.calls.store(0, Ordering::SeqCst);
    let revision = service.store.snapshot().unwrap().revision;
    assert_eq!(
        service
            .configuration_preflight(ClientKind::Claude)
            .unwrap_err()
            .code,
        "managed_policy_changed"
    );
    assert_eq!(fixture.vault.calls.load(Ordering::SeqCst), 0);
    assert_eq!(service.store.snapshot().unwrap().revision, revision);
    assert!(!service.paths.claude_library.exists());
}

#[test]
fn claude_b3a_diagnostic_vault_return_rechecks_actor_endpoint_and_fresh_expiry() {
    for fault in ["actor", "endpoint", "expiry"] {
        let fixture = tests::policy_fixture(ClientKind::Claude);
        let expiry = chrono::Utc::now().timestamp() + 3;
        if fault == "expiry" {
            let mut record = fixture.record.clone();
            record.credential_expires_at = Some(expiry);
            let mut cache = fixture.service.account_cache(&record).unwrap();
            let key = cache
                .keys
                .iter_mut()
                .find(|key| {
                    key.api_key_id == record.api_key_id
                        && key.key_revision == record.view.credential_revision
                })
                .unwrap();
            key.expires_at = Some(expiry);
            fixture
                .service
                .store
                .put(
                    "connection",
                    &record.view.id,
                    &record.view.account_id,
                    &record,
                )
                .unwrap();
            fixture
                .service
                .store
                .put(
                    "account-cache",
                    &record.view.id,
                    &record.view.account_id,
                    &cache,
                )
                .unwrap();
            let reference = record.credential_reference.as_ref().unwrap();
            let bytes = fixture.service.vault.get(reference).unwrap();
            let mut secret: InferenceSecret = serde_json::from_slice(&bytes).unwrap();
            secret.expires_at = Some(expiry);
            fixture
                .service
                .vault
                .put(reference, &serde_json::to_vec(&secret).unwrap())
                .unwrap();
        }
        let preview = fixture
            .service
            .preview_claude_diagnostic(
                &fixture.record.view.id,
                fixture.record.view.revision,
                &fixture.model,
            )
            .unwrap();
        assert!(preview.can_export_import && !preview.can_apply);
        let path = fixture
            .service
            .store
            .root
            .join("late-vault-diagnostic.json");
        let worker_path = path.clone();
        let (entered, waiting) = std::sync::mpsc::channel();
        let (resume, released) = std::sync::mpsc::channel();
        *fixture.vault.pause.lock().unwrap() = Some((entered, released));
        fixture.vault.calls.store(0, Ordering::SeqCst);
        fixture.vault.mutations.store(0, Ordering::SeqCst);
        let service = fixture.service.clone();
        let id = preview.id.clone();
        let thread =
            std::thread::spawn(move || service.export_claude_import(&id, true, &worker_path));
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        match fault {
            "actor" => fixture
                .service
                .store
                .set_meta("active_connection", "")
                .unwrap(),
            "endpoint" => {
                let mut cache = fixture.service.account_cache(&fixture.record).unwrap();
                cache.claude_endpoints.insert(
                    fixture.record.view.node_id.as_ref().unwrap().clone(),
                    "https://late-different.claude.test".into(),
                );
                fixture
                    .service
                    .store
                    .put(
                        "account-cache",
                        &fixture.record.view.id,
                        &fixture.record.view.account_id,
                        &cache,
                    )
                    .unwrap();
            }
            "expiry" => {
                while chrono::Utc::now().timestamp() <= expiry {
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
            _ => unreachable!(),
        }
        resume.send(()).unwrap();
        assert!(thread.join().unwrap().is_err(), "fault {fault}");
        assert_eq!(fixture.vault.calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.vault.mutations.load(Ordering::SeqCst), 0);
        assert!(!path.exists() && !fixture.service.paths.claude_library.exists());
        assert!(
            fixture
                .service
                .store
                .list::<HelperBinding>("binding", None)
                .unwrap()
                .iter()
                .all(|grant| !grant.enabled)
        );
        assert_eq!(
            fixture
                .service
                .export_claude_import(&preview.id, true, &path)
                .unwrap_err()
                .code,
            "preview_expired"
        );
    }
}

#[test]
fn claude_b3a_diagnostic_alias_traversal_and_late_parent_swap_never_write_client_files() {
    use std::os::unix::fs::{MetadataExt, symlink};
    for fault in [
        "claude_alias",
        "codex_alias",
        "claude_case",
        "codex_case",
        "traversal",
        "outside_alias",
        "late_alias",
    ] {
        let fixture = tests::policy_fixture(ClientKind::Claude);
        let root = fixture.service.paths.claude_library.parent().unwrap();
        std::fs::create_dir_all(root).unwrap();
        let main = root.join("claude_desktop_config.json");
        let main_before = br#"{"deploymentMode":"1p","sentinel":"original"}"#;
        std::fs::write(&main, main_before).unwrap();
        let codex_before = std::fs::read(&fixture.service.paths.codex_config).unwrap();
        let alias = fixture.service.store.root.join("export-alias");
        let outside = fixture
            .service
            .store
            .root
            .parent()
            .unwrap()
            .join("outside-export");
        std::fs::create_dir(&outside).unwrap();
        let safe = fixture.service.store.root.join("safe-export");
        std::fs::create_dir(&safe).unwrap();
        let path = match fault {
            "claude_case" => root
                .with_file_name("cLaUdE-3P")
                .join("CLAUDE_DESKTOP_CONFIG.JSON"),
            "codex_case" => fixture
                .service
                .paths
                .codex_config
                .with_file_name("CONFIG.TOML"),
            "traversal" => {
                let prefix = fixture.service.store.root.join("traversal");
                std::fs::create_dir(&prefix).unwrap();
                prefix
                    .join("..")
                    .join(main.strip_prefix(&fixture.service.store.root).unwrap())
            }
            "codex_alias" => {
                symlink(fixture.service.paths.codex_config.parent().unwrap(), &alias).unwrap();
                alias.join("config.toml")
            }
            "outside_alias" => {
                symlink(&outside, &alias).unwrap();
                alias.join("diagnostic.json")
            }
            "late_alias" => {
                symlink(&safe, &alias).unwrap();
                alias.join("claude_desktop_config.json")
            }
            _ => {
                symlink(root, &alias).unwrap();
                alias.join("claude_desktop_config.json")
            }
        };
        if ["claude_case", "codex_case"].contains(&fault) {
            let protected = if fault == "claude_case" {
                &main
            } else {
                &fixture.service.paths.codex_config
            };
            let original = std::fs::metadata(protected).unwrap();
            let aliased = std::fs::metadata(&path).unwrap();
            assert_eq!(
                (original.dev(), original.ino()),
                (aliased.dev(), aliased.ino())
            );
        }
        let preview = fixture
            .service
            .preview_claude_diagnostic(
                &fixture.record.view.id,
                fixture.record.view.revision,
                &fixture.model,
            )
            .unwrap();
        assert!(preview.can_export_import && !preview.can_apply);
        fixture.vault.calls.store(0, Ordering::SeqCst);
        fixture.vault.mutations.store(0, Ordering::SeqCst);
        let failure = if fault == "late_alias" {
            let (entered, waiting) = std::sync::mpsc::channel();
            let (resume, released) = std::sync::mpsc::channel();
            *fixture.vault.pause.lock().unwrap() = Some((entered, released));
            let worker = fixture.service.clone();
            let worker_path = path.clone();
            let id = preview.id.clone();
            let thread =
                std::thread::spawn(move || worker.export_claude_import(&id, true, &worker_path));
            waiting.recv_timeout(Duration::from_secs(5)).unwrap();
            std::fs::remove_file(&alias).unwrap();
            symlink(root, &alias).unwrap();
            resume.send(()).unwrap();
            thread.join().unwrap().unwrap_err()
        } else {
            fixture
                .service
                .export_claude_import(&preview.id, true, &path)
                .unwrap_err()
        };
        assert_eq!(failure.code, "invalid_path", "fault {fault}");
        assert_eq!(std::fs::read(&main).unwrap(), main_before);
        assert_eq!(
            std::fs::read(&fixture.service.paths.codex_config).unwrap(),
            codex_before
        );
        assert!(!outside.join("diagnostic.json").exists());
        assert!(!safe.join("claude_desktop_config.json").exists());
        assert_eq!(
            fixture.vault.calls.load(Ordering::SeqCst),
            u64::from(fault == "late_alias")
        );
        assert_eq!(fixture.vault.mutations.load(Ordering::SeqCst), 0);
        assert!(
            fixture
                .service
                .store
                .list::<HelperBinding>("binding", None)
                .unwrap()
                .iter()
                .all(|binding| !binding.enabled)
        );
        assert_eq!(
            fixture
                .service
                .export_claude_import(&preview.id, true, &path)
                .unwrap_err()
                .code,
            "preview_expired"
        );
    }
}
