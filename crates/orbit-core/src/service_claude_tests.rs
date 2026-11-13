use super::*;
use crate::config::ClaudeContext;
use std::fs;

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
        if fault == "policy-unknown" {
            assert_eq!(client.configuration_access, ConfigurationAccess::Unverified);
        }
        if fault == "version" {
            assert_eq!(client.configuration_access, ConfigurationAccess::Allowed);
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

fn mapping_fixture() -> tests::PolicyFixture {
    let mut f = tests::policy_fixture(ClientKind::Claude);
    let mut cache = f.service.account_cache(&f.record).unwrap();
    cache.models[0].id = "gpt-6-luna".into();
    f.model = cache.models[0].id.clone();
    f.service
        .store
        .put(
            "account-cache",
            &f.record.view.id,
            &f.record.view.account_id,
            &cache,
        )
        .unwrap();
    f.service.http.scripted_mapping(Arc::new(|_| {
        Box::pin(async { Err(fail("synthetic_unexpected_upstream_send")) })
    }));
    f
}

fn preferences_restore_fixture() -> tests::PolicyFixture {
    let f = tests::policy_fixture(ClientKind::Claude);
    let root = f.service.paths.claude_library.parent().unwrap();
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("claude_desktop_config.json"),
        br#"{"deploymentMode":"1p","preferences":{"theme":"system"},"unowned":"retained"}"#,
    )
    .unwrap();
    f
}

fn write_service_preferences(service: &Service) -> Vec<u8> {
    let path = service
        .paths
        .claude_library
        .parent()
        .unwrap()
        .join("claude_desktop_config.json");
    let mut main: std::collections::BTreeMap<String, Box<serde_json::value::RawValue>> =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    main.insert("preferences".into(), serde_json::value::RawValue::from_string(
        r#"{"huge":184467440737095516170000,"fraction":1.00000000000000000000001,"unknown":[true,{"retained":"yes"}]}"#.into()
    ).unwrap());
    let mut bytes = serde_json::to_vec_pretty(&main).unwrap();
    bytes.push(b'\n');
    fs::write(path, &bytes).unwrap();
    bytes
}

#[test]
fn claude_restore_coordination_service_previews_preserves_preferences_and_persists_historical_note()
{
    let f = preferences_restore_fixture();
    let preview = automatic_preview(&f.service, &f);
    assert!(preview.restore_coordination.is_none());
    assert!(
        !serde_json::to_value(&preview)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("restore_coordination")
    );
    let saved = f
        .service
        .apply_client_config(&preview.id, true, false)
        .unwrap();
    tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
    let current = write_service_preferences(&f.service);
    let source = saved.snapshot_id.as_ref().unwrap();
    assert_eq!(
        f.service.preview_restore(source).unwrap_err().code,
        "configuration_conflict"
    );
    let before = preview
        .paths
        .iter()
        .map(|path| read_file(Path::new(path)).unwrap())
        .collect::<Vec<_>>();
    let mutations = f.vault.mutations.load(Ordering::SeqCst);
    let coordinated = f
        .service
        .preview_restore_with_coordination(
            source,
            Some(ClaudeRestoreCoordination::MainPreferencesV1),
        )
        .unwrap();
    assert!(coordinated.can_apply);
    assert_eq!(
        coordinated.restore_coordination,
        Some(ClaudeRestoreCoordination::MainPreferencesV1)
    );
    assert_eq!(
        serde_json::to_value(&coordinated).unwrap()["restore_coordination"],
        "claude_main_preferences_v1"
    );
    let diff: Value = serde_json::from_str(&coordinated.redacted_diff).unwrap();
    assert_eq!(
        diff["claude_desktop_config.json"]["deploymentMode"]["to"],
        "1p"
    );
    assert!(
        !coordinated
            .redacted_diff
            .contains("184467440737095516170000")
    );
    for (path, bytes) in preview.paths.iter().zip(&before) {
        assert!(read_file(Path::new(path)).unwrap() == *bytes);
    }
    assert_eq!(f.vault.mutations.load(Ordering::SeqCst), mutations);
    assert_eq!(
        f.service
            .apply_client_config(&coordinated.id, true, true)
            .unwrap()
            .state,
        "restored"
    );
    let main_path = f
        .service
        .paths
        .claude_library
        .parent()
        .unwrap()
        .join("claude_desktop_config.json");
    let main_bytes = fs::read(&main_path).unwrap();
    let main: std::collections::BTreeMap<String, Box<serde_json::value::RawValue>> =
        serde_json::from_slice(&main_bytes).unwrap();
    let current: std::collections::BTreeMap<String, Box<serde_json::value::RawValue>> =
        serde_json::from_slice(&current).unwrap();
    assert_eq!(main["preferences"].get(), current["preferences"].get());
    assert_eq!(main["deploymentMode"].get(), r#""1p""#);
    assert!(!Path::new(&preview.paths[0]).exists() && !Path::new(&preview.paths[1]).exists());
    let client: ClientView = f
        .service
        .store
        .get("client-state", "claude")
        .unwrap()
        .unwrap();
    assert!(
        client
            .manual_steps
            .iter()
            .any(|note| note == claude::PREFERENCES_RESTORED_NOTE)
    );
    assert!(
        !client
            .manual_steps
            .iter()
            .any(|note| note.contains("配置原字节已恢复"))
    );
    assert!(client.loaded_evidence.is_none() && client.connection_evidence.is_none());
    let restarted = Service::new(
        f.service.store.clone(),
        f.vault.clone(),
        f.service.paths.clone(),
    )
    .unwrap();
    let client = restarted
        .bootstrap()
        .unwrap()
        .clients
        .into_iter()
        .find(|client| client.client == ClientKind::Claude)
        .unwrap();
    assert!(
        client
            .manual_steps
            .iter()
            .any(|note| note == claude::PREFERENCES_RESTORED_NOTE)
    );
    assert!(client.loaded_evidence.is_none() && client.connection_evidence.is_none());
    let fresh = automatic_preview(&restarted, &f);
    let saved = restarted
        .apply_client_config(&fresh.id, true, false)
        .unwrap();
    tests::mark_switch_snapshot(&restarted, saved.snapshot_id.as_ref().unwrap());
    let client: ClientView = restarted
        .store
        .get("client-state", "claude")
        .unwrap()
        .unwrap();
    assert!(
        !client
            .manual_steps
            .iter()
            .any(|note| note == claude::PREFERENCES_RESTORED_NOTE)
    );
    let strict = restarted
        .preview_restore(saved.snapshot_id.as_ref().unwrap())
        .unwrap();
    assert!(strict.restore_coordination.is_none());
    assert_eq!(
        restarted
            .apply_client_config(&strict.id, true, true)
            .unwrap()
            .state,
        "restored"
    );
    let client: ClientView = restarted
        .store
        .get("client-state", "claude")
        .unwrap()
        .unwrap();
    assert!(
        client
            .manual_steps
            .iter()
            .any(|note| note.contains("配置原字节已恢复"))
    );
    assert!(
        !client
            .manual_steps
            .iter()
            .any(|note| note == claude::PREFERENCES_RESTORED_NOTE)
    );
}

#[test]
fn claude_restore_coordination_rejects_wrong_client_or_phase_before_observation_and_vault() {
    let f = preferences_restore_fixture();
    let preview = automatic_preview(&f.service, &f);
    let saved = f
        .service
        .apply_client_config(&preview.id, true, false)
        .unwrap();
    tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
    let source_id = saved.snapshot_id.as_ref().unwrap();
    let source: Journal = f.service.store.get("journal", source_id).unwrap().unwrap();
    let mut paths = f.service.paths.clone();
    let adapter = paths.fixture_claude_configuration_target.clone().unwrap();
    let observed = Arc::new(AtomicU64::new(0));
    let count = observed.clone();
    paths.fixture_claude_configuration_target = Some(Arc::new(move |client, full| {
        count.fetch_add(1, Ordering::SeqCst);
        adapter(client, full)
    }));
    let service = Service::new(f.service.store.clone(), f.vault.clone(), paths).unwrap();
    for fault in ["wrong_client", "uncommitted", "source_uncommitted"] {
        let mut changed = source.clone();
        match fault {
            "wrong_client" => changed.client = ClientKind::Codex,
            "uncommitted" => changed.phase = "publishing".into(),
            _ => changed.phase = "conflict".into(),
        }
        service
            .store
            .put("journal", source_id, &source.account_id, &changed)
            .unwrap();
        observed.store(0, Ordering::SeqCst);
        f.vault.calls.store(0, Ordering::SeqCst);
        f.vault.mutations.store(0, Ordering::SeqCst);
        let revision = service.store.snapshot().unwrap().revision;
        assert_eq!(
            service
                .preview_restore_with_coordination(
                    source_id,
                    Some(ClaudeRestoreCoordination::MainPreferencesV1)
                )
                .unwrap_err()
                .code,
            "configuration_conflict",
            "{fault}"
        );
        assert_eq!(observed.load(Ordering::SeqCst), 0, "{fault}");
        assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0, "{fault}");
        assert_eq!(f.vault.mutations.load(Ordering::SeqCst), 0, "{fault}");
        assert_eq!(
            service.store.snapshot().unwrap().revision,
            revision,
            "{fault}"
        );
    }
}

