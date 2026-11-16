//! Opt-in real Panel/MySQL protocol acceptance. The local TLS proxy drops one
//! successful mutation ACK; PKCE, signatures, session/key authority and data are real.
//! Storage is project-only, Vault is memory, both client paths are fixtures.

use super::*;
use crate::api_keys::{
    ApiKeyChanges, ApiKeyMutation, ApiKeyPage, ApiKeyQuery, ApiKeyView, KeyAction, KeyReference,
    KeyScope,
};
use crate::vault::FakeVault as MemoryVault;
use futures_util::FutureExt;
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    panic::AssertUnwindSafe,
    process::{Child, Command, Stdio},
    time::Instant,
};

type AcceptanceResult<T> = std::result::Result<T, &'static str>;
const MAX_INPUT: u64 = 64 * 1024;

#[derive(Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
struct Input {
    port: u16,
    wrong_san_port: u16,
    #[zeroize(skip)]
    ca_file: PathBuf,
    #[zeroize(skip)]
    bad_ca_file: PathBuf,
    #[zeroize(skip)]
    node: PathBuf,
    #[zeroize(skip)]
    web_driver: PathBuf,
    email: String,
    password: String,
    expected_account_id: String,
    expected_model_ids: Vec<String>,
    denied_model_ids: Vec<String>,
    expected_node_id: String,
}

