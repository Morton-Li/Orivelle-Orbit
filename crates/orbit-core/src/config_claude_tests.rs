use super::*;
use crate::{claude_config, identity::InferenceSecret, vault::FakeVault};

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
}

#[test]
fn claude_b2a_v3_context_authenticates_exact_group_and_preview_has_no_side_effects() {
    let fixture = Fixture::new(false);
    let guard = |_: &ClaudeContext, _: bool| Ok(());
    let engine = fixture.engine(&guard);
    let journal = fixture.prepare(&engine);
    assert!(ConfigEngine::aad(&journal).starts_with("orbit-config-v3\n"));
    assert_eq!(engine.claude_context(&journal).unwrap(), fixture.context);
    assert!(!fixture.context.configuration_target.config_root.exists());
    assert!(
        !fixture
            .store
            .get::<HelperBinding>("binding", &fixture.context.binding_id)
            .unwrap()
            .unwrap()
            .enabled
    );
    assert_eq!(
        engine.apply(&journal.id).unwrap_err().code,
        "claude_configuration_transaction_unavailable"
    );
    assert!(!fixture.context.configuration_target.config_root.exists());
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