#[test]
fn claude_restore_coordination_preview_mode_must_match_authenticated_operation() {
    for coordinated in [false, true] {
        let f = preferences_restore_fixture();
        let preview = automatic_preview(&f.service, &f);
        let saved = f
            .service
            .apply_client_config(&preview.id, true, false)
            .unwrap();
        tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
        if coordinated {
            write_service_preferences(&f.service);
        }
        let restore = f
            .service
            .preview_restore_with_coordination(
                saved.snapshot_id.as_ref().unwrap(),
                coordinated.then_some(ClaudeRestoreCoordination::MainPreferencesV1),
            )
            .unwrap();
        let before = preview
            .paths
            .iter()
            .map(|path| read_file(Path::new(path)).unwrap())
            .collect::<Vec<_>>();
        f.service
            .previews
            .lock()
            .unwrap()
            .get_mut(&restore.id)
            .unwrap()
            .view
            .restore_coordination =
            (!coordinated).then_some(ClaudeRestoreCoordination::MainPreferencesV1);
        assert_eq!(
            f.service
                .apply_client_config(&restore.id, true, true)
                .unwrap_err()
                .code,
            "backup_invalid"
        );
        for (path, bytes) in preview.paths.iter().zip(before) {
            assert!(read_file(Path::new(path)).unwrap() == bytes);
        }
        assert_eq!(
            f.service
                .apply_client_config(&restore.id, true, true)
                .unwrap_err()
                .code,
            "preview_expired"
        );
    }
}

#[test]
fn claude_restore_coordination_vault_return_rechecks_target_and_late_epoch_never_revives_old_grants()
 {
    let f = preferences_restore_fixture();
    let preview = automatic_preview(&f.service, &f);
    let saved = f
        .service
        .apply_client_config(&preview.id, true, false)
        .unwrap();
    tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
    write_service_preferences(&f.service);
    let before = preview
        .paths
        .iter()
        .map(|path| read_file(Path::new(path)).unwrap())
        .collect::<Vec<_>>();
    let (entered, waiting) = std::sync::mpsc::channel();
    let (resume, released) = std::sync::mpsc::channel();
    *f.vault.pause.lock().unwrap() = Some((entered, released));
    let service = f.service.clone();
    let source = saved.snapshot_id.clone().unwrap();
    let worker = std::thread::spawn(move || {
        service.preview_restore_with_coordination(
            &source,
            Some(ClaudeRestoreCoordination::MainPreferencesV1),
        )
    });
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    f.target.store(1, Ordering::SeqCst);
    resume.send(()).unwrap();
    assert!(worker.join().unwrap().is_err());
    assert!(f.service.previews.lock().unwrap().is_empty());
    for (path, bytes) in preview.paths.iter().zip(before) {
        assert!(read_file(Path::new(path)).unwrap() == bytes);
    }

    let f = preferences_restore_fixture();
    let first = automatic_preview(&f.service, &f);
    let old = mapping_context(&f.service, &first).binding_id;
    f.service
        .apply_client_config(&first.id, true, false)
        .unwrap();
    let second = automatic_preview(&f.service, &f);
    let saved = f
        .service
        .apply_client_config(&second.id, true, false)
        .unwrap();
    tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
    write_service_preferences(&f.service);
    let restore = f
        .service
        .preview_restore_with_coordination(
            saved.snapshot_id.as_ref().unwrap(),
            Some(ClaudeRestoreCoordination::MainPreferencesV1),
        )
        .unwrap();
    let installation = f.service.installation(&f.record).unwrap();
    f.service
        .store
        .set_meta(
            &authorization_epoch_reference(
                &installation.origin,
                &installation.id,
                &f.record.view.account_id,
            ),
            "999",
        )
        .unwrap();
    assert_eq!(
        f.service
            .apply_client_config(&restore.id, true, true)
            .unwrap()
            .state,
        "restored"
    );
    assert!(Path::new(&first.paths[0]).exists() && !Path::new(&second.paths[0]).exists());
    assert!(
        !f.service
            .store
            .get::<HelperBinding>("binding", &old)
            .unwrap()
            .unwrap()
            .enabled
    );
    assert!(
        f.service
            .store
            .list::<HelperBinding>("binding", None)
            .unwrap()
            .iter()
            .all(|binding| !binding.enabled)
    );
}

struct MappingReadFailureVault {
    inner: Arc<tests::PolicyVault>,
    reference: String,
    error: &'static str,
    puts: AtomicU64,
    deleted: Mutex<Vec<String>>,
}

#[cfg(target_os = "macos")]
struct MappingCreatorVault {
    inner: Arc<tests::PolicyVault>,
    store: Arc<Store>,
    deny: bool,
    create_calls: AtomicU64,
    mapping_puts: AtomicU64,
}
#[cfg(target_os = "macos")]
impl Vault for MappingCreatorVault {
    fn get(&self, reference: &str) -> Result<Zeroizing<Vec<u8>>> {
        self.inner.get(reference)
    }
    fn get_optional(&self, reference: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        self.inner.get_optional(reference)
    }
    fn put(&self, reference: &str, value: &[u8]) -> Result<()> {
        if reference.starts_with("claude-map/") {
            self.mapping_puts.fetch_add(1, Ordering::SeqCst);
        }
        self.inner.put(reference, value)
    }
    fn create_claude_mapping_secret(&self, binding_id: &str, value: &[u8]) -> Result<()> {
        self.create_calls.fetch_add(1, Ordering::SeqCst);
        if self.deny {
            return Err(fail("vault_unavailable"));
        }
        self.inner.create_claude_mapping_secret(binding_id, value)?;
        let binding: HelperBinding = self.store.get("binding", binding_id)?.unwrap();
        self.store.set_meta(
            &authorization_epoch_reference(
                &binding.origin,
                &binding.installation_id,
                &binding.account_id,
            ),
            &(binding.authorization_epoch + 1).to_string(),
        )
    }
    fn delete(&self, reference: &str) -> Result<()> {
        self.inner.delete(reference)
    }
}