fn checked(condition: bool, code: &'static str) -> AcceptanceResult<()> {
    if condition { Ok(()) } else { Err(code) }
}
fn identity(metadata: &fs::Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}
fn read_project_file(
    project: &Path,
    path: &Path,
    private: bool,
) -> AcceptanceResult<Zeroizing<Vec<u8>>> {
    let canonical = path.canonicalize().map_err(|_| "input_file_unavailable")?;
    checked(
        path.is_absolute() && canonical == path && canonical.starts_with(project.join(".cache")),
        "input_file_outside_project_cache",
    )?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "input_file_unavailable")?;
    let before = file.metadata().map_err(|_| "input_metadata_unavailable")?;
    checked(
        before.is_file()
            && before.uid() == unsafe { libc::geteuid() }
            && before.nlink() == 1
            && before.len() <= MAX_INPUT
            && (!private || before.permissions().mode() & 0o777 == 0o600),
        "input_file_scope_unverified",
    )?;
    let mut bytes = Zeroizing::new(Vec::new());
    (&mut file)
        .take(MAX_INPUT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "input_read_failed")?;
    let after = file.metadata().map_err(|_| "input_metadata_unavailable")?;
    let current = fs::symlink_metadata(path).map_err(|_| "input_metadata_unavailable")?;
    checked(
        bytes.len() as u64 <= MAX_INPUT
            && current.is_file()
            && identity(&before) == identity(&after)
            && identity(&before) == identity(&current),
        "input_changed_during_read",
    )?;
    Ok(bytes)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DriverInput<'a> {
    ca_pem: &'a str,
    port: u16,
    authorization_url: &'a str,
    expected_callback_uri: &'a str,
    email: &'a str,
    password: &'a str,
}
#[derive(Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DriverResult {
    ok: bool,
    code: String,
    login_csrf_rejected: bool,
    consent_csrf_rejected: bool,
    consent_approved: bool,
    callback_delivered: bool,
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.0.try_wait().is_ok_and(|status| status.is_some()) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
fn run_driver(node: PathBuf, script: PathBuf, payload: Zeroizing<Vec<u8>>) -> AcceptanceResult<()> {
    let mut child = OwnedChild(
        Command::new(node)
            .arg(script)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "web_driver_spawn_failed")?,
    );
    let mut stdin = child.0.stdin.take().ok_or("web_driver_stdin_unavailable")?;
    let stdout = child
        .0
        .stdout
        .take()
        .ok_or("web_driver_stdout_unavailable")?;
    let writer = std::thread::spawn(move || {
        stdin
            .write_all(&payload)
            .map_err(|_| "web_driver_input_failed")
    });
    let reader = std::thread::spawn(move || {
        let mut bytes = Zeroizing::new(Vec::new());
        stdout
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| "web_driver_output_failed")?;
        Ok::<_, &'static str>(bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(90);
    let status = loop {
        if let Some(status) = child.0.try_wait().map_err(|_| "web_driver_wait_failed")? {
            break status;
        }
        if Instant::now() >= deadline {
            return Err("web_driver_timeout");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    writer.join().map_err(|_| "web_driver_writer_panicked")??;
    let bytes = reader.join().map_err(|_| "web_driver_reader_panicked")??;
    checked(bytes.len() <= 4096, "web_driver_output_invalid")?;
    let result: DriverResult =
        serde_json::from_slice(&bytes).map_err(|_| "web_driver_output_invalid")?;
    if !status.success() || !result.ok {
        return Err(match result.code.as_str() {
            "invalid_input" => "web_driver_input_rejected",
            "panel_transport_failed" => "web_driver_transport_failed",
            "panel_response_invalid" => "web_driver_response_invalid",
            "login_page_unavailable" => "web_driver_login_page_unavailable",
            "login_csrf_not_rejected" => "web_driver_login_csrf_not_rejected",
            "login_failed" => "web_driver_login_failed",
            "consent_unavailable" => "web_driver_consent_unavailable",
            "consent_csrf_not_rejected" => "web_driver_consent_csrf_not_rejected",
            "consent_failed" => "web_driver_consent_failed",
            "callback_invalid" => "web_driver_callback_invalid",
            "callback_failed" => "web_driver_callback_failed",
            _ => "web_driver_failed",
        });
    }
    checked(
        result.ok
            && result.code == "ok"
            && result.login_csrf_rejected
            && result.consent_csrf_rejected
            && result.consent_approved
            && result.callback_delivered,
        "web_driver_acceptance_incomplete",
    )
}

struct Harness {
    input: Input,
    ca: Zeroizing<Vec<u8>>,
    bad_ca: Zeroizing<Vec<u8>>,
    origin: String,
    run_root: PathBuf,
    service: Arc<Service>,
    vault: Arc<MemoryVault>,
    events: tokio::sync::mpsc::UnboundedReceiver<AuthorizationEvent>,
}
impl Harness {
    fn prepare() -> AcceptanceResult<Self> {
        let project = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .map_err(|_| "project_root_unavailable")?;
        let path = std::env::var_os("ORBIT_PANEL_ACCEPTANCE_INPUT")
            .map(PathBuf::from)
            .ok_or("panel_acceptance_input_required")?;
        let bytes = read_project_file(&project, &path, true)?;
        let input: Input =
            serde_json::from_slice(&bytes).map_err(|_| "panel_acceptance_input_invalid")?;
        checked(
            input.port != 0
                && input.wrong_san_port != 0
                && input.port != input.wrong_san_port
                && !input.email.is_empty()
                && input.email.len() <= 255
                && !input.password.is_empty()
                && input.password.len() <= 4096
                && !input.expected_account_id.is_empty()
                && !input.expected_model_ids.is_empty()
                && !input.denied_model_ids.is_empty()
                && !input.expected_node_id.is_empty(),
            "panel_acceptance_input_invalid",
        )?;
        checked(
            input.web_driver == project.join("tests/acceptance/panel/web-driver.mjs")
                && input.web_driver.canonicalize().ok().as_ref() == Some(&input.web_driver)
                && input.node.is_absolute()
                && input.node.file_name().is_some_and(|name| name == "node")
                && input.node.canonicalize().ok().as_ref() == Some(&input.node)
                && fs::metadata(&input.node).is_ok_and(|meta| meta.is_file()),
            "web_driver_program_scope_invalid",
        )?;
        let ca = read_project_file(&project, &input.ca_file, false)?;
        let bad_ca = read_project_file(&project, &input.bad_ca_file, false)?;
        checked(
            ca.as_slice() != bad_ca.as_slice(),
            "negative_ca_fixture_not_distinct",
        )?;
        let run_root = path
            .parent()
            .ok_or("input_parent_unavailable")?
            .join(format!("native-panel-component-{}", uuid()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&run_root)
            .map_err(|_| "project_run_directory_unavailable")?;
        let store = Arc::new(
            Store::open(&run_root.join("store")).map_err(|_| "project_store_unavailable")?,
        );
        let vault = Arc::new(MemoryVault::default());
        let mut paths =
            SystemPaths::isolated(&store.root).map_err(|_| "fixture_paths_unavailable")?;
        // An explicit empty client inventory prevents native app/policy probes.
        // No fake installed target or policy is used to permit configuration.
        paths.fixture_clients = Some(Vec::new());
        checked(
            paths.isolated
                && !paths.writes_authorized
                && paths.codex_config.starts_with(&store.root)
                && paths.claude_library.starts_with(&store.root)
                && paths.fixture_policy.is_none()
                && paths.fixture_codex_runtime.is_none()
                && paths.fixture_configuration_target.is_none(),
            "fixture_path_scope_invalid",
        )?;
        let service =
            Service::new(store, vault.clone(), paths).map_err(|_| "project_service_unavailable")?;
        // Service retains its own Weak reference. get_mut therefore rejects it
        // even with one strong owner; rebuild that weak link after injecting TLS.
        let mut native = Arc::try_unwrap(service).map_err(|_| "project_service_not_exclusive")?;
        native.http = Http::for_local_tls_acceptance(&ca, input.port)
            .map_err(|_| "strict_tls_client_unavailable")?;
        let service = Arc::new_cyclic(|reference| {
            native.self_reference = reference.clone();
            native
        });
        let origin = canonical_origin(&format!("https://orbit-e2e.test:{}", input.port))
            .map_err(|_| "local_origin_invalid")?;
        let (sender, events) = tokio::sync::mpsc::unbounded_channel();
        service
            .observe_authorization(Arc::new(move |event| {
                let _ = sender.send(event);
            }))
            .map_err(|_| "authorization_observer_unavailable")?;
        Ok(Self {
            input,
            ca,
            bad_ca,
            origin,
            run_root,
            service,
            vault,
            events,
        })
    }
    async fn tls_checks(&self) -> AcceptanceResult<()> {
        let response = self
            .service
            .http
            .request(
                Method::GET,
                &format!("{}/up", self.origin),
                vec![],
                vec![],
                64 * 1024,
                Duration::from_secs(10),
            )
            .await
            .map_err(|_| "strict_tls_positive_failed")?;
        checked(response.status == 200, "panel_health_not_ready")?;
        let wrong_ca = Http::for_local_tls_acceptance(&self.bad_ca, self.input.port)
            .map_err(|_| "negative_ca_fixture_invalid")?;
        match wrong_ca
            .request(
                Method::GET,
                &format!("{}/up", self.origin),
                vec![],
                vec![],
                64 * 1024,
                Duration::from_secs(10),
            )
            .await
        {
            Err(error) if error.code == "local_tls_unknown_issuer" => {}
            _ => return Err("wrong_ca_was_not_certificate_rejection"),
        }
        let wrong_san = Http::for_local_tls_acceptance(&self.ca, self.input.wrong_san_port)
            .map_err(|_| "negative_san_client_invalid")?;
        match wrong_san
            .request(
                Method::GET,
                &format!("https://orbit-e2e.test:{}/up", self.input.wrong_san_port),
                vec![],
                vec![],
                64 * 1024,
                Duration::from_secs(10),
            )
            .await
        {
            Err(error) if error.code == "local_tls_name_mismatch" => {}
            _ => return Err("wrong_san_was_not_certificate_rejection"),
        }
        checked(
            self.service
                .http
                .validate_url("https://outside.invalid/up")
                .is_err()
                && self
                    .service
                    .http
                    .validate_url(&format!("https://127.0.0.1:{}/up", self.input.port))
                    .is_err()
                && self
                    .service
                    .http
                    .validate_url(&format!("http://orbit-e2e.test:{}/up", self.input.port))
                    .is_err(),
            "strict_tls_origin_scope_not_enforced",
        )
    }
    async fn login(&mut self) -> AcceptanceResult<ConnectionRecord> {
        let (start, url) = self
            .service
            .start_login(&self.origin)
            .await
            .map_err(|_| "real_authorization_start_failed")?;
        let callback_uri = self
            .service
            .http
            .local_acceptance_callback_uri()
            .map_err(|_| "actual_service_callback_receipt_missing")?;
        let payload = Zeroizing::new(
            serde_json::to_vec(&DriverInput {
                ca_pem: std::str::from_utf8(&self.ca).map_err(|_| "ca_pem_not_utf8")?,
                port: self.input.port,
                authorization_url: &url,
                expected_callback_uri: &callback_uri,
                email: &self.input.email,
                password: &self.input.password,
            })
            .map_err(|_| "web_driver_input_encode_failed")?,
        );
        let node = self.input.node.clone();
        let script = self.input.web_driver.clone();
        tokio::task::spawn_blocking(move || run_driver(node, script, payload))
            .await
            .map_err(|_| "web_driver_worker_panicked")??;
        let event = tokio::time::timeout(Duration::from_secs(10), self.events.recv())
            .await
            .map_err(|_| "real_authorization_completion_timeout")?
            .ok_or("authorization_observer_closed")?;
        checked(
            event.request_id == start.request_id
                && event.state == "connected"
                && event.error.is_none(),
            "real_authorization_not_connected",
        )?;
        let view = self
            .service
            .bootstrap()
            .map_err(|_| "real_account_bootstrap_failed")?;
        checked(
            view.pending_authorization.is_none()
                && view.session.status == "authenticated"
                && view
                    .session
                    .account
                    .as_ref()
                    .is_some_and(|account| account.id == self.input.expected_account_id),
            "real_login_incomplete",
        )?;
        let record = self
            .service
            .record(
                view.active_connection_id
                    .as_deref()
                    .ok_or("real_active_connection_missing")?,
            )
            .map_err(|_| "real_connection_missing")?;
        let session = self
            .service
            .session(&record)
            .map_err(|_| "real_session_missing")?;
        checked(
            [
                "api-keys:read",
                "api-keys:write",
                "api-keys:use",
                "api-keys:reveal",
            ]
            .iter()
            .all(|scope| session.scopes.iter().any(|granted| granted == scope)),
            "real_key_profile_scopes_missing",
        )?;
        Ok(record)
    }
    fn key_changes(name: &str) -> ApiKeyChanges {
        ApiKeyChanges {
            name: name.into(),
            expires_at: None,
            service_tier: "follow_client".into(),
            spending_limit: Some("7.50000000".into()),
            spending_limit_mode: "cumulative".into(),
            spending_window_minutes: None,
        }
    }
    fn key_mutation(
        record: &ConnectionRecord,
        action: KeyAction,
        key: Option<&ApiKeyView>,
        changes: Option<ApiKeyChanges>,
    ) -> ApiKeyMutation {
        ApiKeyMutation {
            connection_id: record.view.id.clone(),
            expected_revision: record.view.revision,
            action,
            api_key_id: key.map(|key| key.id.clone()),
            management_revision: key.map(|key| key.management_revision.clone()),
            changes,
        }
    }
    fn key_reference(record: &ConnectionRecord, key: &ApiKeyView) -> KeyReference {
        KeyReference {
            connection_id: record.view.id.clone(),
            expected_revision: record.view.revision,
            api_key_id: key.id.clone(),
            key_revision: key.key_revision.clone(),
        }
    }
    fn current(&self, id: &str) -> AcceptanceResult<ConnectionRecord> {
        self.service
            .record(id)
            .map_err(|_| "real_connection_missing")
    }
    async fn key_page(&self, id: &str, search: &str) -> AcceptanceResult<ApiKeyPage> {
        let record = self.current(id)?;
        self.service
            .list_api_keys(&ApiKeyQuery {
                connection_id: record.view.id,
                expected_revision: record.view.revision,
                search: search.into(),
                status: None,
                page: 1,
            })
            .await
            .map_err(|_| "real_key_list_failed")
    }
    async fn api_keys(&self, first: &ConnectionRecord) -> AcceptanceResult<ConnectionRecord> {
        checked(
            first.credential_reference.is_none() && first.api_key_id.is_none(),
            "new_key_profile_issued_automatic_credential",
        )?;
        let page = self.key_page(&first.view.id, "").await?;
        checked(
            page.can_write && page.can_reveal && page.legacy_key.is_none() && page.total == 0,
            "real_initial_key_projection_invalid",
        )?;
        let result = self
            .service
            .mutate_api_key(&Self::key_mutation(
                first,
                KeyAction::Create,
                None,
                Some(Self::key_changes("Synthetic Orbit default")),
            ))
            .await
            .map_err(|_| "real_key_create_failed")?;
        let key = result.api_key.ok_or("real_created_key_missing")?;
        checked(
            result.completed && key.selectable,
            "real_created_key_not_selectable",
        )?;
        let current = self.current(&first.view.id)?;
        self.service
            .select_api_key(&Self::key_reference(&current, &key))
            .await
            .map_err(|_| "real_key_selection_delivery_failed")?;
        let selected = self.current(&first.view.id)?;
        let before = self
            .service
            .reveal_api_key(&Self::key_reference(&selected, &key))
            .await
            .map_err(|_| "real_key_reveal_failed")?;
        checked(
            before.api_key_id == key.id && before.plain_text_token.starts_with("orv_sk_"),
            "real_key_reveal_invalid",
        )?;
        let rotated = self
            .service
            .mutate_api_key(&Self::key_mutation(
                &selected,
                KeyAction::Rotate,
                Some(&key),
                None,
            ))
            .await
            .map_err(|_| "real_key_rotation_failed")?
            .api_key
            .ok_or("real_rotated_key_missing")?;
        let current = self.current(&first.view.id)?;
        let after = self
            .service
            .reveal_api_key(&Self::key_reference(&current, &rotated))
            .await
            .map_err(|_| "real_rotated_key_reveal_failed")?;
        let old_reference = selected
            .credential_reference
            .as_deref()
            .ok_or("real_selected_credential_missing")?;
        checked(
            rotated.id == key.id
                && rotated.key_revision != key.key_revision
                && before.plain_text_token != after.plain_text_token
                && current.api_key_id.as_deref() == Some(rotated.id.as_str())
                && current.view.credential_revision.as_deref()
                    == Some(rotated.key_revision.as_str())
                && current.credential_reference.as_deref() != Some(old_reference)
                && self
                    .service
                    .store
                    .meta(&crate::identity::credential_disabled_reference(
                        old_reference,
                    ))
                    .map_err(|_| "real_credential_fence_unavailable")?
                    .as_deref()
                    == Some("1")
                && self.vault.get(old_reference).is_err(),
            "real_rotation_did_not_fence_old_credential",
        )?;
        let updated = self
            .service
            .mutate_api_key(&Self::key_mutation(
                &current,
                KeyAction::Update,
                Some(&rotated),
                Some(Self::key_changes("Synthetic Orbit selected")),
            ))
            .await
            .map_err(|_| "real_key_update_failed")?
            .api_key
            .ok_or("real_updated_key_missing")?;
        let current = self.current(&first.view.id)?;
        let reset = self
            .service
            .mutate_api_key(&Self::key_mutation(
                &current,
                KeyAction::Reset,
                Some(&updated),
                None,
            ))
            .await
            .map_err(|_| "real_key_counter_reset_failed")?
            .api_key
            .ok_or("real_reset_key_missing")?;
        checked(
            reset.key_revision == rotated.key_revision,
            "metadata_change_rotated_inference_secret",
        )?;
        let metadata = serde_json::to_string(&self.key_page(&first.view.id, "").await?)
            .map_err(|_| "real_key_metadata_encode_failed")?;
        checked(
            !metadata.contains(after.plain_text_token.as_str()),
            "real_key_metadata_leaked_secret",
        )?;

        // The task-owned TLS proxy forwards this request once, then drops only
        // its first successful Panel response. Both attempts use real Panel and
        // production Service request persistence/recovery; no transport mock.
        let current = self.current(&first.view.id)?;
        let recovery = Self::key_mutation(
            &current,
            KeyAction::Create,
            None,
            Some(Self::key_changes("Synthetic Orbit recovery")),
        );
        match self.service.mutate_api_key(&recovery).await {
            Err(error) if error.code == "api_key_operation_pending" => {}
            _ => return Err("real_lost_ack_was_not_pending"),
        }
        let committed = self
            .key_page(&first.view.id, "Synthetic Orbit recovery")
            .await?;
        checked(
            committed.total == 1 && committed.items.len() == 1 && committed.pending_operation,
            "real_lost_ack_did_not_commit_exactly_once",
        )?;
        match self.service.mutate_api_key(&recovery).await {
            Err(error) if error.code == "api_key_operation_pending" => {}
            _ => return Err("real_pending_operation_allowed_second_mutation"),
        }
        let current = self.current(&first.view.id)?;
        let recovered = self
            .service
            .recover_api_key_operation(&KeyScope {
                connection_id: current.view.id,
                expected_revision: current.view.revision,
            })
            .await
            .map_err(|_| "real_key_operation_recovery_failed")?;
        let recovered_key = recovered.api_key.ok_or("real_recovered_key_missing")?;
        let replayed = self
            .key_page(&first.view.id, "Synthetic Orbit recovery")
            .await?;
        checked(
            recovered.completed
                && recovered_key.id == committed.items[0].id
                && replayed.total == 1
                && !replayed.pending_operation,
            "real_key_operation_recovery_duplicated_mutation",
        )?;
        let current = self.current(&first.view.id)?;
        let revoked = self
            .service
            .mutate_api_key(&Self::key_mutation(
                &current,
                KeyAction::Revoke,
                Some(&recovered_key),
                None,
            ))
            .await
            .map_err(|_| "real_key_revoke_failed")?
            .api_key
            .ok_or("real_revoked_key_missing")?;
        checked(
            revoked.state == "revoked" && !revoked.selectable,
            "real_revoked_key_remained_selectable",
        )?;
        let current = self.current(&first.view.id)?;
        let deleted = self
            .service
            .mutate_api_key(&Self::key_mutation(
                &current,
                KeyAction::Delete,
                Some(&revoked),
                None,
            ))
            .await
            .map_err(|_| "real_key_delete_failed")?;
        checked(
            deleted.completed
                && deleted.api_key.is_none()
                && self
                    .key_page(&first.view.id, "Synthetic Orbit recovery")
                    .await?
                    .total
                    == 0,
            "real_deleted_key_still_listed",
        )?;
        let page = self.key_page(&first.view.id, "").await?;
        checked(
            page.selected_ready && page.selected.is_some_and(|key| key.id == reset.id),
            "real_default_key_not_ready_after_management",
        )?;
        self.current(&first.view.id)
    }
    async fn models_and_offline_gate(&self, record: &ConnectionRecord) -> AcceptanceResult<()> {
        let cache = self
            .service
            .account_cache(record)
            .map_err(|_| "real_account_cache_missing")?;
        checked(
            cache.inference_allowed
                && cache
                    .nodes
                    .iter()
                    .any(|node| node.id == self.input.expected_node_id && !node.available)
                && cache.nodes.iter().all(|node| !node.available),
            "seed_node_not_honestly_offline",
        )?;
        let mut expected = self.input.expected_model_ids.clone();
        expected.sort();
        let mut cached = cache
            .models
            .iter()
            .map(|model| model.id.clone())
            .collect::<Vec<_>>();
        cached.sort();
        checked(
            cached == expected
                && self
                    .input
                    .denied_model_ids
                    .iter()
                    .all(|denied| !cached.contains(denied)),
            "bootstrap_model_permissions_incorrect",
        )?;
        let installation = self
            .service
            .installation(record)
            .map_err(|_| "real_installation_missing")?;
        let session = self
            .service
            .session(record)
            .map_err(|_| "real_session_missing")?;
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("node_id", &self.input.expected_node_id)
            .finish();
        // Reading the permitted catalog does not promote this offline node into
        // an available connection. Do not patch a fake node into the record.
        let envelope = self
            .service
            .http
            .panel_envelope::<Vec<Value>>(
                &self.origin,
                &format!("/api/v1/orbit/models?{query}"),
                Method::GET,
                vec![],
                Some((&installation, &session, self.vault.as_ref())),
                true,
            )
            .await
            .map_err(|_| "real_models_endpoint_failed")?;
        let revision =
            text(&envelope.meta, "revision").map_err(|_| "real_models_revision_missing")?;
        let models = envelope
            .data
            .iter()
            .map(|model| map_model(model, &revision))
            .collect::<Result<Vec<_>>>()
            .map_err(|_| "real_models_contract_invalid")?;
        let mut ids = models
            .iter()
            .map(|model| model.id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        checked(
            ids == expected
                && self
                    .input
                    .denied_model_ids
                    .iter()
                    .all(|denied| !ids.contains(denied)),
            "models_endpoint_permissions_incorrect",
        )?;
        checked(models.iter().all(|model| matches!(&model.codex_catalog_binding,
            Some(CodexCatalogBindingView::Declared { verified: false, profile_id, cli_version, source_sha256, .. })
                if profile_id == crate::codex_binding::PROFILE_ID && cli_version == crate::codex_catalog::SUPPORTED_CLI_VERSION && source_sha256 == crate::codex_catalog::SOURCE_SHA256)), "real_declared_catalog_binding_missing")?;
        match self.service.select_node(
            &record.view.id,
            &self.input.expected_node_id,
            record.view.revision,
        ) {
            Err(error) if error.code == "node_unavailable" => {}
            _ => return Err("offline_node_was_selectable"),
        }
        match self.service.list_models(&record.view.id).await {
            Err(error) if error.code == "node_unavailable" => {}
            _ => return Err("offline_connection_was_promoted"),
        }
        match self.service.apply_client_config(&uuid(), true, false) {
            Err(error) if error.code == "integration_authorization_required" => {}
            _ => return Err("project_fixture_allowed_client_write"),
        }
        checked(
            self.service
                .record(&record.view.id)
                .map_err(|_| "real_connection_missing")?
                .view
                .node_id
                .is_none()
                && !self.service.paths.codex_config.exists()
                && !self.service.paths.codex_catalog().exists()
                && !self.service.paths.claude_library.exists()
                && self
                    .service
                    .store
                    .list::<HelperBinding>("binding", None)
                    .map_err(|_| "fixture_binding_store_unavailable")?
                    .is_empty()
                && self
                    .service
                    .store
                    .list::<Journal>("journal", None)
                    .map_err(|_| "fixture_journal_store_unavailable")?
                    .is_empty(),
            "offline_gate_had_configuration_side_effects",
        )
    }
    async fn protected_bootstrap(
        &self,
        installation: &Installation,
        session: &ProofSession,
    ) -> Result<Value> {
        self.service
            .http
            .panel(
                &self.origin,
                "/api/v1/orbit/bootstrap",
                Method::GET,
                vec![],
                Some((installation, session, self.vault.as_ref())),
                true,
            )
            .await
    }
    async fn logout_and_prove_revoked(&self, record: &ConnectionRecord) -> AcceptanceResult<()> {
        let installation = self
            .service
            .installation(record)
            .map_err(|_| "real_installation_missing")?;
        let old_session = self
            .service
            .session(record)
            .map_err(|_| "real_session_missing")?;
        let view = self
            .service
            .logout(&record.view.id)
            .await
            .map_err(|_| "real_logout_failed")?;
        checked(
            view.session.status == "logged_out"
                && self.vault.get(&record.session_reference).is_err()
                && record
                    .credential_reference
                    .as_ref()
                    .is_none_or(|reference| self.vault.get(reference).is_err()),
            "local_logout_incomplete",
        )?;
        match self.protected_bootstrap(&installation, &old_session).await {
            Err(error) if error.code == "session_expired" => Ok(()),
            _ => Err("server_session_was_not_revoked"),
        }
    }
    async fn exercise(&mut self) -> AcceptanceResult<()> {
        self.tls_checks().await?;
        let first_login = self.login().await?;
        let first = self.api_keys(&first_login).await?;
        self.models_and_offline_gate(&first).await?;
        let first_installation = self
            .service
            .installation(&first)
            .map_err(|_| "real_installation_missing")?;
        self.logout_and_prove_revoked(&first).await?;
        let second = self.login().await?;
        let second_installation = self
            .service
            .installation(&second)
            .map_err(|_| "real_installation_missing")?;
        checked(
            first_installation.id == second_installation.id
                && first.view.id == second.view.id
                && first.api_key_id == second.api_key_id
                && first.view.credential_revision == second.view.credential_revision
                && second.credential_state == "active"
                && second.credential_reference.is_some()
                && first.credential_reference != second.credential_reference
                && self
                    .service
                    .store
                    .list::<Installation>("installation", None)
                    .map_err(|_| "installation_store_unavailable")?
                    .len()
                    == 1,
            "installation_or_active_key_was_not_reused",
        )?;
        self.logout_and_prove_revoked(&second).await?;
        let report = json!({ "state": "passed", "scope": "real_panel_mysql_protocol", "tlsChainAndHostname": true,
            "wrongCaRejected": true, "wrongSanRejected": true, "originConfined": true,
            "pkceAndProofExchange": true, "loginCsrfRejected": true, "consentCsrfRejected": true,
            "automaticKeyNotIssued": true, "explicitKeyDelivered": true, "keyListCreateUpdateRotateResetRevokeDelete": true,
            "rotationFencedOldCredential": true, "lostMutationAckRecoveredExactlyOnce": true,
            "installationAndKeyReused": true, "modelPermissionsFiltered": true,
            "sessionRevocationVerified": true,
            "node": "offline", "clientConfiguration": "not_permitted", "vault": "memory",
            "edgeInference": "not_exercised", "candidateGuiAuth": "not_exercised", "desktopLoaded": "unconfirmed" });
        crate::config::atomic_write(
            &self.run_root.join("report.json"),
            &serde_json::to_vec_pretty(&report).map_err(|_| "report_encode_failed")?,
        )
        .map_err(|_| "report_write_failed")
    }
    async fn cleanup(&self) {
        if let Ok(view) = self.service.bootstrap() {
            if let Some(pending) = view.pending_authorization {
                let _ = self.service.cancel_login(&pending.request_id);
            }
        }
        if let Ok(Some(record)) = self.service.active_record()
            && record.session_status == "authenticated"
        {
            let _ = self.service.logout(&record.view.id).await;
        }
        tokio::task::yield_now().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires local Panel/MySQL/TLS with a one-shot lost-ACK proxy and private project input; no system Vault or client writes"]
async fn real_local_panel_pkce_key_permissions_and_revocation() {
    // Run only this selected ignored entry with --test-threads=1. A catch alone
    // does not stop the default hook printing an unexpected private payload.
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let worked = AssertUnwindSafe(async {
        let mut harness = Harness::prepare()?;
        let result = AssertUnwindSafe(harness.exercise()).catch_unwind().await;
        harness.cleanup().await;
        match result {
            Ok(result) => result,
            Err(_) => Err("panel_acceptance_panicked"),
        }
    })
    .catch_unwind()
    .await;
    let failure = match worked {
        Ok(result) => result.err(),
        Err(_) => Some("panel_acceptance_or_cleanup_panicked"),
    };
    std::panic::set_hook(previous_hook);
    if let Some(code) = failure {
        panic!("local Panel acceptance failed: {code}");
    }
    println!(
        r#"{{"state":"passed","scope":"real_panel_mysql_protocol","vault":"memory","node":"offline","clientConfig":"fixture_only","candidateGuiAuth":"not_exercised"}}"#
    );
}
