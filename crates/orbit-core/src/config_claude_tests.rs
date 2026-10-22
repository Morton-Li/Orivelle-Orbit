use super::*;
use crate::{claude_config, identity::InferenceSecret, vault::FakeVault};
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Fixture {
    _directory: tempfile::TempDir,
    store: Store,
    vault: FakeVault,
    installation: Installation,
    context: ClaudeContext,
    files: Vec<FileChange>,
}

impl Fixture {
    fn new(existing: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("home");
        fs::create_dir_all(home.join("Library/Application Support")).unwrap();
        let root = home.join("Library/Application Support/Claude-3p");
        let store = Store::open(&directory.path().join("orbit")).unwrap();
        let vault = FakeVault::default();
        let installation = Installation::create("https://panel.test", &vault).unwrap();
        let binding = Self::grant(&store, &vault, &installation, "new", false);
        store
            .set_meta(
                &crate::identity::authorization_epoch_reference(
                    &installation.origin,
                    &installation.id,
                    "actor",
                ),
                "1",
            )
            .unwrap();
        let profile_id = uuid();
        let active_id = uuid();
        let active_bytes = br#"{"inferenceProvider":"gateway","inferenceGatewayApiKey":"synthetic-old","keepAwakeEnabled":true}"#;
        let metadata = serde_json::to_vec(&serde_json::json!({
            "appliedId": active_id,
            "entries": [{"id": active_id, "name": "Existing"}], "unowned": true
        }))
        .unwrap();
        let main = br#"{"deploymentMode":"1p","unowned":{"theme":"system"}}"#;
        if existing {
            fs::create_dir_all(root.join("configLibrary")).unwrap();
            fs::write(
                root.join("configLibrary").join(format!("{active_id}.json")),
                active_bytes,
            )
            .unwrap();
            fs::write(root.join("configLibrary/_meta.json"), &metadata).unwrap();
            fs::write(root.join("claude_desktop_config.json"), main).unwrap();
        }
        let target = ClaudeConfigurationTarget::synthetic(root.clone()).unwrap();
        let compiled = claude_config::compile(&claude_config::LibraryPlan {
            app_version: &target.app_version,
            source_member_sha256: &target.source_member_sha256,
            user_root: &root,
            profile_id: &profile_id,
            gateway_base_url: "https://claude.edge.test",
            helper: Path::new("/synthetic/orbit-credential-helper"),
            binding_id: &binding.id,
            model_id: "claude-test-model",
            profile_before: None,
            metadata_before: existing.then_some(metadata.as_slice()),
            main_before: existing.then_some(main.as_slice()),
            active_profile: existing.then_some(claude_config::ActiveProfile {
                id: &active_id,
                bytes: active_bytes,
            }),
        })
        .unwrap();
        let files = compiled.into_files();
        let context = ClaudeContext {
            contract_version: 1,
            panel_origin: installation.origin.clone(),
            account_id: "actor".into(),
            installation_id: installation.id.clone(),
            connection_id: uuid(),
            connection_revision: 1,
            authorization_epoch: 1,
            credential_revision: binding.credential_revision.clone(),
            credential_reference: binding.secret_reference.clone(),
            credential_expires_at: None,
            api_key_id: Some("synthetic-key-id".into()),
            node_id: "node".into(),
            node_revision: "node-1".into(),
            claude_base_url: "https://claude.edge.test".into(),
            provider_id: binding.provider_id.clone(),
            catalog_revision: "catalog-1".into(),
            models: vec![ClaudeModelInput {
                id: "claude-test-model".into(),
                provider_id: binding.provider_id,
                revision: "model-1".into(),
                category: "text".into(),
                capabilities: vec!["messages".into()],
                support_status: "unverified".into(),
                reasoning: None,
            }],
            selected_model_id: "claude-test-model".into(),
            selected_model_revision: "model-1".into(),
            scope_revision: sha256(b"synthetic-scope"),
            configuration_target: target,
            policy_revision: sha256(b"synthetic-full-policy"),
            local_policy_revision: sha256(b"synthetic-local-policy"),
            profile_id,
            binding_id: binding.id,
            helper_path: "/synthetic/orbit-credential-helper".into(),
            files: files
                .iter()
                .map(ClaudeFileDigest::from_change)
                .collect::<Result<_>>()
                .unwrap(),
            active_source: existing.then(|| ClaudeActiveSource {
                profile_id: active_id.clone(),
                path: root
                    .join("configLibrary")
                    .join(format!("{active_id}.json"))
                    .display()
                    .to_string(),
                sha256: sha256(active_bytes),
                size: active_bytes.len() as u64,
            }),
        };
        Self {
            _directory: directory,
            store,
            vault,
            installation,
            context,
            files,
        }
    }

