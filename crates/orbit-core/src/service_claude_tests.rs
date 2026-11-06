use super::*;
use crate::config::ClaudeContext;

#[test]
fn claude_core_flow_bootstrap_qualifies_legacy_state_and_replaces_obsolete_guidance() {
    let f = tests::policy_fixture(ClientKind::Claude);
    let mut paths = f.service.paths.clone();
    let adapter = paths.fixture_claude_configuration_target.clone().unwrap();
    let full = Arc::new(AtomicU64::new(0));
    let count = full.clone();
    paths.fixture_claude_configuration_target = Some(Arc::new(move |client, is_full| {
        if is_full {
            count.fetch_add(1, Ordering::SeqCst);
        }
        adapter(client, is_full)
    }));
    let service = Service::new(f.service.store.clone(), f.vault.clone(), paths).unwrap();
    let mut legacy: ClientView = service
        .store
        .get("client-state", "claude")
        .unwrap()
        .unwrap();
    legacy.configuration_state = "unsupported".into();
    legacy.manual_steps = vec!["Claude 第三方推理模式影响整个桌面应用；configLibrary envelope 尚未验证，请使用官方导入流程。".into(), "保留已有状态说明".into()];
    service
        .store
        .put("client-state", "claude", "local", &legacy)
        .unwrap();
    let revision = service.store.snapshot().unwrap().revision;
    f.vault.calls.store(0, Ordering::SeqCst);
    f.vault.mutations.store(0, Ordering::SeqCst);
    full.store(0, Ordering::SeqCst);
    let first = service
        .bootstrap()
        .unwrap()
        .clients
        .into_iter()
        .find(|client| client.client == ClientKind::Claude)
        .unwrap();
    assert_eq!(first.configuration_state, "not_configured");
    assert_eq!(first.configuration_access, ConfigurationAccess::Allowed);
    assert_eq!(first.managed, "unknown");
    assert!(
        first
            .manual_steps
            .iter()
            .any(|note| note.contains("Chat、Code 与 Cowork"))
    );
    assert!(
        first
            .manual_steps
            .iter()
            .any(|note| note.contains("预览三文件")
                && note.contains("确认保存")
                && note.contains("重启"))
    );
    assert!(
        first
            .manual_steps
            .iter()
            .any(|note| note == "保留已有状态说明")
    );
    assert!(
        !first
            .manual_steps
            .iter()
            .any(|note| note.contains("envelope") || note.contains("官方导入"))
    );
    assert_eq!(first.loaded_state, "unconfirmed");
    assert_eq!(first.connection_state, "not_tested");
    assert!(first.loaded_evidence.is_none() && first.connection_evidence.is_none());
    let second = service
        .bootstrap()
        .unwrap()
        .clients
        .into_iter()
        .find(|client| client.client == ClientKind::Claude)
        .unwrap();
    assert_eq!(second.configuration_state, first.configuration_state);
    assert_eq!(second.manual_steps, first.manual_steps);
    assert_eq!(
        full.load(Ordering::SeqCst),
        2,
        "each legacy projection requires a fresh full receipt"
    );
    assert_eq!(service.store.snapshot().unwrap().revision, revision);
    assert_eq!(
        service
            .store
            .get::<ClientView>("client-state", "claude")
            .unwrap()
            .unwrap()
            .configuration_state,
        "unsupported",
        "projection does not claim a durable save"
    );
    assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.vault.mutations.load(Ordering::SeqCst), 0);
    assert!(!service.paths.claude_library.parent().unwrap().exists());
}