#[cfg(target_os = "macos")]
#[test]
fn claude_mapping_creator_denial_and_late_epoch_never_publish_files_or_grants() {
    for deny in [true, false] {
        let f = mapping_fixture();
        let creator = Arc::new(MappingCreatorVault {
            inner: f.vault.clone(),
            store: f.service.store.clone(),
            deny,
            create_calls: AtomicU64::new(0),
            mapping_puts: AtomicU64::new(0),
        });
        let service = Service::new(
            f.service.store.clone(),
            creator.clone(),
            f.service.paths.clone(),
        )
        .unwrap();
        let preview = automatic_preview(&service, &f);
        assert!(
            preview.can_apply,
            "preview blocker: {:?}",
            preview.blocking_reason
        );
        let context = mapping_context(&service, &preview);
        let journal = preview_journal(&service, &preview);
        let before_grants = grants(&service);
        assert_eq!(
            creator.create_calls.load(Ordering::SeqCst),
            0,
            "preview reserves only; it never creates a token"
        );
        assert!(
            creator
                .get_optional(&context.mapping.as_ref().unwrap().token_reference())
                .unwrap()
                .is_none()
        );
        let result = service.apply_client_config(&preview.id, true, false);
        assert!(result.is_err());
        if deny {
            assert_eq!(result.unwrap_err().code, "vault_unavailable");
        }
        assert_eq!(creator.create_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            creator.mapping_puts.load(Ordering::SeqCst),
            0,
            "macOS must not fall back to generic put"
        );
        assert!(preview.paths.iter().all(|path| !Path::new(path).exists()));
        assert_eq!(grants(&service), before_grants);
        assert!(
            service
                .store
                .list::<HelperBinding>("binding", None)
                .unwrap()
                .iter()
                .all(|binding| !binding.enabled)
        );
        assert_eq!(
            service
                .store
                .get::<Journal>("journal", &journal.id)
                .unwrap()
                .unwrap()
                .phase,
            "prepared"
        );
        let record = service
            .store
            .get::<crate::claude_mapping::MappingRecord>("claude-mapping", &context.binding_id)
            .unwrap()
            .unwrap();
        assert!(!record.committed);
        assert!(
            crate::claude_mapping::mapped_helper_token(
                &context.binding_id,
                &service.store,
                creator.as_ref(),
                chrono::Utc::now().timestamp()
            )
            .is_err()
        );
        assert_eq!(
            creator
                .get_optional(&context.mapping.as_ref().unwrap().token_reference())
                .unwrap()
                .is_some(),
            !deny
        );
    }
}
impl Vault for MappingReadFailureVault {
    fn get(&self, reference: &str) -> Result<Zeroizing<Vec<u8>>> {
        if reference == self.reference {
            return Err(fail(self.error));
        }
        self.inner.get(reference)
    }
    // Deliberately use the conservative default get_optional: an adapter's
    // generic read error, including credential_missing, never proves absence.
    fn put(&self, reference: &str, bytes: &[u8]) -> Result<()> {
        self.puts.fetch_add(1, Ordering::SeqCst);
        self.inner.put(reference, bytes)
    }
    fn delete(&self, reference: &str) -> Result<()> {
        self.deleted.lock().unwrap().push(reference.into());
        self.inner.delete(reference)
    }
}

fn mapping_context(service: &Service, preview: &ConfigPreview) -> ClaudeContext {
    service
        .previews
        .lock()
        .unwrap()
        .get(&preview.id)
        .unwrap()
        .claude_context
        .clone()
        .unwrap()
}

async fn take_stopped_mapping_port(port: u16) -> std::net::TcpListener {
    for _ in 0..100 {
        if let Ok(listener) = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)) {
            return listener;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("owned mapping port did not stop within one second");
}