    fn grant(
        store: &Store,
        vault: &FakeVault,
        installation: &Installation,
        revision: &str,
        enabled: bool,
    ) -> HelperBinding {
        let binding = HelperBinding {
            id: uuid(),
            client: ClientKind::Claude,
            origin: installation.origin.clone(),
            account_id: "actor".into(),
            installation_id: installation.id.clone(),
            provider_id: "anthropic".into(),
            credential_revision: revision.into(),
            secret_reference: format!("synthetic-{revision}"),
            enabled,
            expires_at: None,
            authorization_epoch: 1,
        };
        vault
            .put(
                &binding.secret_reference,
                &serde_json::to_vec(&InferenceSecret {
                    origin: binding.origin.clone(),
                    account_id: binding.account_id.clone(),
                    installation_id: binding.installation_id.clone(),
                    provider_id: binding.provider_id.clone(),
                    credential_revision: binding.credential_revision.clone(),
                    token: "orv_sk_public_synthetic".into(),
                    expires_at: None,
                    authorization_epoch: 1,
                })
                .unwrap(),
            )
            .unwrap();
        store
            .put("binding", &binding.id, &binding.account_id, &binding)
            .unwrap();
        binding
    }

    fn engine<'a>(
        &'a self,
        guard: &'a dyn Fn(&ClaudeContext, bool) -> Result<()>,
    ) -> ConfigEngine<'a> {
        ConfigEngine {
            store: &self.store,
            vault: &self.vault,
            allowed_roots: vec![self.context.configuration_target.config_root.clone()],
            policy_guard: None,
            context_guard: None,
            claude_context_guard: Some(guard),
        }
    }

    fn prepare(&self, engine: &ConfigEngine<'_>) -> Journal {
        engine
            .prepare_claude(
                "actor",
                &self.installation,
                self.files.clone(),
                vec![],
                self.context.clone(),
            )
            .unwrap()
    }

    fn saved_count(&self, restoring: bool) -> usize {
        self.files
            .iter()
            .filter(|file| {
                equal_current(file, if restoring { &file.before } else { &file.after }).unwrap()
            })
            .count()
    }

    fn assert_prefix(&self, count: usize, restoring: bool) {
        for (index, file) in self.files.iter().enumerate() {
            let written = if restoring {
                2 - index < count
            } else {
                index < count
            };
            let expected = if written != restoring {
                &file.after
            } else {
                &file.before
            };
            assert!(
                equal_current(file, expected).unwrap(),
                "file {index}, cut {count}, restore {restoring}"
            );
        }
    }

    fn binding(&self, id: &str) -> Option<HelperBinding> {
        self.store.get("binding", id).unwrap()
    }
}