#[test]
fn claude_core_flow_bootstrap_keeps_unqualified_or_pending_legacy_state_conservative() {
    for fault in [
        "full-target",
        "version",
        "managed",
        "policy-unknown",
        "late-policy",
        "source-format",
        "pending",
    ] {
        let f = tests::policy_fixture(ClientKind::Claude);
        let mut paths = f.service.paths.clone();
        let adapter = paths.fixture_claude_configuration_target.clone().unwrap();
        let signal = f.epoch.clone();
        paths.fixture_claude_configuration_target = Some(Arc::new(move |client, full| {
            if full && fault == "full-target" {
                return Err(fail("claude_configuration_target_unverified"));
            }
            let target = adapter(client, full)?;
            if full && fault == "late-policy" {
                signal.store(1, Ordering::SeqCst);
            }
            Ok(target)
        }));
        let service = Service::new(f.service.store.clone(), f.vault.clone(), paths).unwrap();
        if fault == "managed" {
            f.epoch.store(2, Ordering::SeqCst);
        }
        if fault == "policy-unknown" {
            f.epoch.store(3, Ordering::SeqCst);
        }
        if fault == "version" {
            let mut legacy: ClientView = service
                .store
                .get("client-state", "claude")
                .unwrap()
                .unwrap();
            legacy.version = Some("unsupported-version".into());
            service
                .store
                .put("client-state", "claude", "local", &legacy)
                .unwrap();
        }
        if fault == "source-format" {
            std::fs::create_dir_all(&service.paths.claude_library).unwrap();
            std::fs::write(
                service.paths.claude_library.join("_meta.json"),
                b"unrecognized metadata",
            )
            .unwrap();
        }
        if fault == "pending" {
            let preview = automatic_preview(&service, &f);
            let mut journal = preview_journal(&service, &preview);
            journal.phase = "publishing".into();
            service
                .store
                .put("journal", &journal.id, &journal.account_id, &journal)
                .unwrap();
        }
        f.vault.calls.store(0, Ordering::SeqCst);
        f.vault.mutations.store(0, Ordering::SeqCst);
        let revision = service.store.snapshot().unwrap().revision;
        let client = service
            .bootstrap()
            .unwrap()
            .clients
            .into_iter()
            .find(|client| client.client == ClientKind::Claude)
            .unwrap();
        assert_eq!(client.configuration_state, "unsupported", "{fault}");
        assert!(
            client
                .manual_steps
                .iter()
                .any(|note| note.contains("未通过核验时自动接入保持禁用")),
            "{fault}"
        );
        assert!(
            !client
                .manual_steps
                .iter()
                .any(|note| note.contains("预览三文件")),
            "{fault}"
        );
        assert!(client.loaded_evidence.is_none() && client.connection_evidence.is_none());
        assert_eq!(client.loaded_state, "unconfirmed");
        assert_eq!(client.connection_state, "not_tested");
        if fault == "managed" {
            assert_eq!(client.configuration_access, ConfigurationAccess::Blocked);
        }
        if fault == "policy-unknown" || fault == "version" {
            assert_eq!(client.configuration_access, ConfigurationAccess::Unverified);
        }
        assert_eq!(service.store.snapshot().unwrap().revision, revision);
        assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0, "{fault}");
        assert_eq!(f.vault.mutations.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn claude_core_flow_bootstrap_preserves_saved_conflict_and_recovery_instructions() {
    let f = tests::policy_fixture(ClientKind::Claude);
    let preview = automatic_preview(&f.service, &f);
    f.service
        .apply_client_config(&preview.id, true, false)
        .unwrap();
    let mut saved: ClientView = f
        .service
        .store
        .get("client-state", "claude")
        .unwrap()
        .unwrap();
    let saved_revision = saved.configuration_revision.clone();
    for state in ["saved", "conflict"] {
        saved.configuration_state = state.into();
        f.service
            .store
            .put("client-state", "claude", "local", &saved)
            .unwrap();
        let reopened = Service::new(
            f.service.store.clone(),
            f.service.vault.clone(),
            f.service.paths.clone(),
        )
        .unwrap();
        let mut current: ClientView = reopened
            .store
            .get("client-state", "claude")
            .unwrap()
            .unwrap();
        current
            .manual_steps
            .push("整组配置操作尚待恢复，请保留现有恢复记录。".into());
        reopened
            .store
            .put("client-state", "claude", "local", &current)
            .unwrap();
        f.vault.calls.store(0, Ordering::SeqCst);
        let revision = reopened.store.snapshot().unwrap().revision;
        let client = reopened
            .bootstrap()
            .unwrap()
            .clients
            .into_iter()
            .find(|client| client.client == ClientKind::Claude)
            .unwrap();
        assert_eq!(client.configuration_state, state);
        assert_eq!(client.configuration_revision, saved_revision);
        assert!(
            client
                .manual_steps
                .iter()
                .any(|note| note.contains("整组配置操作尚待恢复"))
        );
        assert!(client.loaded_evidence.is_none() && client.connection_evidence.is_none());
        assert_eq!(client.connection_state, "not_tested");
        assert_eq!(
            client.loaded_state,
            if state == "saved" {
                "restart_required"
            } else {
                "unconfirmed"
            }
        );
        assert_eq!(reopened.store.snapshot().unwrap().revision, revision);
        assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0);
    }
}

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

#[test]
fn claude_engine_full_verification_budget_preserves_three_file_roundtrip() {
    for existing in [false, true] {
        let f = tests::policy_fixture(ClientKind::Claude);
        let mut paths = f.service.paths.clone();
        let adapter = paths.fixture_claude_configuration_target.clone().unwrap();
        let full = Arc::new(AtomicU64::new(0));
        let count = full.clone();
        paths.fixture_claude_configuration_target = Some(Arc::new(move |client, is_full| {
            if is_full {
                count.fetch_add(1, Ordering::SeqCst);
            }
            adapter(client, is_full)
        }));
        let service = Service::new(f.service.store.clone(), f.vault.clone(), paths).unwrap();
        let main = service
            .paths
            .claude_library
            .parent()
            .unwrap()
            .join("claude_desktop_config.json");
        let metadata = service.paths.claude_library.join("_meta.json");
        if existing {
            std::fs::create_dir_all(&service.paths.claude_library).unwrap();
            std::fs::write(&main, br#"{"deploymentMode":"1p","unowned":true}"#).unwrap();
        }
        let originals = (read_file(&main).unwrap(), read_file(&metadata).unwrap());
        full.store(0, Ordering::SeqCst);
        let preview = automatic_preview(&service, &f);
        let preview_full = full.swap(0, Ordering::SeqCst);
        assert_eq!(read_file(&main).unwrap(), originals.0);
        assert_eq!(read_file(&metadata).unwrap(), originals.1);
        let saved = service
            .apply_client_config(&preview.id, true, false)
            .unwrap();
        let apply_full = full.swap(0, Ordering::SeqCst);
        let restore = service
            .preview_restore(saved.backup_group_id.as_ref().unwrap())
            .unwrap();
        let restore_preview_full = full.swap(0, Ordering::SeqCst);
        service
            .apply_client_config(&restore.id, true, true)
            .unwrap();
        let restore_full = full.load(Ordering::SeqCst);
        assert_eq!(read_file(&main).unwrap(), originals.0);
        assert_eq!(read_file(&metadata).unwrap(), originals.1);
        assert!(!Path::new(&preview.paths[0]).exists());
        assert!(
            service
                .store
                .list::<HelperBinding>("binding", None)
                .unwrap()
                .iter()
                .all(|binding| !binding.enabled)
        );
        println!(
            "existing={existing}: preview={preview_full}, apply={apply_full}, restore_preview={restore_preview_full}, restore={restore_full}"
        );
        // These are upper budgets for fresh full producers, not cached receipts.
        // Both existing and first-run roots must retain full boundary checks.
        assert!(
            preview_full <= 20
                && apply_full <= 40
                && restore_preview_full <= 15
                && restore_full <= 40,
            "full producer budget exceeded for existing={existing}: {preview_full}/{apply_full}/{restore_preview_full}/{restore_full}"
        );
    }
}

#[test]
fn claude_engine_allowed_hint_cannot_publish_after_full_failure_at_vault_return() {
    let f = tests::policy_fixture(ClientKind::Claude);
    let mut paths = f.service.paths.clone();
    let adapter = paths.fixture_claude_configuration_target.clone().unwrap();
    let fail_full = Arc::new(AtomicU64::new(0));
    let signal = fail_full.clone();
    paths.fixture_claude_configuration_target = Some(Arc::new(move |client, full| {
        if full && signal.load(Ordering::SeqCst) == 1 {
            return Err(fail("claude_signature_verification_timeout"));
        }
        adapter(client, full)
    }));
    let service = Arc::new(Service::new(f.service.store.clone(), f.vault.clone(), paths).unwrap());
    let preview = automatic_preview(&service, &f);
    let journal = preview_journal(&service, &preview);
    let before_grants = grants(&service);
    let (entered, waiting) = std::sync::mpsc::channel();
    let (resume, released) = std::sync::mpsc::channel();
    *f.vault.pause.lock().unwrap() = Some((entered, released));
    f.vault.mutations.store(0, Ordering::SeqCst);
    let worker = service.clone();
    let id = preview.id.clone();
    let thread = std::thread::spawn(move || worker.apply_client_config(&id, true, false));
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    fail_full.store(1, Ordering::SeqCst);
    assert_eq!(
        service
            .client_policy(ClientKind::Claude)
            .unwrap()
            .0
            .configuration_access,
        ConfigurationAccess::Allowed
    );
    resume.send(()).unwrap();
    assert_eq!(
        thread.join().unwrap().unwrap_err().code,
        "claude_signature_verification_timeout"
    );
    assert!(!service.paths.claude_library.parent().unwrap().exists());
    assert_eq!(grants(&service), before_grants);
    assert_eq!(f.vault.mutations.load(Ordering::SeqCst), 0);
    assert_eq!(
        service
            .store
            .get::<Journal>("journal", &journal.id)
            .unwrap()
            .unwrap()
            .phase,
        "prepared"
    );
    assert!(
        service
            .store
            .list::<Journal>("backup", None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn claude_engine_restore_preview_rechecks_full_after_final_backup_key_return() {
    struct FinalBackupVault {
        inner: Arc<tests::PolicyVault>,
        backup_reference: String,
        backup_reads: AtomicU64,
        fail_full: Arc<AtomicU64>,
    }
    impl Vault for FinalBackupVault {
        fn get(&self, reference: &str) -> Result<Zeroizing<Vec<u8>>> {
            let bytes = self.inner.get(reference)?;
            if reference == self.backup_reference
                && self.backup_reads.fetch_add(1, Ordering::SeqCst) == 4
            {
                self.fail_full.store(1, Ordering::SeqCst);
            }
            Ok(bytes)
        }
        fn put(&self, reference: &str, bytes: &[u8]) -> Result<()> {
            self.inner.put(reference, bytes)
        }
        fn delete(&self, reference: &str) -> Result<()> {
            self.inner.delete(reference)
        }
    }
    let f = tests::policy_fixture(ClientKind::Claude);
    let preview = automatic_preview(&f.service, &f);
    let saved = f
        .service
        .apply_client_config(&preview.id, true, false)
        .unwrap();
    let before_files: Vec<_> = preview
        .paths
        .iter()
        .map(|path| read_file(Path::new(path)).unwrap())
        .collect();
    let before_grants = grants(&f.service);
    let fail_full = Arc::new(AtomicU64::new(0));
    let mut paths = f.service.paths.clone();
    let adapter = paths.fixture_claude_configuration_target.clone().unwrap();
    let signal = fail_full.clone();
    paths.fixture_claude_configuration_target = Some(Arc::new(move |client, full| {
        if full && signal.load(Ordering::SeqCst) == 1 {
            return Err(fail("claude_signature_verification_timeout"));
        }
        adapter(client, full)
    }));
    let vault = Arc::new(FinalBackupVault {
        inner: f.vault.clone(),
        backup_reference: f.service.installation(&f.record).unwrap().backup_reference,
        backup_reads: AtomicU64::new(0),
        fail_full,
    });
    let service = Service::new(f.service.store.clone(), vault.clone(), paths).unwrap();
    vault.backup_reads.store(0, Ordering::SeqCst);
    f.vault.mutations.store(0, Ordering::SeqCst);
    assert_eq!(
        service
            .preview_restore(saved.backup_group_id.as_ref().unwrap())
            .unwrap_err()
            .code,
        "claude_signature_verification_timeout"
    );
    assert_eq!(
        vault.backup_reads.load(Ordering::SeqCst),
        5,
        "fault occurs at the final backup-key return, after earlier full checks passed"
    );
    assert_eq!(
        service
            .client_policy(ClientKind::Claude)
            .unwrap()
            .0
            .configuration_access,
        ConfigurationAccess::Allowed
    );
    assert!(
        service.previews.lock().unwrap().is_empty(),
        "a failed final full check cannot yield an applicable preview"
    );
    for (path, before) in preview.paths.iter().zip(before_files) {
        assert_eq!(read_file(Path::new(path)).unwrap(), before);
    }
    assert_eq!(grants(&service), before_grants);
    assert_eq!(f.vault.mutations.load(Ordering::SeqCst), 0);
}

#[test]
fn production_panel_origin_preserves_legacy_offline_claude_backup_restore() {
    let mut fixture = tests::policy_fixture(ClientKind::Claude);
    let first = automatic_preview(&fixture.service, &fixture);
    fixture
        .service
        .apply_client_config(&first.id, true, false)
        .unwrap();
    let original: Vec<_> = first
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
    let http = Http::new().unwrap();
    let calls = Arc::new(AtomicU64::new(0));
    let sent = calls.clone();
    http.scripted(Arc::new(move |_| {
        sent.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(fail("unexpected_transport")) })
    }));
    Arc::get_mut(&mut fixture.service).unwrap().http = http;
    let restore = fixture
        .service
        .preview_restore(saved.backup_group_id.as_ref().unwrap())
        .unwrap();
    assert!(restore.can_apply);
    let result = fixture
        .service
        .apply_client_config(&restore.id, true, true)
        .unwrap();
    assert_eq!(result.state, "restored");
    for (path, bytes) in first.paths.iter().zip(original) {
        assert_eq!(read_file(Path::new(path)).unwrap(), bytes);
    }
    assert!(!Path::new(&second.paths[0]).exists());
    assert!(
        fixture
            .service
            .store
            .list::<HelperBinding>("binding", None)
            .unwrap()
            .iter()
            .all(|binding| !binding.enabled)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
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

#[tokio::test]
async fn claude_b3c_command_refresh_accepts_independent_bootstrap_and_models_revisions() {
    let fixture = tests::policy_fixture(ClientKind::Claude);
    let wire: Value =
        serde_json::from_str(include_str!("../tests/fixtures/public-wire.json")).unwrap();
    let bootstrap = wire["bootstrap_reply"].clone();
    let models_revision = sha256(b"synthetic independent models endpoint revision");
    assert_ne!(
        bootstrap["data"]["meta"]["catalog_revision"],
        models_revision
    );
    let calls = Arc::new(Mutex::new(Vec::new()));
    let requests = calls.clone();
    fixture.service.http.scripted(Arc::new(move |request| {
        assert_eq!(request.method, Method::GET);
        let response = if request.url.ends_with("/api/v1/orbit/bootstrap") {
            requests.lock().unwrap().push("bootstrap");
            bootstrap.clone()
        } else {
            assert!(request.url.contains("/api/v1/orbit/models?node_id="));
            requests.lock().unwrap().push("models");
            json!({"data": bootstrap["data"]["models"], "meta":{"revision":models_revision}})
        };
        Box::pin(async move {
            Ok(crate::network::RemoteResponse {
                status: 200,
                request_id: None,
                bytes: serde_json::to_vec(&response).unwrap(),
            })
        })
    }));
    // This is the Service entry used by the Tauri preview/apply commands.
    let preview = fixture
        .service
        .preview_client_config_refreshed(
            &fixture.record.view.id,
            fixture.record.view.revision,
            ClientKind::Claude,
            &fixture.model,
        )
        .await
        .unwrap();
    assert!(preview.can_apply && preview.paths.len() == 3);
    assert!(!fixture.service.paths.claude_library.exists());
    let saved = fixture
        .service
        .apply_client_config_refreshed(&preview.id, true)
        .await
        .unwrap();
    assert_eq!(saved.state, "saved");
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        ["bootstrap", "models", "bootstrap", "models"]
    );
    assert!(preview.paths.iter().all(|path| Path::new(path).is_file()));
    assert_unobserved(&fixture.service, "saved");
    let restore = fixture
        .service
        .preview_restore(saved.backup_group_id.as_ref().unwrap())
        .unwrap();
    fixture
        .service
        .apply_client_config(&restore.id, true, true)
        .unwrap();
    assert!(preview.paths.iter().all(|path| !Path::new(path).exists()));
}

fn preview_journal(service: &Service, preview: &ConfigPreview) -> Journal {
    let id = service
        .previews
        .lock()
        .unwrap()
        .get(&preview.id)
        .unwrap()
        .journal_id
        .clone()
        .unwrap();
    service.store.get("journal", &id).unwrap().unwrap()
}

fn grants(service: &Service) -> Value {
    serde_json::to_value(
        service
            .store
            .list::<HelperBinding>("binding", None)
            .unwrap(),
    )
    .unwrap()
}

fn set_claude_backup_expiry(service: &Service, id: &str, expires_at: &str) {
    for kind in ["journal", "backup"] {
        let mut source: Journal = service.store.get(kind, id).unwrap().unwrap();
        source.expires_at = expires_at.into();
        service
            .store
            .put(kind, id, &source.account_id, &source)
            .unwrap();
    }
}

fn wait_for_claude_backup_expiry(expires_at: &str) {
    let deadline = chrono::DateTime::parse_from_rfc3339(expires_at).unwrap();
    let limit = std::time::Instant::now() + Duration::from_secs(5);
    while chrono::Utc::now() <= deadline {
        assert!(std::time::Instant::now() < limit, "bounded expiry wait");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn claude_restore_preview_binds_exact_selected_source_and_uses_its_shorter_deadline() {
    let f = tests::policy_fixture(ClientKind::Claude);
    let apply = automatic_preview(&f.service, &f);
    assert!(serde_json::to_value(&apply).unwrap()["restore_source"].is_null());
    let saved = f
        .service
        .apply_client_config(&apply.id, true, false)
        .unwrap();
    let source_id = saved.backup_group_id.unwrap();
    let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc3339();
    set_claude_backup_expiry(&f.service, &source_id, &expires_at);
    let restore = f.service.preview_restore(&source_id).unwrap();
    assert!(restore.can_apply);
    assert_eq!(
        serde_json::to_value(&restore).unwrap()["restore_source"],
        json!({"backup_group_id": source_id, "expires_at": expires_at})
    );
    assert_eq!(restore.expires_at, expires_at);
    assert!(apply.paths.iter().all(|path| Path::new(path).is_file()));
    let diagnostic = f
        .service
        .preview_claude_diagnostic(&f.record.view.id, f.record.view.revision, &f.model)
        .unwrap();
    assert!(serde_json::to_value(&diagnostic).unwrap()["restore_source"].is_null());
    let mut legacy = serde_json::to_value(&restore).unwrap();
    legacy.as_object_mut().unwrap().remove("restore_source");
    assert!(
        serde_json::from_value::<ConfigPreview>(legacy)
            .unwrap()
            .restore_source
            .is_none()
    );
}

#[test]
fn claude_restore_prepared_after_source_expiry_refuses_before_file_or_grant_writes() {
    let f = tests::policy_fixture(ClientKind::Claude);
    let apply = automatic_preview(&f.service, &f);
    let saved = f
        .service
        .apply_client_config(&apply.id, true, false)
        .unwrap();
    let source_id = saved.backup_group_id.unwrap();
    let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(2)).to_rfc3339();
    set_claude_backup_expiry(&f.service, &source_id, &expires_at);
    let restore = f.service.preview_restore(&source_id).unwrap();
    let journal = preview_journal(&f.service, &restore);
    assert_eq!(journal.phase, "prepared");
    let before: Vec<_> = apply
        .paths
        .iter()
        .map(|path| read_file(Path::new(path)).unwrap())
        .collect();
    let before_grants = grants(&f.service);
    wait_for_claude_backup_expiry(&expires_at);
    let observation = f.service.observe_configuration(ClientKind::Claude).unwrap();
    let guard = || {
        f.service.validate_claude_configuration(
            &observation.policy,
            observation.claude_configuration_target.as_ref(),
        )
    };
    let context = |context: &ClaudeContext, restoring: bool| {
        f.service.validate_claude_context(context, restoring)
    };
    let engine = ConfigEngine {
        store: &f.service.store,
        vault: f.service.vault.as_ref(),
        allowed_roots: f.service.paths.configuration_roots.clone(),
        policy_guard: Some(&guard),
        context_guard: None,
        claude_context_guard: Some(&context),
    };
    // The domain boundary must refuse even without relying on the UI preview TTL.
    assert_eq!(
        engine.apply(&journal.id).unwrap_err().code,
        "backup_expired"
    );
    for (path, bytes) in apply.paths.iter().zip(before) {
        assert_eq!(read_file(Path::new(path)).unwrap(), bytes);
    }
    assert_eq!(grants(&f.service), before_grants);
    assert_eq!(
        f.service
            .store
            .get::<Journal>("journal", &journal.id)
            .unwrap()
            .unwrap()
            .phase,
        "prepared"
    );
    assert_eq!(
        f.service
            .store
            .get::<Journal>("journal", &source_id)
            .unwrap()
            .unwrap()
            .phase,
        "committed"
    );
}

#[test]
fn claude_restore_publishing_can_finish_after_source_expiry_without_reviving_revoked_grants() {
    for logged_out in [false, true] {
        let f = tests::policy_fixture(ClientKind::Claude);
        let first = automatic_preview(&f.service, &f);
        f.service
            .apply_client_config(&first.id, true, false)
            .unwrap();
        let before: Vec<_> = first
            .paths
            .iter()
            .map(|path| read_file(Path::new(path)).unwrap())
            .collect();
        let first_grant: HelperBinding = f
            .service
            .store
            .list::<HelperBinding>("binding", None)
            .unwrap()
            .into_iter()
            .find(|binding| binding.enabled)
            .unwrap();
        let second = automatic_preview(&f.service, &f);
        let saved = f
            .service
            .apply_client_config(&second.id, true, false)
            .unwrap();
        let source_id = saved.backup_group_id.unwrap();
        let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(2)).to_rfc3339();
        set_claude_backup_expiry(&f.service, &source_id, &expires_at);
        let restore = f.service.preview_restore(&source_id).unwrap();
        let mut restoring = preview_journal(&f.service, &restore);
        assert!(chrono::Utc::now() < chrono::DateTime::parse_from_rfc3339(&expires_at).unwrap());
        restoring.phase = "publishing".into();
        f.service
            .store
            .put("journal", &restoring.id, &restoring.account_id, &restoring)
            .unwrap();
        // A confirmed restoration was interrupted after its first file write.
        assert_eq!(restore.paths[0], first.paths[2]);
        std::fs::write(&restore.paths[0], before[2].as_ref().unwrap()).unwrap();
        if logged_out {
            f.service.commit_logout(&f.record.view.id).unwrap();
        }
        wait_for_claude_backup_expiry(&expires_at);
        let recovered = Service::new(
            f.service.store.clone(),
            f.service.vault.clone(),
            f.service.paths.clone(),
        )
        .unwrap();
        for (path, original) in first.paths.iter().zip(before) {
            assert_eq!(read_file(Path::new(path)).unwrap(), original);
        }
        assert!(!Path::new(&second.paths[0]).exists());
        let bindings = recovered
            .store
            .list::<HelperBinding>("binding", None)
            .unwrap();
        assert!(
            bindings
                .iter()
                .all(|binding| binding.enabled == (!logged_out && binding.id == first_grant.id))
        );
        assert_eq!(
            recovered
                .store
                .get::<Journal>("journal", &restoring.id)
                .unwrap()
                .unwrap()
                .phase,
            "committed"
        );
        // Successful commit permits the existing retention policy to prune
        // this now-expired source, including its restored journal row.
        assert!(
            recovered
                .store
                .get::<Journal>("journal", &source_id)
                .unwrap()
                .is_none()
        );
        assert!(
            recovered
                .store
                .get::<Journal>("backup", &source_id)
                .unwrap()
                .is_none()
        );
        assert!(
            !recovered
                .pending_configuration_recovery(ClientKind::Claude)
                .unwrap()
        );
        assert_unobserved(&recovered, "not_configured");
    }
}

#[tokio::test]
async fn claude_b3c_automatic_command_consumes_late_authority_changes_before_vault_or_writes() {
    for fault in [
        "target",
        "local-policy",
        "helper",
        "account",
        "epoch",
        "key",
        "node",
        "endpoint",
        "model",
        "catalog",
    ] {
        let f = tests::policy_fixture(ClientKind::Claude);
        let preview = automatic_preview(&f.service, &f);
        let journal = preview_journal(&f.service, &preview);
        let before_grants = grants(&f.service);
        let original_record = f.record.clone();
        let original_cache = f.service.account_cache(&f.record).unwrap();
        let helper = std::fs::read(&f.service.paths.helper).unwrap();
        let mut record = original_record.clone();
        let mut cache = original_cache.clone();
        let epoch = authorization_epoch_reference(
            &record.view.panel_origin,
            &record.installation_id,
            &record.view.account_id,
        );
        match fault {
            "target" => f.target.store(1, Ordering::SeqCst),
            "local-policy" => f.epoch.store(1, Ordering::SeqCst),
            "helper" => {
                std::fs::write(&f.service.paths.helper, b"replaced synthetic helper").unwrap()
            }
            "account" => record.view.account_id = "synthetic-other-actor".into(),
            "epoch" => f.service.store.set_meta(&epoch, "999").unwrap(),
            "key" => cache.keys[0].state = "revoked".into(),
            "node" => cache.nodes[0].available = false,
            "endpoint" => {
                cache.claude_endpoints.insert(
                    record.view.node_id.clone().unwrap(),
                    "https://late-claude.example.test".into(),
                );
            }
            "model" => cache.models[0].revision = sha256(b"late model"),
            "catalog" => cache.catalog_revision = sha256(b"late catalog"),
            _ => unreachable!(),
        }
        f.service
            .store
            .put(
                "connection",
                &record.view.id,
                &record.view.account_id,
                &record,
            )
            .unwrap();
        f.service
            .store
            .put(
                "account-cache",
                &record.view.id,
                &record.view.account_id,
                &cache,
            )
            .unwrap();
        f.service.http.scripted(Arc::new(|_| {
            panic!("late preflight must precede Panel refresh")
        }));
        f.vault.calls.store(0, Ordering::SeqCst);
        f.vault.mutations.store(0, Ordering::SeqCst);
        let revision = f.service.store.snapshot().unwrap().revision;
        assert_eq!(
            f.service
                .apply_client_config_refreshed(&preview.id, true)
                .await
                .unwrap_err()
                .code,
            match fault {
                "target" => "claude_configuration_target_changed",
                "helper" => "helper_unavailable",
                _ => "claude_configuration_context_changed",
            },
            "{fault}"
        );
        assert_eq!(
            f.service.store.snapshot().unwrap().revision,
            revision,
            "{fault}"
        );
        assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0, "{fault}");
        assert_eq!(f.vault.mutations.load(Ordering::SeqCst), 0);
        assert_eq!(grants(&f.service), before_grants);
        assert!(!f.service.paths.claude_library.parent().unwrap().exists());
        assert!(
            f.service
                .store
                .list::<Journal>("backup", None)
                .unwrap()
                .is_empty()
        );
        let current: Journal = f
            .service
            .store
            .get("journal", &journal.id)
            .unwrap()
            .unwrap();
        assert_eq!(current.phase, "prepared");
        assert_eq!(current.ciphertext, journal.ciphertext);
        // Returning the environment to the original state cannot revive a consumed preview.
        f.target.store(0, Ordering::SeqCst);
        f.epoch.store(0, Ordering::SeqCst);
        std::fs::write(&f.service.paths.helper, &helper).unwrap();
        f.service
            .store
            .set_meta(&epoch, &original_record.authorization_epoch.to_string())
            .unwrap();
        f.service
            .store
            .put(
                "connection",
                &original_record.view.id,
                &original_record.view.account_id,
                &original_record,
            )
            .unwrap();
        f.service
            .store
            .put(
                "account-cache",
                &original_record.view.id,
                &original_record.view.account_id,
                &original_cache,
            )
            .unwrap();
        assert_eq!(
            f.service
                .apply_client_config_refreshed(&preview.id, true)
                .await
                .unwrap_err()
                .code,
            "preview_expired",
            "{fault}"
        );
        assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn claude_b3c_refresh_rejects_late_endpoint_and_final_model_scope_without_first_write() {
    for fault in ["endpoint", "model", "catalog"] {
        let f = tests::policy_fixture(ClientKind::Claude);
        let wire: Value =
            serde_json::from_str(include_str!("../tests/fixtures/public-wire.json")).unwrap();
        let bootstrap = wire["bootstrap_reply"].clone();
        let models_revision = sha256(b"independent models revision");
        let stage = Arc::new(AtomicU64::new(0));
        let late = stage.clone();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let requests = calls.clone();
        f.service.http.scripted(Arc::new(move |request| {
            let mut response = bootstrap.clone();
            if request.url.ends_with("/api/v1/orbit/bootstrap") {
                requests.lock().unwrap().push("bootstrap");
                if late.load(Ordering::SeqCst) == 1 && fault == "endpoint" {
                    response["data"]["nodes"][0]["claude_base_url"] = json!("https://late-claude.example.test");
                }
            } else {
                assert!(request.url.contains("/api/v1/orbit/models?node_id="));
                requests.lock().unwrap().push("models");
                response = json!({"data": response["data"]["models"], "meta":{"revision":models_revision}});
                if late.load(Ordering::SeqCst) == 1 {
                    if fault == "model" { response["data"][0]["display_name"] = json!("Late changed model"); }
                    if fault == "catalog" { response["meta"]["revision"] = json!(sha256(b"late catalog revision")); }
                }
            }
            Box::pin(async move { Ok(crate::network::RemoteResponse {status:200, request_id:None, bytes:serde_json::to_vec(&response).unwrap()}) })
        }));
        let preview = f
            .service
            .preview_client_config_refreshed(
                &f.record.view.id,
                f.record.view.revision,
                ClientKind::Claude,
                &f.model,
            )
            .await
            .unwrap();
        let journal = preview_journal(&f.service, &preview);
        let before_grants = grants(&f.service);
        stage.store(1, Ordering::SeqCst);
        assert_eq!(
            f.service
                .apply_client_config_refreshed(&preview.id, true)
                .await
                .unwrap_err()
                .code,
            "claude_configuration_context_changed",
            "{fault}"
        );
        let expected: &[&str] = if fault == "endpoint" {
            &["bootstrap", "models", "bootstrap"]
        } else {
            &["bootstrap", "models", "bootstrap", "models"]
        };
        assert_eq!(calls.lock().unwrap().as_slice(), expected, "{fault}");
        assert_eq!(grants(&f.service), before_grants);
        assert!(!f.service.paths.claude_library.parent().unwrap().exists());
        let current: Journal = f
            .service
            .store
            .get("journal", &journal.id)
            .unwrap()
            .unwrap();
        assert_eq!(current.phase, "prepared");
        assert_eq!(current.ciphertext, journal.ciphertext);
        stage.store(0, Ordering::SeqCst);
        assert_eq!(
            f.service
                .apply_client_config_refreshed(&preview.id, true)
                .await
                .unwrap_err()
                .code,
            "preview_expired"
        );
        assert_eq!(calls.lock().unwrap().as_slice(), expected);
    }
}

#[test]
fn claude_b3c_automatic_apply_rechecks_vault_return_before_first_file_or_grant() {
    for fault in ["target", "helper", "endpoint"] {
        let f = tests::policy_fixture(ClientKind::Claude);
        let preview = automatic_preview(&f.service, &f);
        let before_grants = grants(&f.service);
        let (entered, waiting) = std::sync::mpsc::channel();
        let (resume, released) = std::sync::mpsc::channel();
        *f.vault.pause.lock().unwrap() = Some((entered, released));
        let worker = f.service.clone();
        let id = preview.id.clone();
        let thread = std::thread::spawn(move || worker.apply_client_config(&id, true, false));
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        match fault {
            "target" => f.target.store(1, Ordering::SeqCst),
            "helper" => std::fs::write(
                &f.service.paths.helper,
                b"late synthetic helper replacement",
            )
            .unwrap(),
            "endpoint" => {
                let mut cache = f.service.account_cache(&f.record).unwrap();
                cache.claude_endpoints.insert(
                    f.record.view.node_id.clone().unwrap(),
                    "https://late-claude.example.test".into(),
                );
                f.service
                    .store
                    .put(
                        "account-cache",
                        &f.record.view.id,
                        &f.record.view.account_id,
                        &cache,
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        resume.send(()).unwrap();
        assert_eq!(
            thread.join().unwrap().unwrap_err().code,
            match fault {
                "target" => "claude_configuration_target_changed",
                "helper" => "helper_unavailable",
                _ => "claude_configuration_context_changed",
            }
        );
        assert_eq!(grants(&f.service), before_grants);
        assert!(!f.service.paths.claude_library.parent().unwrap().exists());
        assert_eq!(
            f.service
                .apply_client_config(&preview.id, true, false)
                .unwrap_err()
                .code,
            "preview_expired"
        );
    }
}

#[test]
fn claude_b3c_account_switch_and_same_account_node_switch_manage_only_their_own_grants() {
    for old_authority in ["active", "logout", "deleted"] {
        let f = tests::policy_fixture(ClientKind::Claude);
        let a_preview = automatic_preview(&f.service, &f);
        f.service
            .apply_client_config(&a_preview.id, true, false)
            .unwrap();
        let a_bytes: Vec<_> = a_preview
            .paths
            .iter()
            .map(|p| read_file(Path::new(p)).unwrap())
            .collect();
        let a_binding: HelperBinding = f
            .service
            .store
            .list::<HelperBinding>("binding", None)
            .unwrap()
            .into_iter()
            .find(|g| g.enabled)
            .unwrap();
        let installation = f.service.installation(&f.record).unwrap();
        let mut b = f.record.clone();
        b.view.id = uuid();
        b.view.account_id = "synthetic-account-B".into();
        b.session_reference = format!("synthetic-session/{}", b.view.id);
        b.credential_reference = None;
        b.view.credential_revision = None;
        b.api_key_id = None;
        b.cleanup_references.clear();
        let mut cache = f.service.account_cache(&f.record).unwrap();
        cache.account.id = b.view.account_id.clone();
        cache.account.label = "Synthetic B".into();
        let wire: Value =
            serde_json::from_str(include_str!("../tests/fixtures/public-wire.json")).unwrap();
        let mut token = wire["token_reply"]["data"].clone();
        token["account_id"] = json!(b.view.account_id);
        let (_, session) = serde_json::from_value::<TokenResponse>(token)
            .unwrap()
            .into_session(&installation)
            .unwrap();
        f.service.save_session(&b, session).unwrap();
        f.service
            .store
            .set_meta(
                &authorization_epoch_reference(
                    &b.view.panel_origin,
                    &b.installation_id,
                    &b.view.account_id,
                ),
                &b.authorization_epoch.to_string(),
            )
            .unwrap();
        f.service
            .store
            .put("connection", &b.view.id, &b.view.account_id, &b)
            .unwrap();
        let mut delivery = wire["credential_reply"]["data"].clone();
        delivery["api_key_id"] = json!("synthetic-B-key");
        f.service
            .accept_credential(
                &mut b,
                &installation,
                serde_json::from_value(delivery).unwrap(),
            )
            .unwrap();
        cache.keys[0].api_key_id = b.api_key_id.clone();
        f.service
            .store
            .put("account-cache", &b.view.id, &b.view.account_id, &cache)
            .unwrap();
        f.service.select_connection(&b.view.id).unwrap();
        let first = f
            .service
            .preview_client_config(&b.view.id, b.view.revision, ClientKind::Claude, &f.model)
            .unwrap();
        let first_saved = f
            .service
            .apply_client_config(&first.id, true, false)
            .unwrap();
        let first_binding = f
            .service
            .store
            .list::<HelperBinding>("binding", Some(&b.view.account_id))
            .unwrap()
            .into_iter()
            .find(|g| g.enabled)
            .unwrap();
        assert_eq!(
            serde_json::to_value(
                f.service
                    .store
                    .get::<HelperBinding>("binding", &a_binding.id)
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(&a_binding).unwrap()
        );
        if old_authority == "logout" {
            f.service.commit_logout(&f.record.view.id).unwrap();
        }
        if old_authority == "deleted" {
            f.service.store.delete("binding", &a_binding.id).unwrap();
        }
        let a_authority = serde_json::to_value(
            f.service
                .store
                .get::<HelperBinding>("binding", &a_binding.id)
                .unwrap(),
        )
        .unwrap();
        // A same-account node switch manages B's previous grant and uses the
        // new Claude root, while A's authority stays outside the transaction.
        let mut next = cache.nodes[0].clone();
        next.id = "synthetic-B-next-node".into();
        next.base_url = "https://b-next.example.test/v1".into();
        next.revision = sha256(b"B next node");
        let next_id = next.id.clone();
        cache.nodes.push(next);
        cache
            .claude_endpoints
            .insert(next_id.clone(), "https://b-next.example.test".into());
        f.service
            .store
            .put("account-cache", &b.view.id, &b.view.account_id, &cache)
            .unwrap();
        f.service
            .select_node(&b.view.id, &next_id, b.view.revision)
            .unwrap();
        b = f.service.record(&b.view.id).unwrap();
        let second = f
            .service
            .preview_client_config(&b.view.id, b.view.revision, ClientKind::Claude, &f.model)
            .unwrap();
        let second_saved = f
            .service
            .apply_client_config(&second.id, true, false)
            .unwrap();
        let profile: Value =
            serde_json::from_slice(&std::fs::read(&second.paths[0]).unwrap()).unwrap();
        assert_eq!(
            profile["inferenceGatewayBaseUrl"],
            "https://b-next.example.test"
        );
        assert!(
            !f.service
                .store
                .get::<HelperBinding>("binding", &first_binding.id)
                .unwrap()
                .unwrap()
                .enabled
        );
        let restore = f
            .service
            .preview_restore(second_saved.backup_group_id.as_ref().unwrap())
            .unwrap();
        f.service
            .apply_client_config(&restore.id, true, true)
            .unwrap();
        assert!(
            f.service
                .store
                .get::<HelperBinding>("binding", &first_binding.id)
                .unwrap()
                .unwrap()
                .enabled
        );
        let restore = f
            .service
            .preview_restore(first_saved.backup_group_id.as_ref().unwrap())
            .unwrap();
        f.service
            .apply_client_config(&restore.id, true, true)
            .unwrap();
        for (path, bytes) in a_preview.paths.iter().zip(a_bytes) {
            assert_eq!(read_file(Path::new(path)).unwrap(), bytes);
        }
        assert_eq!(
            serde_json::to_value(
                f.service
                    .store
                    .get::<HelperBinding>("binding", &a_binding.id)
                    .unwrap()
            )
            .unwrap(),
            a_authority,
            "{old_authority}"
        );
        assert!(
            f.service
                .store
                .list::<HelperBinding>("binding", Some(&b.view.account_id))
                .unwrap()
                .iter()
                .all(|grant| !grant.enabled)
        );
    }
}

#[tokio::test]
async fn claude_b3c_pending_or_conflict_blocks_new_and_already_prepared_applies_and_external_restore_stops()
 {
    for phase in ["publishing", "conflict"] {
        let f = tests::policy_fixture(ClientKind::Claude);
        let earlier = automatic_preview(&f.service, &f);
        let command = automatic_preview(&f.service, &f);
        let pending = automatic_preview(&f.service, &f);
        let mut journal = preview_journal(&f.service, &pending);
        journal.phase = phase.into();
        f.service
            .store
            .put("journal", &journal.id, &journal.account_id, &journal)
            .unwrap();
        let blocked = automatic_preview(&f.service, &f);
        assert!(!blocked.can_apply);
        assert_eq!(
            blocked.blocking_reason.as_deref(),
            Some("configuration_recovery_required")
        );
        let before_grants = grants(&f.service);
        f.vault.calls.store(0, Ordering::SeqCst);
        assert_eq!(
            f.service
                .apply_client_config(&earlier.id, true, false)
                .unwrap_err()
                .code,
            "configuration_recovery_required",
            "{phase}"
        );
        f.service.http.scripted(Arc::new(|_| {
            panic!("pending recovery must precede Panel refresh")
        }));
        assert_eq!(
            f.service
                .apply_client_config_refreshed(&command.id, true)
                .await
                .unwrap_err()
                .code,
            "configuration_recovery_required",
            "{phase}"
        );
        assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0);
        assert_eq!(grants(&f.service), before_grants);
        assert!(!f.service.paths.claude_library.parent().unwrap().exists());
    }
    let f = tests::policy_fixture(ClientKind::Claude);
    let preview = automatic_preview(&f.service, &f);
    let saved = f
        .service
        .apply_client_config(&preview.id, true, false)
        .unwrap();
    let restore = f
        .service
        .preview_restore(saved.backup_group_id.as_ref().unwrap())
        .unwrap();
    std::fs::write(&preview.paths[1], b"external metadata edit").unwrap();
    let files: Vec<_> = preview
        .paths
        .iter()
        .map(|p| read_file(Path::new(p)).unwrap())
        .collect();
    let before_grants = grants(&f.service);
    assert_eq!(
        f.service
            .apply_client_config(&restore.id, true, true)
            .unwrap_err()
            .code,
        "configuration_conflict"
    );
    for (path, bytes) in preview.paths.iter().zip(files) {
        assert_eq!(read_file(Path::new(path)).unwrap(), bytes);
    }
    assert_eq!(grants(&f.service), before_grants);
    assert_eq!(
        f.service
            .apply_client_config(&restore.id, true, true)
            .unwrap_err()
            .code,
        "preview_expired"
    );
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
fn claude_qualification_errors_cross_guards_without_promoting_display_hints() {
    for code in [
        "claude_signature_verification_timeout",
        "claude_configuration_target_unverified",
        "claude_configuration_target_changed",
    ] {
        let fixture = tests::policy_fixture(ClientKind::Claude);
        let context = frozen(&fixture);
        let observation = fixture
            .service
            .observe_configuration(ClientKind::Claude)
            .unwrap();
        let mut paths = fixture.service.paths.clone();
        let adapter = paths.fixture_claude_configuration_target.clone().unwrap();
        paths.fixture_claude_configuration_target = Some(Arc::new(move |client, full| {
            if full {
                Err(fail(code))
            } else {
                adapter(client, false)
            }
        }));
        let service =
            Service::new(fixture.service.store.clone(), fixture.vault.clone(), paths).unwrap();
        fixture.vault.calls.store(0, Ordering::SeqCst);
        fixture.vault.mutations.store(0, Ordering::SeqCst);
        assert_eq!(
            service
                .client_policy(ClientKind::Claude)
                .unwrap()
                .0
                .configuration_access,
            ConfigurationAccess::Allowed
        );
        assert_eq!(
            service
                .configuration_preflight(ClientKind::Claude)
                .unwrap_err()
                .code,
            code
        );
        assert_eq!(
            service
                .validate_claude_configuration(
                    &observation.policy,
                    observation.claude_configuration_target.as_ref()
                )
                .unwrap_err()
                .code,
            code
        );
        for restoring in [false, true] {
            assert_eq!(
                service
                    .validate_claude_context(&context, restoring)
                    .unwrap_err()
                    .code,
                code
            );
        }
        let blocked = service
            .preview_client_config(
                &fixture.record.view.id,
                fixture.record.view.revision,
                ClientKind::Claude,
                &fixture.model,
            )
            .unwrap();
        assert_eq!(blocked.blocking_reason.as_deref(), Some(code));
        assert!(!blocked.can_apply && !blocked.can_export_import);
        assert!(blocked.claude_configuration_target.is_none());
        assert!(
            service
                .store
                .list::<HelperBinding>("binding", None)
                .unwrap()
                .is_empty()
        );
        assert!(
            service
                .store
                .list::<Journal>("journal", None)
                .unwrap()
                .is_empty()
        );
        assert!(!service.paths.claude_library.parent().unwrap().exists());
        assert_eq!(fixture.vault.calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.vault.mutations.load(Ordering::SeqCst), 0);
    }
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