#[tokio::test]
async fn claude_mapping_activation_preview_is_reservation_only_commit_enables_exact_scope() {
    let f = mapping_fixture();
    let calls = Arc::new(AtomicU64::new(0));
    let count = calls.clone();
    f.service.http.scripted_mapping(Arc::new(move |request| {
        count.fetch_add(1, Ordering::SeqCst);
        assert!(request.url.ends_with("/v1/messages?beta=true"));
        assert!(request.headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("authorization")
                && value.starts_with("Bearer orv_sk_")
                && !value.contains("orbt_map_")
        }));
        assert_eq!(
            serde_json::from_slice::<Value>(&request.body).unwrap()["model"],
            "gpt-6-luna"
        );
        Box::pin(async {
            Ok(crate::network::MappingResponse {
                status: 200,
                headers: Default::default(),
                stream: Box::pin(futures_util::stream::iter([Ok(
                    hyper::body::Bytes::from_static(b"data: {\"type\":\"message_stop\"}\n\n"),
                )])),
            })
        })
    }));
    let preview = automatic_preview(&f.service, &f);
    let context = mapping_context(&f.service, &preview);
    let plan = context.mapping.as_ref().unwrap();
    assert!(
        preview
            .impact
            .iter()
            .any(|line| line.contains("Sonnet、Opus、Haiku 均映射至"))
    );
    assert_eq!(f.service.claude_mappings.inflight(), 0);
    assert!(preview.paths.iter().all(|path| !Path::new(path).exists()));
    assert!(f.vault.get(&plan.token_reference()).is_err());
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    assert!(
        client
            .get(format!("{}/v1/models", plan.base_url()))
            .timeout(Duration::from_millis(30))
            .send()
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.service
            .apply_client_config(&preview.id, false, false)
            .unwrap_err()
            .code,
        "confirmation_required"
    );
    assert!(f.vault.get(&plan.token_reference()).is_err());
    let saved = f
        .service
        .apply_client_config(&preview.id, true, false)
        .unwrap();
    tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
    assert_eq!(saved.state, "saved");
    let local = crate::claude_mapping::mapped_helper_token(
        &plan.binding_id,
        &f.service.store,
        f.vault.as_ref(),
        chrono::Utc::now().timestamp(),
    )
    .unwrap();
    let models = client
        .get(format!("{}/v1/models", plan.base_url()))
        .bearer_auth(local.as_str())
        .send()
        .await
        .unwrap();
    assert_eq!(models.status(), 200);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let response=client.post(format!("{}/v1/messages?beta=true",plan.base_url()))
        .bearer_auth(local.as_str()).header("anthropic-beta","synthetic-beta")
        .header("content-type","application/json")
        .body(r#"{"model":"claude-sonnet-5","messages":[{"role":"user","content":"synthetic"}],"max_tokens":1}"#)
        .send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.text().await.unwrap().contains("message_stop"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_unobserved(&f.service, "saved");
    f.service.stop_claude_mappings();
}

#[tokio::test]
async fn role_switch_routes_each_alias_and_restores_original() {
    let f = mapping_fixture();
    let mut cache = f.service.account_cache(&f.record).unwrap();
    let roles = ClaudeModelMapping {
        sonnet: f.model.clone(),
        opus: "synthetic-fallback".into(),
        haiku: "synthetic-auxiliary".into(),
    };
    let original = cache
        .models
        .iter()
        .find(|model| model.id == roles.sonnet)
        .unwrap()
        .clone();
    for id in [&roles.opus, &roles.haiku] {
        let mut model = original.clone();
        model.id = id.clone();
        cache.models.push(model);
    }
    f.service
        .store
        .put(
            "account-cache",
            &f.record.view.id,
            &f.record.view.account_id,
            &cache,
        )
        .unwrap();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let captured = observed.clone();
    f.service.http.scripted_mapping(Arc::new(move |request| {
        captured.lock().unwrap().push(
            serde_json::from_slice::<Value>(&request.body).unwrap()["model"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
        Box::pin(async {
            Ok(crate::network::MappingResponse {
                status: 200,
                headers: Default::default(),
                stream: Box::pin(futures_util::stream::empty()),
            })
        })
    }));
    let preview = f
        .service
        .preview_claude_config(
            &f.record.view.id,
            f.record.view.revision,
            &roles.sonnet,
            Some(&roles),
        )
        .unwrap();
    let context = mapping_context(&f.service, &preview);
    let plan = context.mapping.as_ref().unwrap();
    let before = preview
        .paths
        .iter()
        .map(|path| read_file(Path::new(path)).unwrap())
        .collect::<Vec<_>>();
    f.service
        .apply_client_config(&preview.id, true, false)
        .unwrap();
    let local = crate::claude_mapping::mapped_helper_token(
        &plan.binding_id,
        &f.service.store,
        f.vault.as_ref(),
        chrono::Utc::now().timestamp(),
    )
    .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for alias in [
        "claude-sonnet-5",
        "claude-opus-5",
        "claude-haiku-4-5-20251001",
    ] {
        let response = client.post(format!("{}/v1/messages", plan.base_url()))
            .bearer_auth(local.as_str())
            .json(&json!({"model":alias,"messages":[{"role":"user","content":"synthetic"}],"max_tokens":1}))
            .send().await.unwrap();
        assert_eq!(response.status(), 200);
        response.bytes().await.unwrap();
    }
    assert_eq!(*observed.lock().unwrap(), roles.ids());
    let state = f.service.bootstrap().unwrap();
    let snapshot = state
        .switch_snapshots
        .iter()
        .find(|snapshot| snapshot.client == ClientKind::Claude)
        .unwrap();
    let restore = f.service.preview_restore(&snapshot.id).unwrap();
    f.service
        .apply_client_config(&restore.id, true, true)
        .unwrap();
    assert_eq!(
        preview
            .paths
            .iter()
            .map(|path| read_file(Path::new(path)).unwrap())
            .collect::<Vec<_>>(),
        before
    );
    assert!(f.service.bootstrap().unwrap().switch_snapshots.is_empty());
}

#[tokio::test]
async fn claude_mapping_activation_restart_reuses_only_original_port_and_preserves_files_on_conflict()
 {
    let f = mapping_fixture();
    let preview = automatic_preview(&f.service, &f);
    let context = mapping_context(&f.service, &preview);
    let plan = context.mapping.as_ref().unwrap();
    let saved = f
        .service
        .apply_client_config(&preview.id, true, false)
        .unwrap();
    tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
    let main_path = Path::new(&preview.paths[2]);
    let mut main: Value = serde_json::from_slice(&read_file(main_path).unwrap().unwrap()).unwrap();
    main["preferences"] = json!({"syntheticDesktopNoticeDismissed":true});
    std::fs::write(main_path, serde_json::to_vec(&main).unwrap()).unwrap();
    let before: Vec<_> = preview
        .paths
        .iter()
        .map(|path| read_file(Path::new(path)).unwrap())
        .collect();
    f.service.stop_claude_mappings();
    let collider = take_stopped_mapping_port(plan.port).await;
    let reopened = Service::new(
        f.service.store.clone(),
        f.vault.clone(),
        f.service.paths.clone(),
    )
    .unwrap();
    let calls = Arc::new(AtomicU64::new(0));
    let count = calls.clone();
    reopened.http.scripted_mapping(Arc::new(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(crate::network::MappingResponse {
                status: 200,
                headers: Default::default(),
                stream: Box::pin(futures_util::stream::iter([Ok(
                    hyper::body::Bytes::from_static(b"synthetic completed response"),
                )])),
            })
        })
    }));
    let changes = Arc::new(AtomicU64::new(0));
    let changed = changes.clone();
    reopened
        .observe(Arc::new(move |_| {
            changed.fetch_add(1, Ordering::SeqCst);
        }))
        .unwrap();
    assert_eq!(
        reopened.reconcile_claude_mappings().unwrap_err().code,
        "claude_mapping_unavailable"
    );
    assert!(
        reopened
            .bootstrap()
            .unwrap()
            .clients
            .iter()
            .find(|client| client.client == ClientKind::Claude)
            .unwrap()
            .manual_steps
            .iter()
            .any(|note| note.contains("本机模型映射未就绪"))
    );
    for (path, bytes) in preview.paths.iter().zip(&before) {
        assert_eq!(&read_file(Path::new(path)).unwrap(), bytes);
    }
    drop(collider);
    let before_change = changes.load(Ordering::SeqCst);
    reopened.reconcile_claude_mappings().unwrap();
    assert!(changes.load(Ordering::SeqCst) > before_change);
    let persisted: ClientView = reopened
        .store
        .get("client-state", "claude")
        .unwrap()
        .unwrap();
    assert!(
        !persisted
            .manual_steps
            .iter()
            .any(|note| note.contains("本机模型映射未就绪"))
    );
    assert!(
        !reopened
            .bootstrap()
            .unwrap()
            .clients
            .iter()
            .find(|client| client.client == ClientKind::Claude)
            .unwrap()
            .manual_steps
            .iter()
            .any(|note| note.contains("本机模型映射未就绪"))
    );
    let record: crate::claude_mapping::MappingRecord = reopened
        .store
        .get("claude-mapping", &plan.binding_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.plan().unwrap().port, plan.port);
    let token = crate::claude_mapping::mapped_helper_token(
        &plan.binding_id,
        &reopened.store,
        f.vault.as_ref(),
        chrono::Utc::now().timestamp(),
    )
    .unwrap();
    assert_eq!(
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("{}/v1/models", plan.base_url()))
            .bearer_auth(token.as_str())
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("{}/v1/messages", plan.base_url()))
        .bearer_auth(token.as_str())
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-5","messages":[],"max_tokens":1}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let _ = response.bytes().await.unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "unowned Desktop preferences must not break the original authenticated route"
    );
    assert_eq!(
        reopened
            .preview_restore(saved.snapshot_id.as_ref().unwrap())
            .unwrap_err()
            .code,
        "configuration_conflict",
        "runtime preference tolerance must not relax Engine original-byte restore conflicts"
    );
    reopened.stop_claude_mappings();
    let closed = take_stopped_mapping_port(plan.port).await;
    drop(closed);
    for fault in ["mode", "profile", "pointer"] {
        let index = match fault {
            "profile" => 0,
            "pointer" => 1,
            _ => 2,
        };
        let mut value: Value = serde_json::from_slice(before[index].as_ref().unwrap()).unwrap();
        match fault {
            "profile" => {
                value["inferenceGatewayBaseUrl"] = json!("https://unapproved.example.test")
            }
            "pointer" => value["hybridPointer"] = json!({"synthetic":true}),
            _ => value["deploymentMode"] = json!("1p"),
        };
        std::fs::write(&preview.paths[index], serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(reopened.reconcile_claude_mappings().is_err(), "{fault}");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "owned selection changes cannot issue an additional upstream request"
        );
        assert!(std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, plan.port)).is_ok());
        std::fs::write(&preview.paths[index], before[index].as_ref().unwrap()).unwrap();
    }
}

#[tokio::test]
async fn claude_mapping_activation_restore_old_mapping_preserves_valid_grant_and_never_revives_logout()
 {
    for logout in [false, true] {
        let f = mapping_fixture();
        let first = automatic_preview(&f.service, &f);
        let old = mapping_context(&f.service, &first);
        let old_plan = old.mapping.as_ref().unwrap();
        f.service
            .apply_client_config(&first.id, true, false)
            .unwrap();
        let original: Vec<_> = first
            .paths
            .iter()
            .map(|path| read_file(Path::new(path)).unwrap())
            .collect();
        let second = automatic_preview(&f.service, &f);
        let new = mapping_context(&f.service, &second);
        let new_plan = new.mapping.as_ref().unwrap();
        let saved = f
            .service
            .apply_client_config(&second.id, true, false)
            .unwrap();
        tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
        assert_ne!(old_plan.port, new_plan.port);
        if logout {
            f.service.commit_logout(&f.record.view.id).unwrap();
        }
        // Match the real native restore commands: synchronous file/Vault work
        // runs in spawn_blocking, allowing the listener runtime to honor stop.
        let service = f.service.clone();
        let source = saved.snapshot_id.clone().unwrap();
        let restore = tokio::task::spawn_blocking(move || service.preview_restore(&source))
            .await
            .unwrap()
            .unwrap();
        let service = f.service.clone();
        let id = restore.id.clone();
        let result =
            tokio::task::spawn_blocking(move || service.apply_client_config(&id, true, true))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(result.state, "restored");
        for (path, bytes) in first.paths.iter().zip(original) {
            assert_eq!(read_file(Path::new(path)).unwrap(), bytes);
        }
        let token = crate::claude_mapping::mapped_helper_token(
            &old_plan.binding_id,
            &f.service.store,
            f.vault.as_ref(),
            chrono::Utc::now().timestamp(),
        );
        if logout {
            assert!(token.is_err());
            assert!(
                f.service
                    .store
                    .list::<HelperBinding>("binding", None)
                    .unwrap()
                    .iter()
                    .all(|binding| !binding.enabled)
            );
        } else {
            let token = token.unwrap();
            assert_eq!(
                reqwest::Client::builder()
                    .no_proxy()
                    .build()
                    .unwrap()
                    .get(format!("{}/v1/models", old_plan.base_url()))
                    .bearer_auth(token.as_str())
                    .send()
                    .await
                    .unwrap()
                    .status(),
                200
            );
        }
        assert!(
            crate::claude_mapping::mapped_helper_token(
                &new_plan.binding_id,
                &f.service.store,
                f.vault.as_ref(),
                chrono::Utc::now().timestamp()
            )
            .is_err()
        );
        f.service.stop_claude_mappings();
    }
}