#[test]
fn claude_b2a_v3_context_authenticates_exact_group_and_preview_has_no_side_effects() {
    let fixture = Fixture::new(false);
    let guard = |_: &ClaudeContext, _: bool| Ok(());
    let engine = fixture.engine(&guard);
    let journal = fixture.prepare(&engine);
    assert!(ConfigEngine::aad(&journal).starts_with("orbit-config-v3\n"));
    assert_eq!(
        engine.claude_operation(&journal).unwrap(),
        (fixture.context.clone(), false)
    );
    assert!(!fixture.context.configuration_target.config_root.exists());
    assert!(
        !fixture
            .store
            .get::<HelperBinding>("binding", &fixture.context.binding_id)
            .unwrap()
            .unwrap()
            .enabled
    );
    engine.apply(&journal.id).unwrap();
    fixture.assert_prefix(3, false);
    assert!(
        fixture
            .binding(&fixture.context.binding_id)
            .unwrap()
            .enabled
    );
    use std::os::unix::fs::PermissionsExt;
    for path in [
        &fixture.context.configuration_target.config_root,
        &fixture
            .context
            .configuration_target
            .config_root
            .join("configLibrary"),
    ] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    let mut changed = journal.clone();
    changed.target_policy_revision = None;
    assert_eq!(
        engine.decrypt(&changed).err().unwrap().code,
        "backup_invalid"
    );
}

#[test]
fn claude_b2a_null_legacy_mismatched_client_and_unbound_guard_fail_closed() {
    let fixture = Fixture::new(true);
    let guard = |_: &ClaudeContext, _: bool| Ok(());
    let engine = fixture.engine(&guard);
    let journal = fixture.prepare(&engine);
    let operation = engine.decrypt(&journal).unwrap();
    for legacy in [false, true] {
        let mut json = serde_json::to_value(&operation).unwrap();
        if legacy {
            json.as_object_mut().unwrap().remove("claude_context");
        } else {
            json["claude_context"] = serde_json::Value::Null;
        }
        let missing = serde_json::from_value::<Operation>(json).unwrap();
        assert_eq!(
            engine.check_context(&journal, &missing).unwrap_err().code,
            "claude_configuration_context_unverified"
        );
    }
    let mut changed_client = journal.clone();
    changed_client.client = ClientKind::Codex;
    assert_eq!(
        engine
            .check_context(&changed_client, &operation)
            .unwrap_err()
            .code,
        "backup_invalid"
    );
    let mut codex = serde_json::to_value(&fixture.context).unwrap();
    codex.as_object_mut().unwrap().extend(
        serde_json::json!({
            "edge_base_url": "https://codex.edge.test/v1", "source_sha256": "synthetic",
            "catalog_sha256": "synthetic", "catalog_size": 0, "aliases": ["synthetic"],
            "default_alias": "synthetic", "default_reasoning_effort": "none",
            "catalog_path": "/synthetic/models.json", "config_path": "/synthetic/config.toml",
            "configuration_target": null
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let mut mixed = operation.clone();
    mixed.codex_context = Some(serde_json::from_value(codex).unwrap());
    assert_eq!(
        engine.check_context(&journal, &mixed).unwrap_err().code,
        "claude_configuration_context_unverified"
    );
    let unguarded = ConfigEngine {
        claude_context_guard: None,
        ..fixture.engine(&guard)
    };
    assert_eq!(
        unguarded
            .check_context(&journal, &operation)
            .unwrap_err()
            .code,
        "claude_configuration_context_unverified"
    );
    assert_eq!(
        engine
            .prepare(
                ClientKind::Claude,
                "actor",
                &fixture.installation,
                fixture.files.clone(),
                Some(fixture.context.binding_id.clone()),
                vec![]
            )
            .unwrap_err()
            .code,
        "claude_configuration_context_unverified"
    );
    for file in &fixture.files {
        assert!(equal_current(file, &file.before).unwrap());
    }
}

#[test]
fn claude_b2a_changed_order_output_or_active_source_is_refused() {
    let fixture = Fixture::new(true);
    let guard = |_: &ClaudeContext, _: bool| Ok(());
    let engine = fixture.engine(&guard);
    let journal = fixture.prepare(&engine);
    let operation = engine.decrypt(&journal).unwrap();
    for fault in ["order", "hash", "version"] {
        let mut changed = operation.clone();
        match fault {
            "order" => changed.files.swap(0, 1),
            "hash" => changed.files[0].after.as_mut().unwrap().push(b' '),
            "version" => changed.claude_context.as_mut().unwrap().contract_version = 2,
            _ => unreachable!(),
        }
        assert!(engine.check_context(&journal, &changed).is_err());
    }
    fs::write(&fixture.context.active_source.as_ref().unwrap().path, b"{}").unwrap();
    assert_eq!(
        engine.check_context(&journal, &operation).unwrap_err().code,
        "configuration_conflict"
    );
    for file in &fixture.files {
        assert!(equal_current(file, &file.before).unwrap());
    }
}

#[test]
fn claude_b2b_each_apply_and_reverse_restore_cut_reenters_as_a_group() {
    for cut in 0..=3 {
        let fixture = Fixture::new(true);
        let armed = Cell::new(false);
        let restoring = Cell::new(false);
        let current_id = RefCell::new(String::new());
        let guard = |_: &ClaudeContext, _: bool| {
            if armed.get()
                && fixture.saved_count(restoring.get()) == cut
                && fixture
                    .store
                    .get::<Journal>("journal", &current_id.borrow())?
                    .is_some_and(|journal| journal.phase == "publishing")
            {
                return Err(fail("synthetic_cut"));
            }
            Ok(())
        };
        let engine = fixture.engine(&guard);
        let apply = fixture.prepare(&engine);
        *current_id.borrow_mut() = apply.id.clone();
        armed.set(true);
        assert_eq!(engine.apply(&apply.id).unwrap_err().code, "synthetic_cut");
        fixture.assert_prefix(cut, false);
        assert!(
            !fixture
                .binding(&fixture.context.binding_id)
                .unwrap()
                .enabled
        );
        armed.set(false);
        assert_eq!(engine.recover(&apply.id).unwrap().phase, "committed");
        fixture.assert_prefix(3, false);
        assert!(
            fixture
                .binding(&fixture.context.binding_id)
                .unwrap()
                .enabled
        );
        engine.recover(&apply.id).unwrap();

        let restore = engine.prepare_restore(&apply.id).unwrap();
        assert_eq!(
            restore.paths,
            apply.paths.iter().rev().cloned().collect::<Vec<_>>()
        );
        *current_id.borrow_mut() = restore.id.clone();
        restoring.set(true);
        armed.set(true);
        assert_eq!(engine.apply(&restore.id).unwrap_err().code, "synthetic_cut");
        fixture.assert_prefix(cut, true);
        armed.set(false);
        engine.recover(&restore.id).unwrap();
        fixture.assert_prefix(3, true);
        assert!(
            !fixture
                .binding(&fixture.context.binding_id)
                .unwrap()
                .enabled
        );
        engine.recover(&restore.id).unwrap();
        assert_eq!(
            fixture
                .store
                .get::<Journal>("journal", &apply.id)
                .unwrap()
                .unwrap()
                .phase,
            "restored"
        );
    }
}

#[test]
fn claude_b2b_any_third_state_stops_the_whole_group_and_retires_recovery() {
    for changed_index in [0, 2] {
        let fixture = Fixture::new(true);
        let armed = Cell::new(false);
        let injected = Cell::new(false);
        let guard = |_: &ClaudeContext, _: bool| {
            if armed.get() && !injected.get() && fixture.saved_count(false) == 1 {
                fs::write(&fixture.files[changed_index].path, b"external-third-state").unwrap();
                injected.set(true);
            }
            Ok(())
        };
        let engine = fixture.engine(&guard);
        let journal = fixture.prepare(&engine);
        armed.set(true);
        assert_eq!(
            engine.apply(&journal.id).unwrap_err().code,
            "configuration_conflict"
        );
        assert!(injected.get());
        assert!(equal_current(&fixture.files[1], &fixture.files[1].before).unwrap());
        assert_eq!(
            fs::read(&fixture.files[changed_index].path).unwrap(),
            b"external-third-state"
        );
        assert!(
            !fixture
                .binding(&fixture.context.binding_id)
                .unwrap()
                .enabled
        );
        let stopped = fixture
            .store
            .get::<Journal>("journal", &journal.id)
            .unwrap()
            .unwrap();
        assert_eq!(stopped.phase, "conflict");
        let expected = if changed_index == 0 {
            &fixture.files[0].after
        } else {
            &fixture.files[2].before
        };
        fs::write(
            &fixture.files[changed_index].path,
            expected.as_ref().unwrap(),
        )
        .unwrap();
        assert_eq!(
            engine.recover(&journal.id).unwrap_err().code,
            "configuration_conflict"
        );
    }
}

#[test]
fn claude_b2b_all_after_readback_is_required_before_first_and_each_later_grant() {
    for late_grant in [false, true] {
        let fixture = Fixture::new(true);
        let old = Fixture::grant(
            &fixture.store,
            &fixture.vault,
            &fixture.installation,
            "old",
            true,
        );
        let armed = Cell::new(false);
        let injected = Cell::new(false);
        let guard = |_: &ClaudeContext, _: bool| {
            if armed.get()
                && !injected.get()
                && fixture.saved_count(false) == 3
                && (!late_grant || !fixture.binding(&old.id).unwrap().enabled)
            {
                // Returning a member to its legal BEFORE state is sufficient to
                // forbid enabling credentials until the complete AFTER readback.
                fs::write(
                    &fixture.files[1].path,
                    fixture.files[1].before.as_ref().unwrap(),
                )
                .unwrap();
                injected.set(true);
            }
            Ok(())
        };
        let engine = fixture.engine(&guard);
        let journal = engine
            .prepare_claude(
                "actor",
                &fixture.installation,
                fixture.files.clone(),
                vec![old.id.clone()],
                fixture.context.clone(),
            )
            .unwrap();
        armed.set(true);
        assert_eq!(
            engine.apply(&journal.id).unwrap_err().code,
            "configuration_conflict"
        );
        assert!(injected.get());
        assert!(
            !fixture
                .binding(&fixture.context.binding_id)
                .unwrap()
                .enabled
        );
        assert_eq!(fixture.binding(&old.id).unwrap().enabled, !late_grant);
        assert!(equal_current(&fixture.files[2], &fixture.files[2].after).unwrap());
        assert!(equal_current(&fixture.files[1], &fixture.files[1].before).unwrap());
    }
}

#[test]
fn claude_b2b_late_target_policy_epoch_and_binding_changes_stop_before_next_write() {
    for fault in ["target", "policy", "epoch", "binding"] {
        let fixture = Fixture::new(true);
        let armed = Cell::new(false);
        let injected = Cell::new(false);
        let policy_clear = Cell::new(true);
        let policy = || {
            if policy_clear.get() {
                Ok(())
            } else {
                Err(fail("managed_policy_changed"))
            }
        };
        let guard = |_: &ClaudeContext, _: bool| {
            if armed.get() && !injected.get() && fixture.saved_count(false) == 1 {
                injected.set(true);
                match fault {
                    "target" => return Err(fail("claude_configuration_target_changed")),
                    "policy" => policy_clear.set(false),
                    "epoch" => fixture.store.set_meta(
                        &crate::identity::authorization_epoch_reference(
                            &fixture.installation.origin,
                            &fixture.installation.id,
                            "actor",
                        ),
                        "2",
                    )?,
                    "binding" => fixture
                        .store
                        .delete("binding", &fixture.context.binding_id)?,
                    _ => unreachable!(),
                }
            }
            Ok(())
        };
        let engine = ConfigEngine {
            policy_guard: Some(&policy),
            ..fixture.engine(&guard)
        };
        let journal = fixture.prepare(&engine);
        armed.set(true);
        assert!(engine.apply(&journal.id).is_err(), "fault {fault}");
        assert!(injected.get());
        fixture.assert_prefix(1, false);
        assert!(
            fixture
                .binding(&fixture.context.binding_id)
                .is_none_or(|binding| !binding.enabled)
        );
    }
}

struct HookVault<'a> {
    inner: &'a FakeVault,
    after_get: Box<dyn Fn(&str) + Send + Sync + 'a>,
}
impl Vault for HookVault<'_> {
    fn get(&self, reference: &str) -> Result<zeroize::Zeroizing<Vec<u8>>> {
        let result = self.inner.get(reference)?;
        (self.after_get)(reference);
        Ok(result)
    }
    fn put(&self, reference: &str, bytes: &[u8]) -> Result<()> {
        self.inner.put(reference, bytes)
    }
    fn delete(&self, reference: &str) -> Result<()> {
        self.inner.delete(reference)
    }
}

#[test]
fn claude_b2b_key_revoked_during_vault_return_cannot_enable_a_grant() {
    let fixture = Fixture::new(true);
    let armed = AtomicBool::new(false);
    let injected = AtomicBool::new(false);
    let vault = HookVault {
        inner: &fixture.vault,
        after_get: Box::new(|reference| {
            if armed.load(Ordering::SeqCst)
                && reference == fixture.context.credential_reference
                && fixture.saved_count(false) == 3
                && !injected.swap(true, Ordering::SeqCst)
            {
                fixture.vault.delete(reference).unwrap();
            }
        }),
    };
    let guard = |_: &ClaudeContext, _: bool| Ok(());
    let engine = ConfigEngine {
        vault: &vault,
        ..fixture.engine(&guard)
    };
    let journal = fixture.prepare(&engine);
    armed.store(true, Ordering::SeqCst);
    assert!(engine.apply(&journal.id).is_err());
    assert!(injected.load(Ordering::SeqCst));
    fixture.assert_prefix(3, false);
    assert!(
        !fixture
            .binding(&fixture.context.binding_id)
            .unwrap()
            .enabled
    );
}

#[test]
fn claude_b2b_restore_bytes_survive_deleted_or_invalid_old_and_new_grants_without_reviving_them() {
    for fault in [
        "valid",
        "old_deleted",
        "new_deleted",
        "both_deleted",
        "revoked",
        "logout",
        "forgotten",
        "expired",
    ] {
        let fixture = Fixture::new(true);
        let old = Fixture::grant(
            &fixture.store,
            &fixture.vault,
            &fixture.installation,
            "old",
            true,
        );
        let guard = |_: &ClaudeContext, _: bool| Ok(());
        let engine = fixture.engine(&guard);
        let journal = engine
            .prepare_claude(
                "actor",
                &fixture.installation,
                fixture.files.clone(),
                vec![old.id.clone()],
                fixture.context.clone(),
            )
            .unwrap();
        engine.apply(&journal.id).unwrap();
        assert!(!fixture.binding(&old.id).unwrap().enabled);
        match fault {
            "old_deleted" | "both_deleted" => fixture.store.delete("binding", &old.id).unwrap(),
            "revoked" => fixture.vault.delete(&old.secret_reference).unwrap(),
            "logout" => fixture
                .store
                .set_meta(
                    &crate::identity::authorization_epoch_reference(
                        &fixture.installation.origin,
                        &fixture.installation.id,
                        "actor",
                    ),
                    "2",
                )
                .unwrap(),
            "forgotten" => fixture
                .store
                .set_meta(
                    &format!("installation_forgetting/{}", fixture.installation.id),
                    "1",
                )
                .unwrap(),
            "expired" => {
                let mut expired = fixture.binding(&old.id).unwrap();
                expired.expires_at = Some(chrono::Utc::now().timestamp() - 1);
                fixture
                    .store
                    .put("binding", &old.id, "actor", &expired)
                    .unwrap();
            }
            _ => {}
        }
        if ["new_deleted", "both_deleted"].contains(&fault) {
            fixture
                .store
                .delete("binding", &fixture.context.binding_id)
                .unwrap();
        }
        let restore = engine.prepare_restore(&journal.id).unwrap();
        engine.apply(&restore.id).unwrap();
        fixture.assert_prefix(3, true);
        if ["old_deleted", "both_deleted"].contains(&fault) {
            assert!(fixture.binding(&old.id).is_none());
        } else {
            assert_eq!(
                fixture.binding(&old.id).unwrap().enabled,
                ["valid", "new_deleted"].contains(&fault),
                "fault {fault}"
            );
        }
        if ["new_deleted", "both_deleted"].contains(&fault) {
            assert!(fixture.binding(&fixture.context.binding_id).is_none());
        } else {
            assert!(
                !fixture
                    .binding(&fixture.context.binding_id)
                    .unwrap()
                    .enabled
            );
        }
    }
}

#[test]
fn claude_b2b_late_restore_grant_deletion_and_vault_expiry_keep_original_bytes_and_disabled_authority()
 {
    for fault in ["deleted", "epoch", "expired"] {
        let fixture = Fixture::new(true);
        let old = Fixture::grant(
            &fixture.store,
            &fixture.vault,
            &fixture.installation,
            "old",
            true,
        );
        let guard = |_: &ClaudeContext, _: bool| Ok(());
        let initial = fixture.engine(&guard);
        let journal = initial
            .prepare_claude(
                "actor",
                &fixture.installation,
                fixture.files.clone(),
                vec![old.id.clone()],
                fixture.context.clone(),
            )
            .unwrap();
        initial.apply(&journal.id).unwrap();
        let restore = initial.prepare_restore(&journal.id).unwrap();
        let expiry = chrono::Utc::now().timestamp() + 1;
        if fault == "expired" {
            let bytes = fixture.vault.get(&old.secret_reference).unwrap();
            let mut secret: InferenceSecret = serde_json::from_slice(&bytes).unwrap();
            secret.expires_at = Some(expiry);
            fixture
                .vault
                .put(&old.secret_reference, &serde_json::to_vec(&secret).unwrap())
                .unwrap();
        }
        let injected = AtomicBool::new(false);
        let vault = HookVault {
            inner: &fixture.vault,
            after_get: Box::new(|reference| {
                if reference == old.secret_reference && !injected.swap(true, Ordering::SeqCst) {
                    match fault {
                        "deleted" => fixture.store.delete("binding", &old.id).unwrap(),
                        "epoch" => fixture
                            .store
                            .set_meta(
                                &crate::identity::authorization_epoch_reference(
                                    &fixture.installation.origin,
                                    &fixture.installation.id,
                                    "actor",
                                ),
                                "2",
                            )
                            .unwrap(),
                        "expired" => {
                            while chrono::Utc::now().timestamp() <= expiry {
                                std::thread::sleep(std::time::Duration::from_millis(25));
                            }
                        }
                        _ => unreachable!(),
                    }
                }
            }),
        };
        let engine = ConfigEngine {
            vault: &vault,
            ..fixture.engine(&guard)
        };
        engine.apply(&restore.id).unwrap();
        assert!(injected.load(Ordering::SeqCst));
        fixture.assert_prefix(3, true);
        assert!(
            fixture
                .binding(&old.id)
                .is_none_or(|binding| !binding.enabled)
        );
        assert!(
            !fixture
                .binding(&fixture.context.binding_id)
                .unwrap()
                .enabled
        );
    }
}

#[test]
fn claude_b2b_late_restore_source_delete_phase_or_cipher_change_stops_before_next_write() {
    for fault in ["deleted", "restored", "phase", "cipher"] {
        let fixture = Fixture::new(true);
        let clear = |_: &ClaudeContext, _: bool| Ok(());
        let initial = fixture.engine(&clear);
        let apply = fixture.prepare(&initial);
        initial.apply(&apply.id).unwrap();
        let restore = initial.prepare_restore(&apply.id).unwrap();
        let injected = Cell::new(false);
        let guard = |_: &ClaudeContext, _: bool| {
            if !injected.get() && fixture.saved_count(true) == 1 {
                injected.set(true);
                if fault == "deleted" {
                    fixture.store.delete("journal", &apply.id)?;
                } else {
                    let mut source = fixture.store.get::<Journal>("journal", &apply.id)?.unwrap();
                    match fault {
                        "restored" => source.phase = "restored".into(),
                        "phase" => source.phase = "conflict".into(),
                        "cipher" => source.ciphertext.push('A'),
                        _ => unreachable!(),
                    }
                    fixture.store.put("journal", &apply.id, "actor", &source)?;
                }
            }
            Ok(())
        };
        let engine = fixture.engine(&guard);
        assert_eq!(
            engine.apply(&restore.id).unwrap_err().code,
            "configuration_conflict"
        );
        assert!(injected.get());
        fixture.assert_prefix(1, true);
        assert!(
            fixture
                .binding(&fixture.context.binding_id)
                .unwrap()
                .enabled
        );
    }
}

#[test]
fn claude_b2b_source_deleted_during_its_vault_decrypt_cannot_start_restoration() {
    let fixture = Fixture::new(true);
    let guard = |_: &ClaudeContext, _: bool| Ok(());
    let initial = fixture.engine(&guard);
    let apply = fixture.prepare(&initial);
    initial.apply(&apply.id).unwrap();
    let restore = initial.prepare_restore(&apply.id).unwrap();
    let backup_reads = AtomicUsize::new(0);
    let vault = HookVault {
        inner: &fixture.vault,
        after_get: Box::new(|reference| {
            // First decrypts the restore operation; second decrypts its source.
            if reference == fixture.installation.backup_reference
                && backup_reads.fetch_add(1, Ordering::SeqCst) == 1
            {
                fixture.store.delete("journal", &apply.id).unwrap();
            }
        }),
    };
    let engine = ConfigEngine {
        vault: &vault,
        ..fixture.engine(&guard)
    };
    assert_eq!(
        engine.apply(&restore.id).unwrap_err().code,
        "configuration_conflict"
    );
    assert_eq!(backup_reads.load(Ordering::SeqCst), 2);
    fixture.assert_prefix(3, false);
    assert!(
        fixture
            .binding(&fixture.context.binding_id)
            .unwrap()
            .enabled
    );
}

#[test]
fn claude_b2b_committed_reentry_checks_all_after_again_after_last_guard() {
    let fixture = Fixture::new(true);
    let armed = Cell::new(false);
    let calls = Cell::new(0);
    let guard = |_: &ClaudeContext, _: bool| {
        if armed.get() {
            calls.set(calls.get() + 1);
            // Recovery's initial before/after Vault guards are calls 1 and 2.
            // The committed branch's final context guard is call 3.
            if calls.get() == 3 {
                fs::write(&fixture.files[2].path, b"late-committed-reentry-change").unwrap();
            }
        }
        Ok(())
    };
    let engine = fixture.engine(&guard);
    let apply = fixture.prepare(&engine);
    engine.apply(&apply.id).unwrap();
    armed.set(true);
    assert_eq!(
        engine.recover(&apply.id).unwrap_err().code,
        "configuration_conflict"
    );
    assert!(calls.get() >= 3);
    assert_eq!(
        fs::read(&fixture.files[2].path).unwrap(),
        b"late-committed-reentry-change"
    );
}