#[tokio::test]
async fn claude_mapping_activation_vault_return_rechecks_epoch_target_node_model_and_actor() {
    for fault in ["epoch", "target", "node", "model", "actor"] {
        let f = mapping_fixture();
        let preview = automatic_preview(&f.service, &f);
        let context = mapping_context(&f.service, &preview);
        let plan = context.mapping.as_ref().unwrap();
        f.service
            .apply_client_config(&preview.id, true, false)
            .unwrap();
        let record = f
            .service
            .store
            .get::<crate::claude_mapping::MappingRecord>("claude-mapping", &plan.binding_id)
            .unwrap()
            .unwrap();
        let authority = f.service.claude_mapping_authority(record);
        let local = crate::claude_mapping::mapped_helper_token(
            &plan.binding_id,
            &f.service.store,
            f.vault.as_ref(),
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
        let access = authority.authenticate(local.as_bytes()).unwrap();
        assert_eq!(
            f.service.begin_maintenance().err().unwrap().code,
            "update_busy"
        );
        let (entered, waiting) = std::sync::mpsc::channel();
        let (resume, released) = std::sync::mpsc::channel();
        *f.vault.pause.lock().unwrap() = Some((entered, released));
        let worker = std::thread::spawn(move || {
            authority.authorize_messages(access, "claude-sonnet-5".into())
        });
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        match fault {
            "epoch" => f
                .service
                .store
                .set_meta(
                    &authorization_epoch_reference(
                        &context.panel_origin,
                        &context.installation_id,
                        &context.account_id,
                    ),
                    "999",
                )
                .unwrap(),
            "target" => f.target.store(1, Ordering::SeqCst),
            "actor" => {
                let mut record = f.record.clone();
                record.view.account_id = "synthetic-other-actor".into();
                f.service
                    .store
                    .put(
                        "connection",
                        &record.view.id,
                        &record.view.account_id,
                        &record,
                    )
                    .unwrap();
            }
            _ => {
                let mut cache = f.service.account_cache(&f.record).unwrap();
                if fault == "node" {
                    cache.nodes[0].available = false;
                } else {
                    cache.models[0].revision = sha256(b"late mapping model");
                }
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
        }
        resume.send(()).unwrap();
        assert!(worker.join().unwrap().is_err(), "{fault}");
        f.service.stop_claude_mappings();
    }
}

#[tokio::test]
async fn claude_mapping_activation_restore_success_is_independent_of_old_listener_live_scope() {
    let f = mapping_fixture();
    let first = automatic_preview(&f.service, &f);
    let old = mapping_context(&f.service, &first);
    let plan = old.mapping.as_ref().unwrap();
    f.service
        .apply_client_config(&first.id, true, false)
        .unwrap();
    let originals: Vec<_> = first
        .paths
        .iter()
        .map(|path| read_file(Path::new(path)).unwrap())
        .collect();
    let second = automatic_preview(&f.service, &f);
    let saved = f
        .service
        .apply_client_config(&second.id, true, false)
        .unwrap();
    tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
    let mut cache = f.service.account_cache(&f.record).unwrap();
    cache.models[0].revision = sha256(b"new model permission revision");
    f.service
        .store
        .put(
            "account-cache",
            &f.record.view.id,
            &f.record.view.account_id,
            &cache,
        )
        .unwrap();
    let service = f.service.clone();
    let source = saved.snapshot_id.clone().unwrap();
    let restore = tokio::task::spawn_blocking(move || service.preview_restore(&source))
        .await
        .unwrap()
        .unwrap();
    let service = f.service.clone();
    let id = restore.id.clone();
    let result = tokio::task::spawn_blocking(move || service.apply_client_config(&id, true, true))
        .await
        .unwrap();
    for (path, original) in first.paths.iter().zip(originals) {
        assert_eq!(read_file(Path::new(path)).unwrap(), original);
    }
    assert_eq!(
        f.service
            .store
            .get::<Journal>("journal", saved.snapshot_id.as_ref().unwrap())
            .unwrap()
            .unwrap()
            .phase,
        "restored"
    );
    assert_eq!(
        result.unwrap().state,
        "restored",
        "optional listener qualification is not a prerequisite for restoring original bytes"
    );
    let _closed_port = take_stopped_mapping_port(plan.port).await;
    assert!(
        f.service
            .bootstrap()
            .unwrap()
            .clients
            .iter()
            .find(|client| client.client == ClientKind::Claude)
            .unwrap()
            .manual_steps
            .iter()
            .any(|note| note.contains("本机模型映射未就绪"))
    );
}

#[tokio::test]
async fn claude_mapping_activation_recomputed_plain_row_hash_cannot_authorize_a_file_path() {
    let f = mapping_fixture();
    let preview = automatic_preview(&f.service, &f);
    let context = mapping_context(&f.service, &preview);
    let plan = context.mapping.as_ref().unwrap();
    f.service
        .apply_client_config(&preview.id, true, false)
        .unwrap();
    f.service.stop_claude_mappings();
    let _closed = take_stopped_mapping_port(plan.port).await;
    drop(_closed);
    let mut row: crate::claude_mapping::MappingRecord = f
        .service
        .store
        .get("claude-mapping", &plan.binding_id)
        .unwrap()
        .unwrap();
    let outside = f
        .service
        .store
        .root
        .join("synthetic-unqualified-config-must-not-be-followed.json");
    assert!(!outside.exists());
    row.context.files[0].path = outside.display().to_string();
    row.context.scope_revision.clear();
    row.context.scope_revision = sha256(&serde_json::to_vec(&row.context).unwrap());
    f.service
        .store
        .put(
            "claude-mapping",
            &plan.binding_id,
            &row.context.account_id,
            &row,
        )
        .unwrap();
    assert_eq!(
        f.service.reconcile_claude_mappings().unwrap_err().code,
        "credential_missing",
        "the Keychain context pin must reject before a path-read could report missing/conflict"
    );
    assert!(!outside.exists());
}

#[tokio::test]
async fn claude_mapping_activation_stream_permit_blocks_updates_until_observation_finishes() {
    let f = mapping_fixture();
    let calls = Arc::new(AtomicU64::new(0));
    let count = calls.clone();
    let (sender, receiver) = tokio::sync::mpsc::channel(2);
    let receiver = Arc::new(Mutex::new(Some(receiver)));
    f.service.http.scripted_mapping(Arc::new(move|_|{
        assert_eq!(count.fetch_add(1,Ordering::SeqCst),0,"mapping must not resend");
        let receiver=receiver.lock().unwrap().take().unwrap();
        Box::pin(async move{Ok(crate::network::MappingResponse{status:200,headers:Default::default(),stream:Box::pin(futures_util::stream::unfold(receiver,|mut receiver|async move{receiver.recv().await.map(|chunk|(chunk,receiver))}))})})
    }));
    let preview = automatic_preview(&f.service, &f);
    let context = mapping_context(&f.service, &preview);
    let plan = context.mapping.as_ref().unwrap();
    f.service
        .apply_client_config(&preview.id, true, false)
        .unwrap();
    let token = crate::claude_mapping::mapped_helper_token(
        &plan.binding_id,
        &f.service.store,
        f.vault.as_ref(),
        chrono::Utc::now().timestamp(),
    )
    .unwrap();
    sender
        .send(Ok(hyper::body::Bytes::from_static(b"data: {}\n\n")))
        .await
        .unwrap();
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("{}/v1/messages", plan.base_url()))
        .bearer_auth(token.as_str())
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-5","messages":[],"max_tokens":1,"stream":true}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(f.service.active_tasks().unwrap());
    assert_eq!(
        f.service.begin_maintenance().err().unwrap().code,
        "update_busy"
    );
    drop(response);
    drop(sender);
    for _ in 0..100 {
        if !f.service.active_tasks().unwrap()
            && f.service.mutation_count.load(Ordering::SeqCst) == 0
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!f.service.active_tasks().unwrap());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let maintenance = f.service.begin_maintenance().unwrap();
    let row = f
        .service
        .store
        .get("claude-mapping", &plan.binding_id)
        .unwrap()
        .unwrap();
    let authority = f.service.claude_mapping_authority(row);
    assert_eq!(
        authority.authenticate(token.as_bytes()).err().unwrap().code,
        "update_busy"
    );
    drop(maintenance);
    f.service.stop_claude_mappings();
}

#[tokio::test]
async fn claude_mapping_activation_active_token_outlives_backup_prune_and_cleanup_keeps_mode_guard_closed()
 {
    let f = mapping_fixture();
    let preview = automatic_preview(&f.service, &f);
    let context = mapping_context(&f.service, &preview);
    let plan = context.mapping.as_ref().unwrap();
    let saved = f
        .service
        .apply_client_config(&preview.id, true, false)
        .unwrap();
    tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
    let mut row: crate::claude_mapping::MappingRecord = f
        .service
        .store
        .get("claude-mapping", &plan.binding_id)
        .unwrap()
        .unwrap();
    row.retained_until = (chrono::Utc::now() - chrono::Duration::days(1)).to_rfc3339();
    f.service
        .store
        .put(
            "claude-mapping",
            &plan.binding_id,
            &row.context.account_id,
            &row,
        )
        .unwrap();
    f.service
        .store
        .delete("backup", saved.snapshot_id.as_ref().unwrap())
        .unwrap();
    f.service
        .store
        .delete("journal", saved.snapshot_id.as_ref().unwrap())
        .unwrap();
    f.service.reconcile_claude_mappings().unwrap();
    assert!(
        crate::claude_mapping::mapped_helper_token(
            &plan.binding_id,
            &f.service.store,
            f.vault.as_ref(),
            chrono::Utc::now().timestamp()
        )
        .is_ok(),
        "active service token is not a backup to prune"
    );
    f.service.commit_logout(&f.record.view.id).unwrap();
    f.service.reconcile_claude_mappings().unwrap();
    assert!(
        f.service
            .store
            .get::<crate::claude_mapping::MappingRecord>("claude-mapping", &plan.binding_id)
            .unwrap()
            .is_none()
    );
    assert!(f.vault.get(&plan.token_reference()).is_err());
    let binding: HelperBinding = f
        .service
        .store
        .get("binding", &plan.binding_id)
        .unwrap()
        .unwrap();
    assert!(!binding.enabled);
    assert_eq!(
        crate::claude_mapping::unmapped_helper_token(
            &binding,
            ClientKind::Claude,
            &plan.binding_id,
            &f.service.store,
            f.vault.as_ref(),
            chrono::Utc::now().timestamp()
        )
        .unwrap_err()
        .code,
        "binding_disabled"
    );

    let unconfirmed = mapping_fixture();
    let preview = automatic_preview(&unconfirmed.service, &unconfirmed);
    let context = mapping_context(&unconfirmed.service, &preview);
    let plan = context.mapping.as_ref().unwrap();
    assert!(
        unconfirmed
            .vault
            .get_optional(&plan.token_reference())
            .unwrap()
            .is_none()
    );
    let mut row: crate::claude_mapping::MappingRecord = unconfirmed
        .service
        .store
        .get("claude-mapping", &plan.binding_id)
        .unwrap()
        .unwrap();
    row.retained_until = (chrono::Utc::now() - chrono::Duration::days(1)).to_rfc3339();
    unconfirmed
        .service
        .store
        .put(
            "claude-mapping",
            &plan.binding_id,
            &row.context.account_id,
            &row,
        )
        .unwrap();
    unconfirmed
        .service
        .previews
        .lock()
        .unwrap()
        .remove(&preview.id);
    let mutations = unconfirmed.vault.mutations.load(Ordering::SeqCst);
    unconfirmed.service.reconcile_claude_mappings().unwrap();
    assert!(
        unconfirmed
            .service
            .store
            .get::<crate::claude_mapping::MappingRecord>("claude-mapping", &plan.binding_id)
            .unwrap()
            .is_none(),
        "an expired unconfirmed preview has no secret to retain"
    );
    assert_eq!(
        unconfirmed.vault.mutations.load(Ordering::SeqCst),
        mutations,
        "known absence needs no Vault put or delete"
    );
    assert_eq!(
        unconfirmed
            .service
            .store
            .meta("claude_mapping_cleanup_pending")
            .unwrap()
            .as_deref(),
        Some("0")
    );
    let binding: HelperBinding = unconfirmed
        .service
        .store
        .get("binding", &plan.binding_id)
        .unwrap()
        .unwrap();
    assert!(!binding.enabled);

    // A present malformed item is not absence and cannot remove the mode guard.
    unconfirmed
        .service
        .store
        .put(
            "claude-mapping",
            &plan.binding_id,
            &row.context.account_id,
            &row,
        )
        .unwrap();
    unconfirmed
        .vault
        .put(&plan.token_reference(), b"{}")
        .unwrap();
    let mutations = unconfirmed.vault.mutations.load(Ordering::SeqCst);
    unconfirmed.service.reconcile_claude_mappings().unwrap();
    assert!(
        unconfirmed
            .service
            .store
            .get::<crate::claude_mapping::MappingRecord>("claude-mapping", &plan.binding_id)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        unconfirmed.vault.mutations.load(Ordering::SeqCst),
        mutations
    );
    assert_eq!(
        unconfirmed
            .service
            .store
            .meta("claude_mapping_cleanup_pending")
            .unwrap()
            .as_deref(),
        Some("1")
    );
}

#[test]
fn claude_mapping_none_keeps_legacy_scope_bytes_and_backup_restoration() {
    let mut f = tests::policy_fixture(ClientKind::Claude);
    let mut cache = f.service.account_cache(&f.record).unwrap();
    cache.models[0].id = "claude-sonnet-5".into();
    f.model = cache.models[0].id.clone();
    f.service
        .store
        .put(
            "account-cache",
            &f.record.view.id,
            &f.record.view.account_id,
            &cache,
        )
        .unwrap();
    let preview = automatic_preview(&f.service, &f);
    let context = f
        .service
        .previews
        .lock()
        .unwrap()
        .get(&preview.id)
        .unwrap()
        .claude_context
        .clone()
        .unwrap();
    let old_wire = serde_json::to_vec(&context).unwrap();
    assert!(
        !serde_json::from_slice::<Value>(&old_wire)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("mapping")
    );
    let mut restored: ClaudeContext = serde_json::from_slice(&old_wire).unwrap();
    assert!(restored.mapping.is_none());
    assert_eq!(serde_json::to_vec(&restored).unwrap(), old_wire);
    let old_revision = restored.scope_revision.clone();
    restored.scope_revision.clear();
    assert_eq!(
        sha256(&serde_json::to_vec(&restored).unwrap()),
        old_revision,
        "a missing mapping field must not add null or change the historical scope digest"
    );
    let saved = f
        .service
        .apply_client_config(&preview.id, true, false)
        .unwrap();
    tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
    let restore = f
        .service
        .preview_restore(saved.snapshot_id.as_ref().unwrap())
        .unwrap();
    assert_eq!(
        f.service
            .apply_client_config(&restore.id, true, true)
            .unwrap()
            .state,
        "restored"
    );
    assert!(preview.paths.iter().all(|path| !Path::new(path).exists()));
}

#[test]
fn claude_mapping_helper_modes_and_late_epoch_never_expose_the_upstream_key() {
    use crate::claude_mapping::{
        MappingManager, MappingRecord, ensure_local_secret, mapped_helper_token,
        unmapped_helper_token,
    };
    let f = tests::policy_fixture(ClientKind::Claude);
    let observation = f.service.observe_configuration(ClientKind::Claude).unwrap();
    let target = observation.claude_configuration_target.as_ref().unwrap();
    let mut context = frozen(&f);
    let manager = MappingManager::default();
    let mapping = manager
        .reserve(&context.binding_id, &context.profile_id, &f.model, None)
        .unwrap();
    let compiled = crate::claude_config::compile(&crate::claude_config::LibraryPlan {
        app_version: &target.app_version,
        source_member_sha256: &target.source_member_sha256,
        user_root: &target.config_root,
        profile_id: &context.profile_id,
        gateway_base_url: &context.claude_base_url,
        helper: &f.service.paths.helper,
        binding_id: &context.binding_id,
        model_id: &f.model,
        mapping: Some(&mapping),
        profile_before: None,
        metadata_before: None,
        main_before: None,
        active_profile: None,
    })
    .unwrap();
    context.mapping = Some(mapping);
    context.files = compiled
        .files()
        .iter()
        .map(crate::config::ClaudeFileDigest::from_change)
        .collect::<Result<_>>()
        .unwrap();
    context.scope_revision.clear();
    context.scope_revision = sha256(&serde_json::to_vec(&context).unwrap());
    let binding = HelperBinding {
        id: context.binding_id.clone(),
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
    f.service
        .store
        .put("binding", &binding.id, &binding.account_id, &binding)
        .unwrap();
    let policy = || {
        f.service
            .validate_claude_configuration(&observation.policy, Some(target))
    };
    let scope = |current: &ClaudeContext, restoring: bool| {
        assert_eq!(current, &context);
        f.service.validate_claude_context(current, restoring)
    };
    let engine = ConfigEngine {
        store: &f.service.store,
        vault: f.vault.as_ref(),
        allowed_roots: f.service.paths.configuration_roots.clone(),
        policy_guard: Some(&policy),
        context_guard: None,
        claude_context_guard: Some(&scope),
    };
    let install = f.service.installation(&f.record).unwrap();
    let journal = engine
        .prepare_claude(
            &context.account_id,
            &install,
            compiled.into_files(),
            vec![],
            context.clone(),
        )
        .unwrap();
    let mut record = MappingRecord {
        journal_id: journal.id.clone(),
        context: context.clone(),
        committed: false,
        retained_until: journal.expires_at.clone(),
    };
    f.service
        .store
        .put("claude-mapping", &binding.id, &binding.account_id, &record)
        .unwrap();
    for error in ["credential_missing", "vault_unavailable"] {
        let denied = MappingReadFailureVault {
            inner: f.vault.clone(),
            reference: record.plan().unwrap().token_reference(),
            error,
            puts: AtomicU64::new(0),
            deleted: Mutex::new(Vec::new()),
        };
        assert_eq!(
            ensure_local_secret(&record, &binding, &denied)
                .unwrap_err()
                .code,
            error
        );
        assert!(crate::claude_mapping::verify_context_secret(&record, &denied).is_err());
        assert_eq!(
            denied.puts.load(Ordering::SeqCst),
            0,
            "an ambiguous read failure cannot replace an existing secret"
        );
        assert!(denied.deleted.lock().unwrap().is_empty());
    }
    ensure_local_secret(&record, &binding, f.vault.as_ref()).unwrap();
    let reference = record.plan().unwrap().token_reference();
    let original_secret = f.vault.get(&reference).unwrap();
    let mutations = f.vault.mutations.load(Ordering::SeqCst);
    ensure_local_secret(&record, &binding, f.vault.as_ref()).unwrap();
    assert_eq!(f.vault.mutations.load(Ordering::SeqCst), mutations);
    assert_eq!(
        f.vault.get(&reference).unwrap().as_slice(),
        original_secret.as_slice(),
        "existing authenticated token is reused without create/update/ACL migration"
    );
    for corrupt in [b"{}".to_vec(), {
        let mut value: Value = serde_json::from_slice(&original_secret).unwrap();
        value["context_revision"] = json!(sha256(b"wrong context pin"));
        serde_json::to_vec(&value).unwrap()
    }] {
        f.vault.put(&reference, &corrupt).unwrap();
        let mutations = f.vault.mutations.load(Ordering::SeqCst);
        assert_eq!(
            ensure_local_secret(&record, &binding, f.vault.as_ref())
                .unwrap_err()
                .code,
            "credential_missing"
        );
        assert!(crate::claude_mapping::verify_context_secret(&record, f.vault.as_ref()).is_err());
        assert_eq!(f.vault.mutations.load(Ordering::SeqCst), mutations);
        assert!(f.vault.get(&reference).unwrap().as_slice() == corrupt.as_slice());
    }
    f.vault.put(&reference, &original_secret).unwrap();
    assert_eq!(
        mapped_helper_token(
            &binding.id,
            &f.service.store,
            f.vault.as_ref(),
            chrono::Utc::now().timestamp()
        )
        .unwrap_err()
        .code,
        "binding_disabled"
    );
    engine.apply(&journal.id).unwrap();
    record.committed = true;
    f.service
        .store
        .put("claude-mapping", &binding.id, &binding.account_id, &record)
        .unwrap();
    let binding: HelperBinding = f
        .service
        .store
        .get("binding", &binding.id)
        .unwrap()
        .unwrap();
    let local = mapped_helper_token(
        &binding.id,
        &f.service.store,
        f.vault.as_ref(),
        chrono::Utc::now().timestamp(),
    )
    .unwrap();
    assert!(local.starts_with("orbt_map_"));
    let upstream = bound_helper_token(
        &binding,
        ClientKind::Claude,
        &binding.id,
        &f.service.store,
        f.vault.as_ref(),
        chrono::Utc::now().timestamp(),
    )
    .unwrap();
    assert!(upstream.starts_with("orv_sk_") && upstream.as_str() != local.as_str());
    assert_eq!(
        unmapped_helper_token(
            &binding,
            ClientKind::Claude,
            &binding.id,
            &f.service.store,
            f.vault.as_ref(),
            chrono::Utc::now().timestamp()
        )
        .unwrap_err()
        .code,
        "binding_disabled",
        "changing executable helper mode must not reveal a mapped binding's real key"
    );
    let mut direct = binding.clone();
    direct.id = uuid();
    f.service
        .store
        .put("binding", &direct.id, &direct.account_id, &direct)
        .unwrap();
    let legacy = unmapped_helper_token(
        &direct,
        ClientKind::Claude,
        &direct.id,
        &f.service.store,
        f.vault.as_ref(),
        chrono::Utc::now().timestamp(),
    )
    .unwrap();
    assert!(
        legacy.as_str() == upstream.as_str(),
        "an unrelated original direct binding keeps its old helper contract"
    );
    assert!(
        mapped_helper_token(
            &direct.id,
            &f.service.store,
            f.vault.as_ref(),
            chrono::Utc::now().timestamp()
        )
        .is_err()
    );
    let (entered, waiting) = std::sync::mpsc::channel();
    let (resume, released) = std::sync::mpsc::channel();
    *f.vault.pause.lock().unwrap() = Some((entered, released));
    let store = f.service.store.clone();
    let vault = f.vault.clone();
    let id = binding.id.clone();
    let worker = std::thread::spawn(move || {
        mapped_helper_token(&id, &store, vault.as_ref(), chrono::Utc::now().timestamp())
    });
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    f.service
        .store
        .set_meta(
            &authorization_epoch_reference(
                &binding.origin,
                &binding.installation_id,
                &binding.account_id,
            ),
            "999",
        )
        .unwrap();
    resume.send(()).unwrap();
    assert_eq!(worker.join().unwrap().unwrap_err().code, "binding_disabled");
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
        tests::mark_switch_snapshot(&service, saved.snapshot_id.as_ref().unwrap());
        let apply_full = full.swap(0, Ordering::SeqCst);
        let restore = service
            .preview_restore(saved.snapshot_id.as_ref().unwrap())
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
    tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
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
            .preview_restore(saved.snapshot_id.as_ref().unwrap())
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
    tests::mark_switch_snapshot(&fixture.service, saved.snapshot_id.as_ref().unwrap());
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
    // The mapper owns a weak self-reference. Rebuild this uniquely owned
    // direct fixture to keep the strict production-origin Http unchanged.
    let mut service = Arc::try_unwrap(fixture.service)
        .unwrap_or_else(|_| panic!("fixture service must be uniquely owned"));
    service.http = http;
    fixture.service = Arc::new_cyclic(|self_reference| {
        service.self_reference = self_reference.clone();
        service
    });
    let restore = fixture
        .service
        .preview_restore(saved.snapshot_id.as_ref().unwrap())
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
    let mut fixture = mapping_fixture();
    let wire: Value =
        serde_json::from_str(include_str!("../tests/fixtures/public-wire.json")).unwrap();
    let bootstrap = wire["bootstrap_reply"].clone();
    // The immutable wire exposes a non-Claude route. Keep this command-level
    // regression on that real shape rather than inheriting the direct fixture.
    let mut cache = fixture.service.account_cache(&fixture.record).unwrap();
    fixture.model = bootstrap["data"]["models"][0]["id"]
        .as_str()
        .unwrap()
        .into();
    cache.models[0].id = fixture.model.clone();
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
    tests::mark_switch_snapshot(&fixture.service, saved.snapshot_id.as_ref().unwrap());
    assert_eq!(saved.state, "saved");
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        ["bootstrap", "models", "bootstrap", "models"]
    );
    assert!(preview.paths.iter().all(|path| Path::new(path).is_file()));
    assert_unobserved(&fixture.service, "saved");
    let restore = fixture
        .service
        .preview_restore(saved.snapshot_id.as_ref().unwrap())
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

fn set_snapshot_timestamp(service: &Service, id: &str, expires_at: &str) {
    let mut source: Journal = service.store.get("journal", id).unwrap().unwrap();
    source.expires_at = expires_at.into();
    service
        .store
        .put("journal", id, &source.account_id, &source)
        .unwrap();
}

#[test]
fn claude_switch_snapshot_remains_restorable_after_its_creation_timestamp_deadline() {
    let f = tests::policy_fixture(ClientKind::Claude);
    let apply = automatic_preview(&f.service, &f);
    let saved = f
        .service
        .apply_client_config(&apply.id, true, false)
        .unwrap();
    tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
    let source_id = saved.snapshot_id.unwrap();
    let old_deadline = (chrono::Utc::now() - chrono::Duration::days(60)).to_rfc3339();
    set_snapshot_timestamp(&f.service, &source_id, &old_deadline);
    let restore = f.service.preview_restore(&source_id).unwrap();
    assert!(restore.can_apply);
    assert_eq!(
        serde_json::to_value(&restore).unwrap()["restore_source"],
        json!({"snapshot_id": source_id, "expires_at": old_deadline})
    );
    assert!(
        chrono::DateTime::parse_from_rfc3339(&restore.expires_at).unwrap() > chrono::Utc::now()
    );
    f.service.commit_logout(&f.record.view.id).unwrap();
    assert_eq!(
        f.service
            .apply_client_config(&restore.id, true, true)
            .unwrap()
            .state,
        "restored"
    );
}

#[test]
fn claude_switch_restore_expired_confirmation_refuses_before_files_or_grants() {
    let f = tests::policy_fixture(ClientKind::Claude);
    let apply = automatic_preview(&f.service, &f);
    let saved = f
        .service
        .apply_client_config(&apply.id, true, false)
        .unwrap();
    tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
    let restore = f
        .service
        .preview_restore(saved.snapshot_id.as_ref().unwrap())
        .unwrap();
    let before = apply
        .paths
        .iter()
        .map(|path| read_file(Path::new(path)).unwrap())
        .collect::<Vec<_>>();
    let before_grants = grants(&f.service);
    f.service
        .previews
        .lock()
        .unwrap()
        .get_mut(&restore.id)
        .unwrap()
        .view
        .expires_at = (chrono::Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
    f.vault.calls.store(0, Ordering::SeqCst);
    f.vault.mutations.store(0, Ordering::SeqCst);
    assert_eq!(
        f.service
            .apply_client_config(&restore.id, true, true)
            .unwrap_err()
            .code,
        "preview_expired"
    );
    assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0);
    assert_eq!(grants(&f.service), before_grants);
    assert_eq!(
        apply
            .paths
            .iter()
            .map(|path| read_file(Path::new(path)).unwrap())
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn claude_switch_restore_publishing_ignores_old_snapshot_timestamp_without_reviving_revoked_grants()
{
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
        tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
        let source_id = saved.snapshot_id.unwrap();
        let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(-1)).to_rfc3339();
        set_snapshot_timestamp(&f.service, &source_id, &expires_at);
        let restore = f.service.preview_restore(&source_id).unwrap();
        let mut restoring = preview_journal(&f.service, &restore);
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
        // The transaction is terminal; switch snapshots have no backup retention policy.
        assert_eq!(
            recovered
                .store
                .get::<Journal>("journal", &source_id)
                .unwrap()
                .unwrap()
                .phase,
            "restored"
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
        let mut bootstrap = wire["bootstrap_reply"].clone();
        // Keep the valid baseline aligned with this direct Claude fixture so
        // each late response below is rejected for its intended scope change.
        bootstrap["data"]["models"][0]["id"] = json!(f.model);
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
        let mut token = tests::token_reply()["data"].clone();
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
        tests::mark_switch_snapshot(&f.service, first_saved.snapshot_id.as_ref().unwrap());
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
        tests::mark_switch_snapshot(&f.service, second_saved.snapshot_id.as_ref().unwrap());
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
            .preview_restore(second_saved.snapshot_id.as_ref().unwrap())
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
            .preview_restore(first_saved.snapshot_id.as_ref().unwrap())
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
    tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
    let restore = f
        .service
        .preview_restore(saved.snapshot_id.as_ref().unwrap())
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
        tests::mark_switch_snapshot(&fixture.service, saved.snapshot_id.as_ref().unwrap());
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
            .preview_restore(saved.snapshot_id.as_ref().unwrap())
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
    tests::mark_switch_snapshot(&fixture.service, saved.snapshot_id.as_ref().unwrap());
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
        .preview_restore(saved.snapshot_id.as_ref().unwrap())
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
    tests::mark_switch_snapshot(&fixture.service, saved.snapshot_id.as_ref().unwrap());
    let id = saved.snapshot_id.as_ref().unwrap();
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
            .is_none()
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
    tests::mark_switch_snapshot(&fixture.service, saved.snapshot_id.as_ref().unwrap());
    let source_id = saved.snapshot_id.as_ref().unwrap();
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
        mapping: None,
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

#[tokio::test]
async fn compatible_major_upgrade_preserves_saved_mapping_and_restore() {
    let f = mapping_fixture();
    let preview = automatic_preview(&f.service, &f);
    let saved = f
        .service
        .apply_client_config(&preview.id, true, false)
        .unwrap();
    tests::mark_switch_snapshot(&f.service, saved.snapshot_id.as_ref().unwrap());
    {
        let mut clients = f.service.clients.lock().unwrap();
        let client = clients
            .iter_mut()
            .find(|c| c.client == ClientKind::Claude)
            .unwrap();
        client.version = Some("999.0.0".into());
    }
    f.service
        .configuration_preflight(ClientKind::Claude)
        .unwrap();
    f.service.reconcile_claude_mappings().unwrap();
    let restore = f
        .service
        .preview_restore(saved.snapshot_id.as_deref().unwrap())
        .unwrap();
    assert!(restore.can_apply);
    f.service
        .apply_client_config(&restore.id, true, true)
        .unwrap();
    f.service.stop_claude_mappings();
}
