use crate::{
    clipboard::{ClipboardImage, ImageClipboardSink},
    config::{
        CodexContext, CodexModelInput, ConfigEngine, FileChange, Journal, codex_catalog_patch,
        codex_configuration_revision, read_file,
    },
    dto::*,
    error::{PublicError, Result, fail},
    identity::{
        HelperBinding, InferenceSecret, Installation, authorization_epoch_reference,
        bound_helper_token, canonical_origin, sha256, uuid, validate_uuid,
    },
    images::{self, InputRecord, TaskSnapshot},
    network::{CredentialDelivery, Http, ProofSession, TokenResponse},
    store::Store,
    system::{ClaudeConfigurationTarget, CodexConfigurationTarget, SystemPaths, detect_clients},
    vault::Vault,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Digest;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Semaphore,
};
use zeroize::Zeroizing;

#[path = "service_claude.rs"]
mod claude;
#[cfg(all(test, target_os = "macos"))]
#[path = "service_claude_tests.rs"]
mod claude_authority_tests;

#[derive(Clone, Serialize, Deserialize)]
struct ConnectionRecord {
    view: ConnectionView,
    installation_id: String,
    session_reference: String,
    credential_reference: Option<String>,
    session_status: String,
    expires_at: i64,
    #[serde(default)]
    session_expires_at: Option<i64>,
    #[serde(default)]
    refresh_attempt: Option<String>,
    #[serde(default)]
    credential_expires_at: Option<i64>,
    #[serde(default)]
    credential_state: String,
    #[serde(default)]
    api_key_id: Option<String>,
    #[serde(default)]
    authorization_epoch: u64,
    #[serde(default)]
    cleanup_references: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct AccountCache {
    account: AccountView,
    nodes: Vec<NodeView>,
    models: Vec<ModelView>,
    catalog_revision: String,
    inference_allowed: bool,
    #[serde(default)]
    keys: Vec<KeyMetadata>,
    #[serde(default)]
    claude_endpoints: HashMap<String, String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct KeyMetadata {
    provider_id: String,
    api_key_id: Option<String>,
    key_revision: Option<String>,
    state: String,
    expires_at: Option<i64>,
}
#[derive(Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
struct SessionEnvelope {
    origin: String,
    account_id: String,
    installation_id: String,
    #[zeroize(skip)]
    session: ProofSession,
}
struct PreviewEntry {
    view: ConfigPreview,
    journal_id: Option<String>,
    import_candidate: Option<(Vec<u8>, String)>,
    policy: crate::policy::Snapshot,
    configuration_target: Option<CodexConfigurationTarget>,
    claude_configuration_target: Option<ClaudeConfigurationTarget>,
    claude_diagnostic_scope: Option<String>,
    claude_context: Option<crate::config::ClaudeContext>,
    catalog_context: Option<CodexContext>,
    _mapping_reservation: Option<crate::claude_mapping::ReservationTicket>,
}
struct ConfigObservation {
    target: ClientView,
    policy: crate::policy::Snapshot,
    configuration_target: Option<CodexConfigurationTarget>,
    claude_configuration_target: Option<ClaudeConfigurationTarget>,
    claude_configuration_error: Option<PublicError>,
    runtime_view: Option<CodexRuntimeView>,
}
fn configuration_access_revision(target_revision: &str, local_policy_revision: &str) -> String {
    sha256(
        format!("codex-configuration-access-v1\n{target_revision}\n{local_policy_revision}\n")
            .as_bytes(),
    )
}
fn configuration_target_view(
    observation: &ConfigObservation,
) -> Option<CodexConfigurationTargetView> {
    let receipt = observation.configuration_target.as_ref()?;
    observation.policy.require_user_config_clear().ok()?;
    Some(CodexConfigurationTargetView {
        cli_version: receipt.cli_version.clone(),
        revision: configuration_access_revision(
            &receipt.revision,
            &observation.policy.local_revision,
        ),
    })
}
struct ForgetEntry {
    view: ForgetInstallationPreview,
    installation_id: String,
    account_ids: Vec<String>,
    backup_ids: Vec<String>,
    expires_at: i64,
}
type Observer = Arc<dyn Fn(BootstrapView) + Send + Sync>;
type AuthorizationObserver = Arc<dyn Fn(AuthorizationEvent) + Send + Sync>;
struct PendingAuthorization {
    view: LoginStart,
    epoch: u64,
    cancel: Arc<tokio::sync::Notify>,
    committed_connection: Option<String>,
}
pub struct MutationLease {
    count: Arc<AtomicU64>,
}
struct ImageExportScope {
    connection_id: String,
    connection_revision: u64,
    panel_origin: String,
    account_id: String,
    installation_id: String,
    authorization_epoch: u64,
}
/// Single-use native operation frozen before a background worker is queued.
/// These private fields cannot be constructed from IPC or serialized.
pub struct ImageExportPlan {
    service: Arc<Service>,
    scope: ImageExportScope,
    image: HistoryImage,
    source: PathBuf,
    expected_hash: String,
    _operation: MutationLease,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
pub struct DecodedImageCopy {
    plan: ImageExportPlan,
    pixels: ClipboardImage,
}
pub struct ReadImageExport {
    plan: ImageExportPlan,
    bytes: Vec<u8>,
}
impl ImageExportPlan {
    pub fn image(&self) -> &HistoryImage {
        &self.image
    }
    pub fn read(self) -> Result<ReadImageExport> {
        let bytes = images::bounded_read(&self.source, images::MAX_IMAGE_BYTES)?;
        if sha256(&bytes) != self.expected_hash {
            return Err(fail("local_image_corrupt"));
        }
        let validated = images::validate_image(&bytes)?;
        if validated.mime != self.image.mime
            || validated.width != self.image.width
            || validated.height != self.image.height
            || bytes.len() as u64 != self.image.bytes
        {
            return Err(fail("invalid_image"));
        }
        Ok(ReadImageExport { plan: self, bytes })
    }
    pub fn decode(self) -> Result<DecodedImageCopy> {
        let ReadImageExport { plan, bytes } = self.read()?;
        let pixels = ClipboardImage::decode(&bytes, (plan.image.width, plan.image.height))?;
        Ok(DecodedImageCopy { plan, pixels })
    }
}
impl DecodedImageCopy {
    pub fn commit(self, sink: &mut dyn ImageClipboardSink) -> Result<()> {
        let Self { plan, pixels } = self;
        plan.service
            .commit_image_export(&plan, || sink.set_image(pixels))
    }
}
impl ReadImageExport {
    pub fn save(self, target: &Path) -> Result<()> {
        let Self { plan, bytes } = self;
        plan.service.commit_image_export(&plan, || {
            if !target.is_absolute() || plan.source == target {
                return Err(fail("invalid_path"));
            }
            if plan.service.paths.isolated && !target.starts_with(&plan.service.store.root) {
                return Err(fail("integration_authorization_required"));
            }
            // No plaintext target temporary file is created before the scope check.
            // Retain the account gate through creation, writing, rename and sync.
            crate::config::atomic_write(target, &bytes)
        })
    }
}
impl Drop for MutationLease {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::SeqCst);
    }
}
pub struct MaintenanceLease<'a> {
    service: &'a Service,
}
impl Drop for MaintenanceLease<'_> {
    fn drop(&mut self) {
        if let Ok(_gate) = self.service.account_gate.lock() {
            // A durable pending update owns the database baseline. Do not write
            // maintenance=0 after prepared or reopen ordinary operations on error.
            if crate::update_recovery::ensure_ready(&self.service.store.root).is_err() {
                return;
            }
            // A storage failure keeps the application closed to mutations until restart.
            if self
                .service
                .store
                .set_meta("maintenance_active", "0")
                .is_ok()
            {
                self.service
                    .maintenance_active
                    .store(false, Ordering::SeqCst);
                self.service.changed();
            }
        }
    }
}
pub struct Service {
    pub store: Arc<Store>,
    pub vault: Arc<dyn Vault>,
    pub http: Http,
    pub paths: SystemPaths,
    self_reference: std::sync::Weak<Self>,
    claude_mappings: crate::claude_mapping::MappingManager,
    state_revision: AtomicU64,
    login_epoch: AtomicU64,
    maintenance_active: AtomicBool,
    mutation_count: Arc<AtomicU64>,
    account_gate: Mutex<()>,
    clients: Mutex<Vec<ClientView>>,
    previews: Mutex<HashMap<String, PreviewEntry>>,
    forgets: Mutex<HashMap<String, ForgetEntry>>,
    diagnostics: Mutex<HashMap<String, DiagnosticsPreview>>,
    observer: Mutex<Option<Observer>>,
    authorization_observer: Mutex<Option<AuthorizationObserver>>,
    pending_authorization: Mutex<Option<PendingAuthorization>>,
    cancelled: Mutex<HashSet<String>>,
    image_workers: Arc<Semaphore>,
    image_memory_workers: Arc<Semaphore>,
    session_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    network_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    received_memory: Mutex<HashMap<String, Vec<images::ReceivedImage>>>,
}
impl Service {
    pub(crate) fn validate_upgrade_store(store: &Store) -> Result<()> {
        let _ = store.snapshot()?;
        let _ = store.settings()?;
        let _ = store.list::<LoginStart>("authorization-pending", None)?;
        let _ = store.list::<AuthorizationEvent>("authorization-terminal", None)?;
        let _ = store.list::<ConnectionRecord>("connection", None)?;
        let _ = store.list::<AccountCache>("account-cache", None)?;
        let _ = store.list::<Installation>("installation", None)?;
        let _ = store.list::<HelperBinding>("binding", None)?;
        let _ = store.list::<ClientView>("client-state", None)?;
        let _ = store.list::<TaskSnapshot>("task-snapshot", None)?;
        let _ = store.list::<TaskView>("task", None)?;
        let _ = store.list::<HistoryImage>("image", None)?;
        let _ = store.list::<InputRecord>("input", None)?;
        let _ = store.list::<Journal>("journal", None)?;
        let _ = store.list::<Journal>("backup", None)?;
        Ok(())
    }
    pub fn new(store: Arc<Store>, vault: Arc<dyn Vault>, paths: SystemPaths) -> Result<Arc<Self>> {
        crate::update_recovery::ensure_ready(&store.root)?;
        let mut clients = detect_clients(&paths);
        for client in &mut clients {
            if let Some(old) = store.get::<ClientView>("client-state", client.client.as_str())? {
                client.configuration_state = old.configuration_state;
                client.configuration_revision = old.configuration_revision;
                client.loaded_state = if client.configuration_state == "saved" {
                    "restart_required"
                } else {
                    "unconfirmed"
                }
                .into();
                if client.client == ClientKind::Claude
                    && old
                        .manual_steps
                        .iter()
                        .any(|note| note == claude::PREFERENCES_RESTORED_NOTE)
                {
                    client
                        .manual_steps
                        .push(claude::PREFERENCES_RESTORED_NOTE.into());
                }
            }
        }
        for mut record in store.list::<ConnectionRecord>("connection", None)? {
            if record.session_status == "refreshing"
                || record.refresh_attempt.is_some() && record.session_status == "authenticated"
            {
                record.session_status = "expired".into();
                store.put(
                    "connection",
                    &record.view.id,
                    &record.view.account_id,
                    &record,
                )?;
            }
        }
        store.set_meta("preview_runtime_nonce", &uuid())?;
        store.delete("authorization-pending", "local")?;
        store.set_meta("maintenance_active", "0")?;
        for client in &clients {
            store.put("client-state", client.client.as_str(), "local", client)?;
        }
        let http = Http::new()?;
        #[cfg(test)]
        let http = if paths.isolated {
            Http::fixture()
        } else {
            http
        };
        let service = Arc::new_cyclic(|self_reference| Self {
            store,
            vault,
            http,
            paths,
            self_reference: self_reference.clone(),
            claude_mappings: Default::default(),
            state_revision: AtomicU64::new(1),
            login_epoch: AtomicU64::new(0),
            maintenance_active: AtomicBool::new(false),
            mutation_count: Arc::new(AtomicU64::new(0)),
            account_gate: Mutex::new(()),
            clients: Mutex::new(clients),
            previews: Mutex::new(HashMap::new()),
            forgets: Mutex::new(HashMap::new()),
            diagnostics: Mutex::new(HashMap::new()),
            observer: Mutex::new(None),
            authorization_observer: Mutex::new(None),
            pending_authorization: Mutex::new(None),
            cancelled: Mutex::new(HashSet::new()),
            image_workers: Arc::new(Semaphore::new(2)),
            image_memory_workers: Arc::new(Semaphore::new(2)),
            session_locks: Mutex::new(HashMap::new()),
            network_locks: Mutex::new(HashMap::new()),
            received_memory: Mutex::new(HashMap::new()),
        });
        service.coordinate_configuration_recovery()?;
        Ok(service)
    }
    pub fn observe(&self, observer: Observer) -> Result<()> {
        *self
            .observer
            .lock()
            .map_err(|_| fail("state_unavailable"))? = Some(observer);
        Ok(())
    }
    pub fn ensure_mutations_available(&self) -> Result<()> {
        crate::update_recovery::ensure_ready(&self.store.root)?;
        if self.maintenance_active.load(Ordering::SeqCst) {
            return Err(fail("update_busy"));
        }
        Ok(())
    }
    pub fn begin_mutation(&self) -> Result<MutationLease> {
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        self.ensure_mutations_available()?;
        self.mutation_count
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_add(1))
            .map_err(|_| fail("update_busy"))?;
        Ok(MutationLease {
            count: self.mutation_count.clone(),
        })
    }
    pub fn begin_maintenance(&self) -> Result<MaintenanceLease<'_>> {
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        self.ensure_mutations_available()?;
        crate::update_recovery::ensure_configuration_idle(&self.store)?;
        if self.active_tasks()?
            || self.mutation_count.load(Ordering::SeqCst) > 0
            || self
                .pending_authorization
                .lock()
                .map_err(|_| fail("state_unavailable"))?
                .is_some()
        {
            return Err(fail("update_busy"));
        }
        self.store.set_meta("maintenance_active", "1")?;
        self.maintenance_active.store(true, Ordering::SeqCst);
        self.changed();
        Ok(MaintenanceLease { service: self })
    }
    pub fn observe_authorization(&self, observer: AuthorizationObserver) -> Result<()> {
        *self
            .authorization_observer
            .lock()
            .map_err(|_| fail("state_unavailable"))? = Some(observer);
        Ok(())
    }
    fn emit_authorization(&self, event: AuthorizationEvent) {
        let observer = self
            .authorization_observer
            .lock()
            .ok()
            .and_then(|o| o.clone());
        if let Some(observer) = observer {
            observer(event);
        }
    }
    fn abort_pending_locked(&self) -> Result<()> {
        self.login_epoch.fetch_add(1, Ordering::SeqCst);
        let pending = self
            .pending_authorization
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .take();
        if let Some(pending) = pending {
            pending.cancel.notify_one();
            self.store.delete("authorization-pending", "local")?;
            let event = AuthorizationEvent {
                request_id: pending.view.request_id,
                state: if pending.committed_connection.is_some() {
                    "connected"
                } else {
                    "cancelled"
                }
                .into(),
                error: None,
            };
            self.store
                .put("authorization-terminal", "local", "local", &event)?;
            self.changed();
            self.emit_authorization(event);
        }
        Ok(())
    }
    fn finish_authorization(&self, id: &str, result: Result<()>) -> Result<()> {
        self.complete_authorization(id, result, false)
    }
    fn complete_authorization(
        &self,
        id: &str,
        result: Result<()>,
        signal_cancel: bool,
    ) -> Result<()> {
        let event = {
            let _gate = self
                .account_gate
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            let mut slot = self
                .pending_authorization
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            if slot.as_ref().is_none_or(|p| p.view.request_id != id) {
                return Ok(());
            }
            let pending = slot
                .take()
                .ok_or_else(|| fail("authorization_unavailable"))?;
            if pending.committed_connection.is_none() && (signal_cancel || result.is_err()) {
                self.login_epoch.fetch_add(1, Ordering::SeqCst);
                pending.cancel.notify_one();
            }
            let (state, error) = if pending.committed_connection.is_some() {
                ("connected", None)
            } else {
                match result {
                    Ok(()) => ("connected", None),
                    Err(error) => (
                        match error.code.as_str() {
                            "authorization_denied" => "denied",
                            "authorization_expired" => "expired",
                            "authorization_cancelled" => "cancelled",
                            _ => "failed",
                        },
                        Some(error),
                    ),
                }
            };
            self.store.delete("authorization-pending", "local")?;
            let event = AuthorizationEvent {
                request_id: id.into(),
                state: state.into(),
                error,
            };
            self.store
                .put("authorization-terminal", "local", "local", &event)?;
            event
        };
        self.changed();
        self.emit_authorization(event);
        Ok(())
    }
    /// This local cancellation never revokes or clears an existing active account.
    pub fn cancel_login(&self, id: &str) -> Result<BootstrapView> {
        let _operation = self.begin_mutation()?;
        let event = {
            let _gate = self
                .account_gate
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            let mut slot = self
                .pending_authorization
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            if slot.as_ref().is_none_or(|p| p.view.request_id != id) {
                let terminal = self
                    .store
                    .get::<AuthorizationEvent>("authorization-terminal", "local")?;
                if terminal.is_some_and(|e| e.request_id == id) {
                    return self.bootstrap();
                }
                return Err(fail("authorization_unavailable"));
            }
            let pending = slot
                .take()
                .ok_or_else(|| fail("authorization_unavailable"))?;
            let state = if pending.committed_connection.is_some() {
                "connected"
            } else {
                self.login_epoch.fetch_add(1, Ordering::SeqCst);
                pending.cancel.notify_one();
                "cancelled"
            };
            self.store.delete("authorization-pending", "local")?;
            let event = AuthorizationEvent {
                request_id: id.into(),
                state: state.into(),
                error: None,
            };
            self.store
                .put("authorization-terminal", "local", "local", &event)?;
            event
        };
        self.changed();
        self.emit_authorization(event);
        self.bootstrap()
    }
    pub fn fail_login(&self, id: &str, error: PublicError) -> Result<()> {
        let _operation = self.begin_mutation()?;
        self.complete_authorization(id, Err(error), true)
    }
    fn changed(&self) {
        self.state_revision.fetch_add(1, Ordering::SeqCst);
        let observer = self.observer.lock().ok().and_then(|o| o.clone());
        if let (Some(observer), Ok(snapshot)) = (observer, self.bootstrap()) {
            observer(snapshot);
        }
    }
    fn record(&self, id: &str) -> Result<ConnectionRecord> {
        validate_uuid(id)?;
        self.store
            .get("connection", id)?
            .ok_or_else(|| fail("connection_unavailable"))
    }
    fn installation(&self, record: &ConnectionRecord) -> Result<Installation> {
        let install = self
            .store
            .installation(&record.view.panel_origin)?
            .ok_or_else(|| fail("installation_identity_mismatch"))?;
        if install.id != record.installation_id {
            return Err(fail("installation_identity_mismatch"));
        }
        Ok(install)
    }
    fn ensure_installation_available(&self, id: &str) -> Result<()> {
        if self
            .store
            .meta(&format!("installation_forgetting/{id}"))?
            .as_deref()
            == Some("1")
        {
            return Err(fail("installation_forgetting"));
        }
        Ok(())
    }
    fn active_record(&self) -> Result<Option<ConnectionRecord>> {
        self.store
            .meta("active_connection")?
            .filter(|id| !id.is_empty())
            .map(|id| self.record(&id))
            .transpose()
    }
    fn account_cache(&self, record: &ConnectionRecord) -> Result<AccountCache> {
        self.store
            .get("account-cache", &record.view.id)?
            .ok_or_else(|| fail("account_unavailable"))
    }
    fn network_lock(&self, id: &str) -> Result<Arc<tokio::sync::Mutex<()>>> {
        Ok(self
            .network_locks
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .entry(id.into())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone())
    }
    fn scoped_task(&self, id: &str) -> Result<TaskSnapshot> {
        validate_uuid(id)?;
        let snap = self
            .store
            .get::<TaskSnapshot>("task-snapshot", id)?
            .ok_or_else(|| fail("task_unavailable"))?;
        let active = self
            .active_record()?
            .ok_or_else(|| fail("session_expired"))?;
        if active.session_status != "authenticated"
            || active
                .session_expires_at
                .is_none_or(|at| at <= chrono::Utc::now().timestamp())
        {
            return Err(fail("session_expired"));
        }
        if snap.panel_origin != active.view.panel_origin
            || snap.task.account_id != active.view.account_id
        {
            return Err(fail("account_scope_mismatch"));
        }
        Ok(snap)
    }
    fn scoped_image(&self, id: &str) -> Result<(HistoryImage, TaskSnapshot)> {
        validate_uuid(id)?;
        let image = self
            .store
            .get::<HistoryImage>("image", id)?
            .ok_or_else(|| fail("local_image_missing"))?;
        let snap = self.scoped_task(&image.task_id)?;
        if image.account_id != snap.task.account_id {
            return Err(fail("account_scope_mismatch"));
        }
        Ok((image, snap))
    }
    fn persist_task(&self, snap: &TaskSnapshot) -> Result<()> {
        self.ensure_mutations_available()?;
        self.store.put_task(snap)?;
        self.changed();
        Ok(())
    }
    pub fn image_memory_permit(&self) -> Result<tokio::sync::OwnedSemaphorePermit> {
        self.image_memory_workers
            .clone()
            .try_acquire_owned()
            .map_err(|_| fail("image_busy"))
    }
    fn session(&self, record: &ConnectionRecord) -> Result<ProofSession> {
        self.http.require_panel_origin(&record.view.panel_origin)?;
        let bytes = self.vault.get(&record.session_reference)?;
        let envelope: SessionEnvelope =
            serde_json::from_slice(&bytes).map_err(|_| fail("credential_missing"))?;
        if envelope.origin != record.view.panel_origin
            || envelope.account_id != record.view.account_id
            || envelope.installation_id != record.installation_id
        {
            return Err(fail("account_scope_mismatch"));
        }
        let serialized = Zeroizing::new(
            serde_json::to_vec(&envelope.session).map_err(|_| fail("serialization_failed"))?,
        );
        serde_json::from_slice(&serialized).map_err(|_| fail("credential_missing"))
    }
    fn save_session(&self, record: &ConnectionRecord, session: ProofSession) -> Result<()> {
        let bytes = Zeroizing::new(
            serde_json::to_vec(&SessionEnvelope {
                origin: record.view.panel_origin.clone(),
                account_id: record.view.account_id.clone(),
                installation_id: record.installation_id.clone(),
                session,
            })
            .map_err(|_| fail("serialization_failed"))?,
        );
        self.vault.put(&record.session_reference, &bytes)
    }
    fn invalidate_evidence(&self) -> Result<()> {
        for client in self
            .clients
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .iter_mut()
        {
            client.loaded_evidence = None;
            client.connection_evidence = None;
            client.loaded_state = if client.configuration_state == "saved" {
                "restart_required"
            } else {
                "unconfirmed"
            }
            .into();
            client.connection_state = "not_tested".into();
            self.store
                .put("client-state", client.client.as_str(), "local", client)?;
        }
        Ok(())
    }
    pub fn bootstrap(&self) -> Result<BootstrapView> {
        // All actor data and its revision are read from the same SQLite transaction.
        let snapshot = self.store.snapshot()?;
        let records = snapshot.list::<ConnectionRecord>("connection", None)?;
        let active = snapshot
            .meta
            .get("active_connection")
            .filter(|id| !id.is_empty())
            .and_then(|id| records.iter().find(|r| &r.view.id == id))
            .cloned();
        let mut session = SessionView::default();
        let mut nodes = vec![];
        let mut tasks = vec![];
        let mut history = vec![];
        let mut backups = vec![];
        if let Some(record) = &active {
            session.status = record.session_status.clone();
            if matches!(session.status.as_str(), "authenticated" | "refreshing")
                && record
                    .session_expires_at
                    .is_some_and(|at| at <= chrono::Utc::now().timestamp())
            {
                session.status = "expired".into();
            }
            session.panel_origin = Some(record.view.panel_origin.clone());
            if let Some(cache) = snapshot.get::<AccountCache>("account-cache", &record.view.id)? {
                if matches!(session.status.as_str(), "authenticated" | "refreshing") {
                    session.account = Some(cache.account);
                    nodes = cache.nodes;
                }
                session.inference_credential_status = if record.view.credential_revision.is_some()
                    && record.credential_reference.is_some()
                    && session.status == "authenticated"
                    && record.credential_state == "active"
                    && record
                        .credential_expires_at
                        .is_none_or(|at| at > chrono::Utc::now().timestamp())
                {
                    "ready"
                } else if record.credential_state == "revoked" {
                    "revoked"
                } else if !cache.inference_allowed {
                    "permission_denied"
                } else {
                    "unavailable"
                }
                .into();
            }
            if matches!(session.status.as_str(), "authenticated" | "refreshing") {
                for snap in
                    snapshot.list::<TaskSnapshot>("task-snapshot", Some(&record.view.account_id))?
                {
                    if snap.panel_origin == record.view.panel_origin {
                        if let Some(task) = snapshot.get("task", &snap.task.id)? {
                            tasks.push(task);
                        }
                    }
                }
                let task_ids: HashSet<String> =
                    tasks.iter().map(|t: &TaskView| t.id.clone()).collect();
                history = snapshot
                    .list::<HistoryImage>("image", Some(&record.view.account_id))?
                    .into_iter()
                    .filter(|i| task_ids.contains(&i.task_id))
                    .map(|mut i| {
                        if !self
                            .store
                            .root
                            .join("previews")
                            .join(format!("{}.png", i.id))
                            .exists()
                        {
                            i.preview_url = None;
                        } else {
                            i.preview_url =
                                snapshot
                                    .meta
                                    .get("preview_runtime_nonce")
                                    .and_then(|nonce| {
                                        images::preview_url_with_token(
                                            &i.id,
                                            &crate::store::preview_scope_token(
                                                nonce,
                                                &record.view.panel_origin,
                                                &record.view.account_id,
                                                &record.installation_id,
                                                &record.view.id,
                                                record.view.revision,
                                            ),
                                        )
                                        .ok()
                                    });
                        }
                        i
                    })
                    .collect();
            }
        } else if self.paths.isolated {
            session.status = "vault_unavailable".into();
        }
        let installation_records = snapshot.list::<Installation>("installation", None)?;
        let mut backup_candidates = snapshot.list::<Journal>("backup", None)?;
        for j in snapshot.list::<Journal>("journal", None)? {
            if ["publishing", "conflict", "committed"].contains(&j.phase.as_str())
                && !backup_candidates.iter().any(|b| b.id == j.id)
            {
                backup_candidates.push(j);
            }
        }
        for j in backup_candidates {
            if installation_records
                .iter()
                .any(|i| i.id == j.installation.id && i.origin == j.installation.origin)
            {
                backups.push(BackupGroup {
                    id: j.id,
                    client: j.client,
                    owner: RestoreOwner {
                        account_id: j.account_id.clone(),
                        account_label: if j.account_label.is_empty() {
                            j.account_id
                        } else {
                            j.account_label
                        },
                        panel_origin: j.installation.origin,
                        installation_id: j.installation.id,
                    },
                    created_at: j.created_at,
                    expires_at: j.expires_at.clone(),
                    file_count: j.paths.len() as u32,
                    state: if ["publishing", "conflict"].contains(&j.phase.as_str()) {
                        "conflict"
                    } else if chrono::DateTime::parse_from_rfc3339(&j.expires_at)
                        .is_ok_and(|at| at > chrono::Utc::now())
                    {
                        "available"
                    } else {
                        "unavailable"
                    }
                    .into(),
                });
            }
        }
        let installations = installation_records
            .into_iter()
            .map(|i| {
                let mut labels = records
                    .iter()
                    .filter(|r| r.installation_id == i.id)
                    .map(|r| r.view.label.clone())
                    .chain(
                        backups
                            .iter()
                            .filter(|b| b.owner.installation_id == i.id)
                            .map(|b| b.owner.account_label.clone()),
                    )
                    .collect::<Vec<_>>();
                labels.sort();
                labels.dedup();
                InstallationView {
                    id: i.id,
                    panel_origin: i.origin,
                    account_labels: labels,
                }
            })
            .collect();
        let mut clients = snapshot.list::<ClientView>("client-state", Some("local"))?;
        for client in &mut clients {
            let (mut observed, policy) = self.observe_client_policy(client.clone());
            // Old client-state rows predate the Claude library writer. Upgrade
            // only that display state after the full producer and local policy
            // qualify; the lightweight access hint is never sufficient.
            if observed.client == ClientKind::Claude
                && observed.configuration_state == "unsupported"
                && observed.configuration_access == ConfigurationAccess::Allowed
                && !self.pending_configuration_recovery(ClientKind::Claude)?
                && self.claude_revision().is_ok()
                && let Ok(observation) = self.observe_configuration(ClientKind::Claude)
                && self
                    .require_claude_configuration(&observation, Some(&policy), None)
                    .is_ok()
            {
                observed.configuration_state = "not_configured".into();
                observed.configuration_access_revision = observation
                    .claude_configuration_target
                    .as_ref()
                    .map(|target| {
                        claude::access_revision(
                            &target.revision,
                            &observation.policy.local_revision,
                        )
                    });
                let instruction = "登录 Panel 并选择节点与模型后，预览三文件变更并确认保存；重启 Claude 后，加载与连通仍需单独核验。";
                if !observed.manual_steps.iter().any(|note| note == instruction) {
                    observed.manual_steps.push(instruction.into());
                }
            }
            *client = observed;
        }
        Ok(BootstrapView {
            schema_version: 1,
            state_revision: snapshot.revision,
            source: "local".into(),
            platform: if cfg!(target_os = "macos") {
                Some("macos".into())
            } else if cfg!(target_os = "windows") {
                Some("windows".into())
            } else {
                None
            },
            pending_authorization: snapshot.get("authorization-pending", "local")?,
            maintenance_active: snapshot
                .meta
                .get("maintenance_active")
                .is_some_and(|v| v == "1"),
            session,
            connections: records.into_iter().map(|r| r.view).collect(),
            installations,
            nodes,
            active_connection_id: active.map(|r| r.view.id),
            clients,
            settings: snapshot.get("settings", "local")?.unwrap_or_default(),
            tasks,
            image_history: history,
            backups,
        })
    }
    pub async fn start_login(self: &Arc<Self>, panel_origin: &str) -> Result<(LoginStart, String)> {
        self.http.require_panel_origin(panel_origin)?;
        let operation = self.begin_mutation()?;
        let origin = canonical_origin(panel_origin)?;
        let epoch = {
            let _gate = self
                .account_gate
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            self.abort_pending_locked()?;
            self.login_epoch.load(Ordering::SeqCst)
        };
        let installation = {
            let _gate = self
                .account_gate
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            let installation = match self.store.installation(&origin)? {
                Some(i) => i,
                None => {
                    let i = Installation::create(&origin, self.vault.as_ref())?;
                    self.store
                        .put("installation", &sha256(origin.as_bytes()), &origin, &i)?;
                    i
                }
            };
            self.ensure_installation_available(&installation.id)?;
            installation
        };
        // Prove vault access before opening a browser or allocating an authorization request.
        let _ = self.vault.get(&installation.signing_reference)?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| fail("callback_unavailable"))?;
        let port = listener
            .local_addr()
            .map_err(|_| fail("callback_unavailable"))?
            .port();
        let redirect_uri = format!("http://127.0.0.1:{port}/orivelle-orbit/callback");
        let mut entropy = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(&mut *entropy);
        let state = URL_SAFE_NO_PAD.encode(*entropy);
        OsRng.fill_bytes(&mut *entropy);
        let verifier = Zeroizing::new(URL_SAFE_NO_PAD.encode(*entropy));
        let challenge = URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()));
        let body=serde_json::to_vec(&json!({"client_id":"orivelle-orbit","installation_id":installation.id,"public_key":installation.public_key,"installation_name":"Orivelle Orbit desktop","redirect_uri":redirect_uri,"state":state,"code_challenge":challenge,"code_challenge_method":"S256"})).map_err(|_|fail("serialization_failed"))?;
        #[derive(Deserialize)]
        struct Start {
            authorization_request_id: String,
            authorization_url: String,
            expires_at: String,
            proof_nonce: String,
        }
        let start: Start = self
            .http
            .panel(
                &origin,
                "/api/v1/orbit/authorization-requests",
                Method::POST,
                body,
                None,
                false,
            )
            .await?;
        let url = self.http.validate_url(&start.authorization_url)?;
        let pairs = url.query_pairs().collect::<Vec<_>>();
        if url.origin().ascii_serialization() != origin
            || url.path() != "/orbit/authorize"
            || pairs.len() != 1
            || pairs[0].0 != "request"
            || pairs[0].1 != start.authorization_request_id
            || start.proof_nonce.is_empty()
        {
            return Err(fail("invalid_authorization_url"));
        }
        let expire = chrono::DateTime::parse_from_rfc3339(&start.expires_at)
            .map_err(|_| fail("invalid_panel_response"))?
            .timestamp();
        if expire <= chrono::Utc::now().timestamp() {
            return Err(fail("authorization_expired"));
        }
        let seconds = (expire - chrono::Utc::now().timestamp()).min(600) as u64;
        let view = LoginStart {
            request_id: start.authorization_request_id.clone(),
            expires_at: start.expires_at,
        };
        let cancel = Arc::new(tokio::sync::Notify::new());
        {
            let _gate = self
                .account_gate
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            if self.login_epoch.load(Ordering::SeqCst) != epoch {
                return Err(fail("authorization_cancelled"));
            }
            self.store
                .put("authorization-pending", "local", "local", &view)?;
            *self
                .pending_authorization
                .lock()
                .map_err(|_| fail("state_unavailable"))? = Some(PendingAuthorization {
                view: view.clone(),
                epoch,
                cancel: cancel.clone(),
                committed_connection: None,
            });
        }
        self.changed();
        let service = self.clone();
        let pending_id = view.request_id.clone();
        tokio::spawn(async move {
            let _operation = operation;
            let result = tokio::select! {
                _=cancel.notified()=>Err(fail("authorization_cancelled")),
                result=tokio::time::timeout(Duration::from_secs(seconds),service.finish_login(listener,installation,
                    start.authorization_request_id,start.proof_nonce,state,verifier,redirect_uri,epoch)) =>
                    result.unwrap_or_else(|_|Err(fail("authorization_expired"))),
            };
            let _ = service.finish_authorization(&pending_id, result);
        });
        Ok((view, start.authorization_url))
    }
    async fn finish_login(
        self: &Arc<Self>,
        listener: TcpListener,
        installation: Installation,
        request_id: String,
        nonce: String,
        state: String,
        verifier: Zeroizing<String>,
        redirect_uri: String,
        epoch: u64,
    ) -> Result<()> {
        let (mut socket, peer) = listener
            .accept()
            .await
            .map_err(|_| fail("callback_unavailable"))?;
        if !peer.ip().is_loopback() {
            return Err(fail("invalid_callback"));
        }
        let mut request = vec![0u8; 8193];
        let count = tokio::time::timeout(Duration::from_secs(5), async {
            let mut count = 0;
            loop {
                if count == request.len() {
                    return Err(fail("invalid_callback"));
                }
                let read = socket
                    .read(&mut request[count..])
                    .await
                    .map_err(|_| fail("invalid_callback"))?;
                if read == 0 {
                    return Err(fail("invalid_callback"));
                }
                count += read;
                if count > 8192 {
                    return Err(fail("invalid_callback"));
                }
                if request[..count].windows(4).any(|w| w == b"\r\n\r\n") {
                    return Ok(count);
                }
            }
        })
        .await
        .map_err(|_| fail("invalid_callback"))??;
        let request_text =
            std::str::from_utf8(&request[..count]).map_err(|_| fail("invalid_callback"))?;
        let line = request_text
            .lines()
            .next()
            .ok_or_else(|| fail("invalid_callback"))?;
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let authority = url::Url::parse(&redirect_uri)
            .map_err(|_| fail("invalid_callback"))?
            .authority()
            .to_string();
        let host = request_text
            .lines()
            .filter_map(|l| l.split_once(':'))
            .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
            .map(|(_, value)| value.trim())
            .collect::<Vec<_>>();
        if fields.len() != 3
            || fields[0] != "GET"
            || fields[2] != "HTTP/1.1"
            || !fields[1].starts_with("/orivelle-orbit/callback?")
            || host != vec![authority.as_str()]
        {
            return Err(fail("invalid_callback"));
        }
        let url = url::Url::parse(&format!("http://127.0.0.1{}", fields[1]))
            .map_err(|_| fail("invalid_callback"))?;
        let mut parameters = HashMap::new();
        for (k, v) in url.query_pairs() {
            if parameters.insert(k.into_owned(), v.into_owned()).is_some() {
                return Err(fail("invalid_callback"));
            }
        }
        if parameters.get("state") != Some(&state)
            || parameters.get("iss") != Some(&installation.origin)
            || parameters.len() != 3
        {
            return Err(fail("invalid_callback"));
        }
        if parameters.get("error").map(String::as_str) == Some("access_denied") {
            return Err(fail("authorization_denied"));
        }
        let code = parameters
            .remove("code")
            .ok_or_else(|| fail("invalid_callback"))?;
        if code.is_empty() || code.len() > 2048 {
            return Err(fail("invalid_callback"));
        }
        let body=Zeroizing::new(serde_json::to_vec(&json!({"grant_type":"authorization_code","authorization_request_id":request_id,"code":code,"code_verifier":&*verifier,"redirect_uri":redirect_uri})).map_err(|_|fail("serialization_failed"))?);
        let provisional = ProofSession {
            binding_id: request_id,
            nonce,
            access_token: String::new(),
            refresh_token: String::new(),
            expires_at: 0,
            session_expires_at: String::new(),
            scopes: vec![],
            credential_delivery_ticket: None,
        };
        if self.login_epoch.load(Ordering::SeqCst) != epoch {
            return Err(fail("authorization_cancelled"));
        }
        let response: TokenResponse = self
            .http
            .panel(
                &installation.origin,
                "/api/v1/orbit/token",
                Method::POST,
                body.to_vec(),
                Some((&installation, &provisional, self.vault.as_ref())),
                false,
            )
            .await?;
        let (account_id, session) = response.into_session(&installation)?;
        let ticket = session.credential_delivery_ticket.clone();
        let mut record =
            self.commit_authorization(&installation, account_id.clone(), session, epoch)?;
        let id = record.view.id.clone();
        if let Some(ticket) = ticket {
            let session = self.session(&record)?;
            let body = Zeroizing::new(
                serde_json::to_vec(&json!({"credential_delivery_ticket":ticket}))
                    .map_err(|_| fail("serialization_failed"))?,
            );
            let delivery = self
                .http
                .panel::<CredentialDelivery>(
                    &installation.origin,
                    "/api/v1/orbit/installations/current/inference-credential",
                    Method::POST,
                    body.to_vec(),
                    Some((&installation, &session, self.vault.as_ref())),
                    true,
                )
                .await;
            if let Ok(delivery) = delivery {
                if self.login_epoch.load(Ordering::SeqCst) == epoch {
                    self.accept_credential(&mut record, &installation, delivery)?;
                }
            } /* Lost single-use delivery is never replayed. Account login remains available. */
        }
        self.refresh_account_cache(&record.view.id, false).await?;
        {
            let _gate = self
                .account_gate
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            if self.login_epoch.load(Ordering::SeqCst) != epoch {
                return Err(fail("authorization_cancelled"));
            }
            let current = self.record(&id)?;
            if current.session_status != "authenticated" || current.view.account_id != account_id {
                return Err(fail("authorization_cancelled"));
            }
            self.store.set_meta("active_connection", &current.view.id)?;
            self.invalidate_evidence()?;
        }
        self.changed();
        let text = "Orbit authorization completed. You may close this window.";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
            text.len()
        );
        let _ = socket.write_all(response.as_bytes()).await;
        Ok(())
    }
    fn commit_authorization(
        &self,
        installation: &Installation,
        account_id: String,
        session: ProofSession,
        epoch: u64,
    ) -> Result<ConnectionRecord> {
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        self.ensure_mutations_available()?;
        if self.login_epoch.load(Ordering::SeqCst) != epoch {
            return Err(fail("authorization_cancelled"));
        }
        self.ensure_installation_available(&installation.id)?;
        if self
            .store
            .installation(&installation.origin)?
            .is_none_or(|i| i.id != installation.id)
        {
            return Err(fail("installation_identity_mismatch"));
        }
        let existing = self
            .store
            .list::<ConnectionRecord>("connection", Some(&account_id))?
            .into_iter()
            .find(|r| r.view.panel_origin == installation.origin);
        let id = existing
            .as_ref()
            .map(|r| r.view.id.clone())
            .unwrap_or_else(uuid);
        let previous_revision = existing.as_ref().map_or(0, |r| r.view.revision);
        let mut record = ConnectionRecord {
            view: ConnectionView {
                id: id.clone(),
                label: account_id.clone(),
                panel_origin: installation.origin.clone(),
                account_id: account_id.clone(),
                node_id: existing.as_ref().and_then(|r| r.view.node_id.clone()),
                node_label: None,
                edge_base_url: None,
                provider_id: "openai".into(),
                revision: previous_revision + 1,
                credential_revision: None,
                permission_state: "unavailable".into(),
            },
            installation_id: installation.id.clone(),
            session_reference: format!(
                "account/{}/{}/session",
                installation.id,
                sha256(account_id.as_bytes())
            ),
            credential_reference: None,
            session_status: "authenticated".into(),
            expires_at: session.expires_at,
            session_expires_at: Some(parse_timestamp(&session.session_expires_at)?),
            refresh_attempt: None,
            credential_expires_at: None,
            credential_state: "unavailable".into(),
            api_key_id: None,
            authorization_epoch: 0,
            cleanup_references: existing
                .as_ref()
                .map(|r| r.cleanup_references.clone())
                .unwrap_or_default(),
        };
        if let Some(old) = existing
            .as_ref()
            .and_then(|r| r.credential_reference.as_ref())
        {
            if !record.cleanup_references.contains(old) {
                record.cleanup_references.push(old.clone());
            }
        }
        if let Some(old) = existing.as_ref().map(|r| &r.session_reference) {
            if old != &record.session_reference && !record.cleanup_references.contains(old) {
                record.cleanup_references.push(old.clone());
            }
        }
        record.authorization_epoch = self.advance_authorization_epoch(&record)?;
        record.session_status = "authorizing".into();
        self.store.put("connection", &id, &account_id, &record)?;
        if let Err(error) = self.save_session(&record, session) {
            record.session_status = "vault_unavailable".into();
            self.store.put("connection", &id, &account_id, &record)?;
            return Err(error);
        }
        record.session_status = "authenticated".into();
        record
            .cleanup_references
            .retain(|r| r != &record.session_reference);
        self.store
            .disable_bindings(&account_id, &installation.origin)?;
        self.stop_claude_mappings_for(&account_id, &installation.origin)?;
        self.store.put("connection", &id, &account_id, &record)?;
        let _ = self.cleanup_pending(&mut record)?;
        if let Some(pending) = self
            .pending_authorization
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .as_mut()
        {
            if pending.epoch == epoch {
                pending.committed_connection = Some(record.view.id.clone());
                self.store.set_meta("active_connection", &record.view.id)?;
                self.invalidate_evidence()?;
            }
        }
        Ok(record)
    }
    fn accept_credential(
        &self,
        record: &mut ConnectionRecord,
        installation: &Installation,
        delivery: CredentialDelivery,
    ) -> Result<()> {
        let _operation = self.begin_mutation()?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let current = self.record(&record.view.id)?;
        self.ensure_installation_available(&record.installation_id)?;
        if current.view.revision != record.view.revision
            || current.session_status != "authenticated"
        {
            return Err(fail("connection_conflict"));
        }
        if delivery.provider_id != "openai"
            || delivery.key_revision.len() != 64
            || !delivery.key_revision.bytes().all(|b| b.is_ascii_hexdigit())
            || !delivery.plain_text_token.starts_with("orv_sk_")
            || delivery.plain_text_token.len() > 4096
            || delivery.plain_text_token.chars().any(char::is_whitespace)
        {
            return Err(fail("invalid_panel_response"));
        }
        let expires_at = delivery
            .expires_at
            .as_deref()
            .map(parse_timestamp)
            .transpose()?;
        if expires_at.is_some_and(|at| at <= chrono::Utc::now().timestamp()) {
            return Err(fail("credential_expired"));
        }
        let reference = format!(
            "inference/{}/{}/{}",
            installation.id,
            sha256(record.view.account_id.as_bytes()),
            delivery.key_revision
        );
        let secret = InferenceSecret {
            origin: installation.origin.clone(),
            account_id: record.view.account_id.clone(),
            installation_id: installation.id.clone(),
            provider_id: delivery.provider_id.clone(),
            credential_revision: delivery.key_revision.clone(),
            token: delivery.plain_text_token,
            expires_at,
            authorization_epoch: record.authorization_epoch,
        };
        let bytes =
            Zeroizing::new(serde_json::to_vec(&secret).map_err(|_| fail("serialization_failed"))?);
        self.vault.put(&reference, &bytes)?;
        record.cleanup_references.retain(|r| r != &reference);
        if let Some(old) = record.credential_reference.take() {
            if old != reference {
                if !record.cleanup_references.contains(&old) {
                    record.cleanup_references.push(old);
                }
            }
        }
        record.credential_reference = Some(reference);
        record.view.credential_revision = Some(delivery.key_revision);
        record.view.provider_id = delivery.provider_id;
        record.view.permission_state = "allowed".into();
        record.api_key_id = Some(delivery.api_key_id);
        record.credential_expires_at = expires_at;
        record.credential_state = "active".into();
        self.store.put(
            "connection",
            &record.view.id,
            &record.view.account_id,
            record,
        )?;
        let _ = self.cleanup_pending(record)?;
        Ok(())
    }
    async fn fresh_session(
        &self,
        id: &str,
    ) -> Result<(ConnectionRecord, Installation, ProofSession)> {
        let _operation = self.begin_mutation()?;
        let lock = {
            let mut locks = self
                .session_locks
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            locks
                .entry(id.into())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;
        let mut record = self.record(id)?;
        self.http.require_panel_origin(&record.view.panel_origin)?;
        self.ensure_installation_available(&record.installation_id)?;
        if record.session_status != "authenticated" {
            return Err(fail("session_expired"));
        }
        let installation = self.installation(&record)?;
        let mut session = self.session(&record)?;
        if parse_timestamp(&session.session_expires_at)? <= chrono::Utc::now().timestamp() {
            let _gate = self
                .account_gate
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            let current = self.record(id)?;
            if current.view.revision == record.view.revision
                && current.session_status == "authenticated"
            {
                record.session_status = "expired".into();
                self.store
                    .put("connection", id, &record.view.account_id, &record)?;
            }
            return Err(fail("session_expired"));
        }
        if session.expires_at <= chrono::Utc::now().timestamp() + 30 {
            let body = Zeroizing::new(
                serde_json::to_vec(
                    &json!({"grant_type":"refresh_token","refresh_token":session.refresh_token}),
                )
                .map_err(|_| fail("serialization_failed"))?,
            );
            // A rotating refresh credential cannot be retried after an ambiguous response
            // or vault failure. Persist the recovery gate before the single network call.
            {
                let _gate = self
                    .account_gate
                    .lock()
                    .map_err(|_| fail("state_unavailable"))?;
                let current = self.record(id)?;
                if current.view.revision != record.view.revision
                    || current.session_status != "authenticated"
                {
                    return Err(fail("connection_conflict"));
                }
                record.session_status = "refreshing".into();
                record.refresh_attempt = Some(uuid());
                self.store
                    .put("connection", id, &record.view.account_id, &record)?;
            }
            self.changed();
            let rotation = async {
                let response: TokenResponse = self
                    .http
                    .panel(
                        &record.view.panel_origin,
                        "/api/v1/orbit/token",
                        Method::POST,
                        body.to_vec(),
                        Some((&installation, &session, self.vault.as_ref())),
                        false,
                    )
                    .await?;
                let (account, new_session) = response.into_session(&installation)?;
                if account != record.view.account_id {
                    return Err(fail("account_scope_mismatch"));
                }
                self.commit_refresh(&mut record, new_session)
            }
            .await;
            if let Err(error) = rotation {
                self.fail_refresh(&record, &error)?;
                self.changed();
                return Err(error);
            }
            session = self.session(&record)?;
            self.changed();
        }
        Ok((record, installation, session))
    }
    fn commit_refresh(
        &self,
        record: &mut ConnectionRecord,
        new_session: ProofSession,
    ) -> Result<()> {
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        self.ensure_mutations_available()?;
        let current = self.record(&record.view.id)?;
        self.ensure_installation_available(&record.installation_id)?;
        if current.view.revision != record.view.revision
            || current.session_status != "refreshing"
            || current.refresh_attempt != record.refresh_attempt
            || current.authorization_epoch != record.authorization_epoch
        {
            return Err(fail("connection_conflict"));
        }
        record.expires_at = new_session.expires_at;
        record.session_expires_at = Some(parse_timestamp(&new_session.session_expires_at)?);
        self.save_session(record, new_session)?;
        record.session_status = "authenticated".into();
        record.refresh_attempt = None;
        self.store.put(
            "connection",
            &record.view.id,
            &record.view.account_id,
            record,
        )
    }
    fn fail_refresh(&self, expected: &ConnectionRecord, error: &PublicError) -> Result<()> {
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        self.ensure_mutations_available()?;
        let mut current = self.record(&expected.view.id)?;
        if current.view.revision == expected.view.revision
            && current.authorization_epoch == expected.authorization_epoch
            && current.session_status == "refreshing"
            && current.refresh_attempt == expected.refresh_attempt
        {
            current.session_status = if error.code == "vault_unavailable" {
                "vault_unavailable"
            } else {
                "expired"
            }
            .into();
            self.store.put(
                "connection",
                &current.view.id,
                &current.view.account_id,
                &current,
            )?;
        }
        Ok(())
    }
    fn advance_authorization_epoch(&self, record: &ConnectionRecord) -> Result<u64> {
        let key = authorization_epoch_reference(
            &record.view.panel_origin,
            &record.installation_id,
            &record.view.account_id,
        );
        let old = self
            .store
            .meta(&key)?
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
            .max(record.authorization_epoch);
        let next = old
            .checked_add(1)
            .ok_or_else(|| fail("local_metadata_invalid"))?;
        self.store.set_meta(&key, &next.to_string())?;
        Ok(next)
    }
    fn apply_known_key_state(
        &self,
        record: &mut ConnectionRecord,
        cache: &AccountCache,
    ) -> Result<()> {
        let key = cache.keys.iter().find(|k| {
            k.provider_id == record.view.provider_id
                && k.api_key_id == record.api_key_id
                && k.key_revision == record.view.credential_revision
        });
        let ready = cache.inference_allowed
            && key.is_some_and(|k| {
                k.state == "active"
                    && k.expires_at
                        .is_none_or(|at| at > chrono::Utc::now().timestamp())
            });
        if !ready && record.credential_reference.is_some() {
            let state = key
                .map(|k| k.state.clone())
                .unwrap_or_else(|| "unavailable".into());
            self.disable_inference_before_cleanup(record, &state)?;
        } else if let Some(key) = key {
            record.credential_state = key.state.clone();
            record.credential_expires_at = key.expires_at;
            for mut binding in self
                .store
                .list::<HelperBinding>("binding", Some(&record.view.account_id))?
            {
                if binding.origin == record.view.panel_origin
                    && binding.installation_id == record.installation_id
                    && Some(&binding.credential_revision)
                        == record.view.credential_revision.as_ref()
                {
                    binding.expires_at = key.expires_at;
                    self.store
                        .put("binding", &binding.id, &binding.account_id, &binding)?;
                }
            }
        }
        Ok(())
    }
    fn cleanup_pending(&self, record: &mut ConnectionRecord) -> Result<bool> {
        let pending = std::mem::take(&mut record.cleanup_references);
        for reference in pending {
            if self.vault.delete(&reference).is_err()
                && !record.cleanup_references.contains(&reference)
            {
                record.cleanup_references.push(reference);
            }
        }
        let failed = !record.cleanup_references.is_empty();
        if failed {
            record.session_status = "vault_unavailable".into();
            record.view.permission_state = "unavailable".into();
        }
        self.store.put(
            "connection",
            &record.view.id,
            &record.view.account_id,
            record,
        )?;
        Ok(failed)
    }
    fn disable_inference_before_cleanup(
        &self,
        record: &mut ConnectionRecord,
        state: &str,
    ) -> Result<()> {
        // The durable authorization epoch and non-ready record precede all fallible
        // secret deletion, so a denied delete can never reopen a helper grant.
        record.authorization_epoch = self.advance_authorization_epoch(record)?;
        if let Some(reference) = record.credential_reference.take() {
            if !record.cleanup_references.contains(&reference) {
                record.cleanup_references.push(reference);
            }
        }
        record.view.credential_revision = None;
        record.credential_expires_at = None;
        record.credential_state = state.into();
        record.view.permission_state = "unavailable".into();
        record.view.revision += 1;
        self.store.put(
            "connection",
            &record.view.id,
            &record.view.account_id,
            record,
        )?;
        self.store
            .disable_bindings(&record.view.account_id, &record.view.panel_origin)?;
        self.stop_claude_mappings_for(&record.view.account_id, &record.view.panel_origin)?;
        let _ = self.cleanup_pending(record)?;
        self.invalidate_evidence()
    }
    async fn refresh_account_cache(&self, id: &str, activate: bool) -> Result<()> {
        let _operation = self.begin_mutation()?;
        let lock = self.network_lock(&format!("account/{id}"))?;
        let _serial = lock.lock().await;
        let (record, installation, session) = self.fresh_session(id).await?;
        let value: Value = self
            .http
            .panel(
                &record.view.panel_origin,
                "/api/v1/orbit/bootstrap",
                Method::GET,
                vec![],
                Some((&installation, &session, self.vault.as_ref())),
                true,
            )
            .await?;
        let mut cache = map_account_cache(&value, &record)?;
        cache.account.updated_at = Some(chrono::Utc::now().to_rfc3339());
        {
            let _gate = self
                .account_gate
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            let mut current = self.record(id)?;
            if current.session_status != "authenticated"
                || current.view.revision != record.view.revision
                || current.installation_id != record.installation_id
                || current.authorization_epoch != record.authorization_epoch
            {
                return Err(fail("connection_conflict"));
            }
            current.view.label = cache.account.label.clone();
            self.apply_known_key_state(&mut current, &cache)?;
            current.view.permission_state = if !cache.inference_allowed {
                "readonly"
            } else if current.credential_reference.is_some() {
                "allowed"
            } else {
                "unavailable"
            }
            .into();
            let old_cache = self.account_cache(&current).ok();
            let mut invalidate = false;
            if let Some(node_id) = &current.view.node_id {
                if let Some(node) = cache.nodes.iter().find(|n| &n.id == node_id) {
                    if current.view.edge_base_url.as_deref() != Some(&node.base_url)
                        || old_cache
                            .as_ref()
                            .and_then(|c| c.nodes.iter().find(|n| &n.id == node_id))
                            .is_some_and(|n| n.revision != node.revision)
                    {
                        current.view.revision += 1;
                        invalidate = true;
                    }
                    current.view.node_label = Some(node.label.clone());
                    current.view.edge_base_url = Some(node.base_url.clone());
                } else {
                    current.view.node_id = None;
                    current.view.edge_base_url = None;
                    current.view.node_label = None;
                    current.view.revision += 1;
                    invalidate = true;
                }
            }
            self.store
                .put("account-cache", id, &current.view.account_id, &cache)?;
            self.store
                .put("connection", id, &current.view.account_id, &current)?;
            if activate {
                self.store.set_meta("active_connection", id)?;
                invalidate = true;
            }
            if invalidate {
                self.invalidate_evidence()?;
            }
        }
        self.changed();
        Ok(())
    }
    pub async fn refresh_account(&self, id: &str, revision: u64) -> Result<BootstrapView> {
        let _operation = self.begin_mutation()?;
        let record = self.record(id)?;
        if record.view.revision != revision || record.session_status != "authenticated" {
            return Err(fail("connection_conflict"));
        }
        self.refresh_account_cache(id, false).await?;
        if self.image_workers.available_permits() == 2 {
            for mut snapshot in self
                .store
                .list::<TaskSnapshot>("task-snapshot", Some(&record.view.account_id))?
            {
                if snapshot.task.connection_id == id
                    && snapshot.panel_origin == record.view.panel_origin
                    && snapshot.installation_id == record.installation_id
                    && snapshot.task.request_id.is_some()
                    && !["queued", "sending", "waiting", "saving"]
                        .contains(&snapshot.task.state.as_str())
                    && (snapshot.task.result_state == "unconfirmed"
                        || [
                            "unavailable",
                            "pending",
                            "reconciliation_pending",
                            "incomplete",
                        ]
                        .contains(&snapshot.task.charge_state.as_str()))
                {
                    let _ = self.reconcile_task(&mut snapshot).await;
                }
            }
        }
        self.bootstrap()
    }
    pub fn select_connection(&self, id: &str) -> Result<BootstrapView> {
        let _operation = self.begin_mutation()?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        self.abort_pending_locked()?;
        let _ = self.record(id)?;
        self.store.set_meta("active_connection", id)?;
        self.invalidate_evidence()?;
        self.changed();
        self.bootstrap()
    }
    pub fn connection_view(&self, id: &str) -> Result<ConnectionView> {
        Ok(self.record(id)?.view)
    }
    pub fn read_preview_asset(&self, path: &str) -> Result<Vec<u8>> {
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let parts = path
            .strip_prefix('/')
            .ok_or_else(|| fail("preview_scope_unavailable"))?
            .split('/')
            .collect::<Vec<_>>();
        if parts.len() != 2
            || parts[0].len() != 64
            || !parts[0].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(fail("preview_scope_unavailable"));
        }
        let id = parts[1]
            .strip_suffix(".png")
            .ok_or_else(|| fail("preview_scope_unavailable"))?;
        validate_uuid(id)?;
        let scope = self
            .store
            .preview_scope()?
            .ok_or_else(|| fail("preview_scope_unavailable"))?;
        if parts[0] != scope.token {
            return Err(fail("preview_scope_unavailable"));
        }
        let input = self.store.get::<InputRecord>("input", id)?;
        let input_owned = input.is_some_and(|i| {
            i.account_id == scope.account_id
                && i.connection_id == scope.connection_id
                && i.connection_revision == scope.connection_revision
        });
        let history_owned = if let Some(image) = self.store.get::<HistoryImage>("image", id)? {
            self.store
                .get::<TaskSnapshot>("task-snapshot", &image.task_id)?
                .is_some_and(|s| {
                    image.account_id == scope.account_id
                        && s.task.account_id == scope.account_id
                        && s.panel_origin == scope.origin
                        && s.task.id == image.task_id
                })
        } else {
            false
        };
        if !input_owned && !history_owned {
            return Err(fail("preview_scope_unavailable"));
        }
        let bytes = images::bounded_read(
            &self.store.root.join("previews").join(format!("{id}.png")),
            4 * 1024 * 1024,
        )?;
        if image::guess_format(&bytes).ok() != Some(image::ImageFormat::Png) {
            return Err(fail("invalid_image"));
        }
        Ok(bytes)
    }
    pub fn active_tasks(&self) -> Result<bool> {
        if self.claude_mappings.inflight() != 0
            || self.image_workers.available_permits() != 2
            || self.image_memory_workers.available_permits() != 2
            || !self
                .received_memory
                .lock()
                .map_err(|_| fail("state_unavailable"))?
                .is_empty()
        {
            return Ok(true);
        }
        Ok(self.store.list::<TaskView>("task", None)?.iter().any(|t| {
            t.local_save_available
                || ["queued", "sending", "waiting", "saving"].contains(&t.state.as_str())
        }))
    }
    pub fn select_node(
        &self,
        id: &str,
        node_id: &str,
        expected_revision: u64,
    ) -> Result<BootstrapView> {
        let _operation = self.begin_mutation()?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let mut record = self.record(id)?;
        if record.view.revision != expected_revision {
            return Err(fail("connection_conflict"));
        }
        let cache = self.account_cache(&record)?;
        let node = cache
            .nodes
            .iter()
            .find(|n| n.id == node_id && n.available)
            .ok_or_else(|| fail("node_unavailable"))?;
        record.view.node_id = Some(node.id.clone());
        record.view.node_label = Some(node.label.clone());
        record.view.edge_base_url = Some(node.base_url.clone());
        record.view.revision += 1;
        self.store
            .put("connection", id, &record.view.account_id, &record)?;
        self.invalidate_evidence()?;
        self.changed();
        self.bootstrap()
    }
    pub async fn list_models(&self, id: &str) -> Result<Vec<ModelView>> {
        let _operation = self.begin_mutation()?;
        let (record, installation, session) = self.fresh_session(id).await?;
        let node = record
            .view
            .node_id
            .as_ref()
            .ok_or_else(|| fail("node_unavailable"))?;
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("node_id", node)
            .finish();
        let target = format!("/api/v1/orbit/models?{query}");
        let envelope: crate::network::PanelEnvelope<Vec<Value>> = self
            .http
            .panel_envelope(
                &record.view.panel_origin,
                &target,
                Method::GET,
                vec![],
                Some((&installation, &session, self.vault.as_ref())),
                true,
            )
            .await?;
        let revision = text(&envelope.meta, "revision")?;
        let models = envelope
            .data
            .iter()
            .map(|v| map_model(v, &revision))
            .collect::<Result<Vec<_>>>()?;
        {
            let _gate = self
                .account_gate
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            let current = self.record(id)?;
            if current.view.revision != record.view.revision
                || current.session_status != "authenticated"
            {
                return Err(fail("connection_conflict"));
            }
            let mut cache = self.account_cache(&record)?;
            if cache.catalog_revision != revision {
                self.invalidate_evidence()?;
            }
            cache.catalog_revision = revision;
            cache.models = models.clone();
            self.store
                .put("account-cache", id, &record.view.account_id, &cache)?;
        }
        self.changed();
        Ok(models)
    }
    fn commit_logout(&self, id: &str) -> Result<()> {
        let _operation = self.begin_mutation()?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let mut record = self.record(id)?;
        record.authorization_epoch = self.advance_authorization_epoch(&record)?;
        if record.session_status != "logged_out"
            && !record
                .cleanup_references
                .contains(&record.session_reference)
        {
            record
                .cleanup_references
                .push(record.session_reference.clone());
        }
        if let Some(reference) = record.credential_reference.take() {
            if !record.cleanup_references.contains(&reference) {
                record.cleanup_references.push(reference);
            }
        }
        record.view.credential_revision = None;
        record.credential_state = "unavailable".into();
        record.credential_expires_at = None;
        record.view.permission_state = "unavailable".into();
        record.view.revision += 1;
        record.refresh_attempt = None;
        record.session_status = "logged_out".into();
        self.store
            .put("connection", id, &record.view.account_id, &record)?;
        self.store
            .disable_bindings(&record.view.account_id, &record.view.panel_origin)?;
        self.stop_claude_mappings_for(&record.view.account_id, &record.view.panel_origin)?;
        let secret_failure = self.cleanup_pending(&mut record)?;
        self.invalidate_evidence()?;
        if secret_failure {
            return Err(fail("vault_unavailable"));
        }
        Ok(())
    }
    pub async fn logout(&self, id: &str) -> Result<BootstrapView> {
        let _operation = self.begin_mutation()?;
        {
            let _gate = self
                .account_gate
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            self.abort_pending_locked()?;
        }
        let record = self.record(id)?;
        let remote = if let Ok(session) = self.session(&record) {
            let install = self.installation(&record)?;
            Some((install, session))
        } else {
            None
        };
        self.commit_logout(id)?;
        self.changed();
        if let Some((install, session)) = remote {
            let _ = self
                .http
                .panel::<Value>(
                    &record.view.panel_origin,
                    "/api/v1/orbit/sessions/current/revoke",
                    Method::POST,
                    b"{}".to_vec(),
                    Some((&install, &session, self.vault.as_ref())),
                    true,
                )
                .await;
        }
        self.bootstrap()
    }
    pub async fn revoke_inference_key(
        &self,
        id: &str,
        expected: &str,
        confirmed: bool,
    ) -> Result<BootstrapView> {
        let _operation = self.begin_mutation()?;
        if !confirmed {
            return Err(fail("confirmation_required"));
        }
        if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(fail("invalid_credential_revision"));
        }
        let initial = self.record(id)?;
        if initial.view.credential_revision.as_deref() != Some(expected) {
            return Err(fail("connection_conflict"));
        }
        let (record, installation, session) = self.fresh_session(id).await?;
        if record.view.permission_state != "allowed" {
            return Err(fail("permission_denied"));
        }
        if record.view.credential_revision.as_deref() != Some(expected) {
            return Err(fail("connection_conflict"));
        }
        let body = serde_json::to_vec(&json!({"expected_key_revision":expected}))
            .map_err(|_| fail("serialization_failed"))?;
        let response: Value = self
            .http
            .panel(
                &record.view.panel_origin,
                "/api/v1/orbit/installations/current/inference-key/revoke",
                Method::POST,
                body,
                Some((&installation, &session, self.vault.as_ref())),
                true,
            )
            .await?;
        if response["revoked"] != true {
            return Err(fail("invalid_panel_response"));
        }
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let mut current = self.record(id)?;
        if current.view.credential_revision.as_deref() != Some(expected) {
            return Err(fail("connection_conflict"));
        }
        self.disable_inference_before_cleanup(&mut current, "revoked")?;
        self.changed();
        self.bootstrap()
    }
    pub async fn delete_connection(
        &self,
        id: &str,
        expected_revision: u64,
        expected_credential: Option<&str>,
        revoke: bool,
        confirmed: bool,
    ) -> Result<BootstrapView> {
        let _operation = self.begin_mutation()?;
        if !confirmed {
            return Err(fail("confirmation_required"));
        }
        let initial = self.record(id)?;
        if initial.view.revision != expected_revision
            || initial.view.credential_revision.as_deref() != expected_credential
        {
            return Err(fail("connection_conflict"));
        }
        if revoke {
            self.revoke_inference_key(
                id,
                expected_credential.ok_or_else(|| fail("credential_missing"))?,
                true,
            )
            .await?;
        } else {
            let current = self.record(id)?;
            if current.view.revision != expected_revision
                || current.view.credential_revision.as_deref() != expected_credential
            {
                return Err(fail("connection_conflict"));
            }
        }
        self.logout(id).await?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let after = self.record(id)?;
        if after.session_status != "logged_out"
            || after.view.revision != expected_revision + 1 + u64::from(revoke)
        {
            return Err(fail("connection_conflict"));
        }
        self.store.delete("connection", id)?;
        self.store.delete("account-cache", id)?;
        if self.store.meta("active_connection")?.as_deref() == Some(id) {
            self.store.set_meta("active_connection", "")?;
        }
        self.changed();
        self.bootstrap()
    }
    fn current_connection(
        &self,
        id: &str,
        revision: u64,
    ) -> Result<(ConnectionRecord, AccountCache)> {
        let record = self.record(id)?;
        self.ensure_installation_available(&record.installation_id)?;
        let active = self
            .active_record()?
            .ok_or_else(|| fail("session_expired"))?;
        if active.view.id != id
            || record.view.revision != revision
            || record.session_status != "authenticated"
            || record.view.permission_state != "allowed"
            || record
                .session_expires_at
                .is_none_or(|at| at <= chrono::Utc::now().timestamp())
            || record.credential_reference.is_none()
            || record.view.credential_revision.is_none()
            || record.credential_state != "active"
            || record
                .credential_expires_at
                .is_some_and(|at| at <= chrono::Utc::now().timestamp())
        {
            return Err(fail("connection_conflict"));
        }
        let cache = self.account_cache(&record)?;
        if !cache.inference_allowed {
            return Err(fail("permission_denied"));
        }
        Ok((record, cache))
    }
    fn context(
        &self,
        record: &ConnectionRecord,
        cache: &AccountCache,
        model: &ModelView,
    ) -> Result<PreviewContext> {
        let node_id = record
            .view
            .node_id
            .clone()
            .ok_or_else(|| fail("node_unavailable"))?;
        let node = cache
            .nodes
            .iter()
            .find(|n| n.id == node_id && n.available)
            .ok_or_else(|| fail("node_unavailable"))?;
        Ok(PreviewContext {
            account_id: record.view.account_id.clone(),
            account_label: cache.account.label.clone(),
            connection_id: record.view.id.clone(),
            connection_revision: record.view.revision,
            credential_revision: record
                .view
                .credential_revision
                .clone()
                .ok_or_else(|| fail("credential_missing"))?,
            node_id,
            node_label: node.label.clone(),
            model_revision: model.revision.clone(),
        })
    }
    fn config_engine(&self) -> ConfigEngine<'_> {
        ConfigEngine {
            store: &self.store,
            vault: self.vault.as_ref(),
            allowed_roots: self.paths.configuration_roots.clone(),
            policy_guard: None,
            context_guard: None,
            claude_context_guard: None,
        }
    }
    fn codex_engine<'a>(
        &'a self,
        guard: &'a dyn Fn() -> Result<()>,
        context_guard: &'a dyn Fn(Option<&CodexContext>, bool) -> Result<()>,
    ) -> ConfigEngine<'a> {
        ConfigEngine {
            store: &self.store,
            vault: self.vault.as_ref(),
            allowed_roots: self.paths.configuration_roots.clone(),
            policy_guard: Some(guard),
            context_guard: Some(context_guard),
            claude_context_guard: None,
        }
    }
    fn observe_configuration(&self, client: ClientKind) -> Result<ConfigObservation> {
        let (mut target, mut policy) = self.client_policy(client)?;
        let (configuration_target, runtime_view) = if client == ClientKind::Codex {
            let snapshot = self.paths.codex_runtime();
            (
                self.paths.codex_configuration_target(&target).ok(),
                Some(snapshot.view()),
            )
        } else {
            (None, None)
        };
        let (claude_configuration_target, claude_configuration_error) =
            if client == ClientKind::Claude {
                match self.paths.claude_configuration_target(&target) {
                    Ok(target) => (Some(target), None),
                    Err(error) => (None, Some(error)),
                }
            } else {
                (None, None)
            };
        if claude_configuration_target.is_some() {
            let (after_target, after_policy) = self.client_policy(client)?;
            if policy.revision != after_policy.revision
                || policy.local_revision != after_policy.local_revision
            {
                return Err(fail("managed_policy_changed"));
            }
            target = after_target;
            policy = after_policy;
        }
        Ok(ConfigObservation {
            target,
            policy,
            configuration_target,
            claude_configuration_target,
            claude_configuration_error,
            runtime_view,
        })
    }
    fn require_configuration(
        &self,
        observation: &ConfigObservation,
        expected_policy: Option<&crate::policy::Snapshot>,
        expected_target: Option<&CodexConfigurationTarget>,
    ) -> Result<()> {
        if !self.paths.writes_authorized {
            return Err(fail("integration_authorization_required"));
        }
        if observation.target.client == ClientKind::Claude {
            return self.require_claude_configuration(observation, expected_policy, None);
        }
        if observation.target.client == ClientKind::Codex {
            let current = observation
                .configuration_target
                .as_ref()
                .ok_or_else(|| fail("codex_configuration_target_unverified"))?;
            current.validate_root()?;
            if self.paths.codex_config != current.config_root.join("config.toml") {
                return Err(fail("codex_configuration_target_unverified"));
            }
            if expected_target.is_some_and(|expected| expected != current) {
                return Err(fail("codex_configuration_target_changed"));
            }
        }
        if observation.target.client == ClientKind::Codex {
            if let Some(expected) = expected_policy {
                expected.matches_user_config(&observation.policy)?;
            } else {
                observation.policy.require_user_config_clear()?;
            }
        } else if let Some(expected) = expected_policy {
            expected.matches(&observation.policy)?;
        } else {
            observation.policy.require_clear()?;
        }
        if !observation.target.installed
            || !observation.target.platform_supported
            || observation.target.version.is_none()
        {
            return Err(fail("client_unsupported"));
        }
        Ok(())
    }
    /// Used before any account/session refresh, Vault access or configuration mutation.
    pub fn configuration_preflight(&self, client: ClientKind) -> Result<()> {
        self.require_configuration(&self.observe_configuration(client)?, None, None)
    }
    fn validate_configuration(
        &self,
        client: ClientKind,
        policy: &crate::policy::Snapshot,
        configuration_target: Option<&CodexConfigurationTarget>,
    ) -> Result<()> {
        self.require_configuration(
            &self.observe_configuration(client)?,
            Some(policy),
            configuration_target,
        )
    }
    fn codex_inputs(
        &self,
        record: &ConnectionRecord,
        cache: &AccountCache,
    ) -> Vec<CodexModelInput> {
        cache
            .models
            .iter()
            .filter(|model| {
                model.provider_id == record.view.provider_id
                    && model.category == "text"
                    && model
                        .capabilities
                        .iter()
                        .any(|capability| capability == "openai.responses")
            })
            .map(|model| CodexModelInput {
                id: model.id.clone(),
                provider_id: model.provider_id.clone(),
                revision: model.revision.clone(),
                category: model.category.clone(),
                capabilities: model.capabilities.clone(),
                binding: model.codex_catalog_binding.clone(),
                reasoning: model.reasoning.clone(),
            })
            .collect()
    }
    fn plan_codex(
        &self,
        record: &ConnectionRecord,
        cache: &AccountCache,
        selected: &str,
    ) -> Result<crate::codex_binding::PlannedCatalog> {
        crate::codex_binding::compile_scope(
            &crate::codex_binding::CatalogScope {
                context: crate::codex_binding::ScopeContext {
                    panel_origin: &record.view.panel_origin,
                    account_id: &record.view.account_id,
                    installation_id: &record.installation_id,
                    connection_id: &record.view.id,
                    connection_revision: record.view.revision,
                    authorization_epoch: record.authorization_epoch,
                    credential_revision: record
                        .view
                        .credential_revision
                        .as_deref()
                        .ok_or_else(|| fail("credential_missing"))?,
                    node_id: record
                        .view
                        .node_id
                        .as_deref()
                        .ok_or_else(|| fail("node_unavailable"))?,
                    provider_id: &record.view.provider_id,
                    catalog_revision: &cache.catalog_revision,
                },
                models: &cache.models,
            },
            selected,
        )
    }
    fn freeze_codex(
        &self,
        record: &ConnectionRecord,
        cache: &AccountCache,
        planned: &crate::codex_binding::PlannedCatalog,
        configuration_target: &CodexConfigurationTarget,
    ) -> Result<CodexContext> {
        let node = cache
            .nodes
            .iter()
            .find(|node| Some(&node.id) == record.view.node_id.as_ref() && node.available)
            .ok_or_else(|| fail("node_unavailable"))?;
        if Some(&node.base_url) != record.view.edge_base_url.as_ref() {
            return Err(fail("codex_catalog_scope_changed"));
        }
        Ok(CodexContext {
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
            edge_base_url: node.base_url.clone(),
            provider_id: record.view.provider_id.clone(),
            catalog_revision: cache.catalog_revision.clone(),
            models: self.codex_inputs(record, cache),
            profile_id: crate::codex_binding::PROFILE_ID.into(),
            source_sha256: crate::codex_catalog::SOURCE_SHA256.into(),
            scope_revision: sha256(
                &serde_json::to_vec(&(planned.scope_revision(), &node.revision, &node.base_url))
                    .map_err(|_| fail("serialization_failed"))?,
            ),
            catalog_sha256: planned.catalog().sha256().into(),
            catalog_size: planned.catalog().bytes().len() as u64,
            aliases: planned.catalog().main_aliases().to_vec(),
            default_alias: planned.catalog().default_alias().into(),
            default_reasoning_effort: planned.default_reasoning_effort().into(),
            catalog_path: self
                .paths
                .codex_catalog()
                .to_str()
                .ok_or_else(|| fail("invalid_path"))?
                .into(),
            config_path: self
                .paths
                .codex_config
                .to_str()
                .ok_or_else(|| fail("invalid_path"))?
                .into(),
            configuration_target: Some(configuration_target.clone()),
        })
    }
    fn validate_codex_context(&self, context: Option<&CodexContext>, restore: bool) -> Result<()> {
        let Some(context) = context else {
            return Err(fail("codex_catalog_context_unverified"));
        };
        if Path::new(&context.catalog_path) != self.paths.codex_catalog()
            || Path::new(&context.config_path) != self.paths.codex_config
        {
            return Err(fail("codex_catalog_scope_changed"));
        }
        let frozen_target = context
            .configuration_target
            .as_ref()
            .ok_or_else(|| fail("codex_catalog_context_unverified"))?;
        let (target, _) = self.client_policy(ClientKind::Codex)?;
        if self.paths.codex_configuration_target(&target)? != *frozen_target {
            return Err(fail("codex_configuration_target_changed"));
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
        if restore {
            return Ok(());
        }
        let (record, cache) = self
            .current_connection(&context.connection_id, context.connection_revision)
            .map_err(|_| fail("codex_catalog_scope_changed"))?;
        let node = cache
            .nodes
            .iter()
            .find(|node| node.id == context.node_id && node.available);
        let epoch = self
            .store
            .meta(&authorization_epoch_reference(
                &context.panel_origin,
                &context.installation_id,
                &context.account_id,
            ))?
            .and_then(|value| value.parse::<u64>().ok());
        if context.profile_id != crate::codex_binding::PROFILE_ID
            || context.source_sha256 != crate::codex_catalog::SOURCE_SHA256
            || record.view.panel_origin != context.panel_origin
            || record.view.account_id != context.account_id
            || record.installation_id != context.installation_id
            || record.authorization_epoch != context.authorization_epoch
            || epoch != Some(context.authorization_epoch)
            || record.view.credential_revision.as_ref() != Some(&context.credential_revision)
            || record.credential_reference.as_ref() != Some(&context.credential_reference)
            || record.credential_expires_at != context.credential_expires_at
            || record.api_key_id != context.api_key_id
            || cache.account.id != context.account_id
            || record.view.node_id.as_ref() != Some(&context.node_id)
            || record.view.provider_id != context.provider_id
            || record.view.edge_base_url.as_ref() != Some(&context.edge_base_url)
            || node.is_none_or(|node| {
                node.revision != context.node_revision || node.base_url != context.edge_base_url
            })
            || cache.catalog_revision != context.catalog_revision
            || self.codex_inputs(&record, &cache) != context.models
        {
            return Err(fail("codex_catalog_scope_changed"));
        }
        Ok(())
    }
    fn project_configuration_revision(&self, target: &mut ClientView) {
        if target.client == ClientKind::Claude {
            let current = self.claude_revision();
            if current.as_ref().is_err()
                || target.configuration_revision != current.as_ref().ok().cloned().flatten()
            {
                target.loaded_evidence = None;
                target.connection_evidence = None;
                target.loaded_state = "unconfirmed".into();
                target.connection_state = "not_tested".into();
                if target.configuration_state == "saved" {
                    target.configuration_state = "conflict".into();
                }
            }
            target.configuration_revision = current.ok().flatten();
            return;
        }
        if target.client != ClientKind::Codex {
            return;
        }
        target.config_paths = vec![
            self.paths.codex_catalog().display().to_string(),
            self.paths.codex_config.display().to_string(),
        ];
        let observed =
            codex_configuration_revision(&self.paths.codex_config, &self.paths.codex_catalog());
        if observed.as_ref().is_err()
            || target.configuration_revision != observed.as_ref().ok().cloned().flatten()
        {
            target.loaded_evidence = None;
            target.connection_evidence = None;
            target.loaded_state = "unconfirmed".into();
            target.connection_state = "not_tested".into();
            if target.configuration_state == "saved" {
                target.configuration_state = "conflict".into();
            }
        }
        target.configuration_revision = observed.ok().flatten();
    }
    fn client_policy(&self, client: ClientKind) -> Result<(ClientView, crate::policy::Snapshot)> {
        let target = self
            .clients
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .iter()
            .find(|c| c.client == client)
            .cloned()
            .ok_or_else(|| fail("client_unsupported"))?;
        Ok(self.observe_client_policy(target))
    }
    fn observe_client_policy(
        &self,
        mut target: ClientView,
    ) -> (ClientView, crate::policy::Snapshot) {
        if target.client == ClientKind::Codex {
            target = self.paths.fresh_codex_client(&target);
        } else {
            target = self.paths.fresh_claude_client(&target);
        }
        self.project_configuration_revision(&mut target);
        let policy = crate::policy::observe(&self.paths, &target);
        policy.project(&mut target);
        target.configuration_access = ConfigurationAccess::Unverified;
        target.configuration_access_revision = None;
        if self.paths.writes_authorized {
            if policy.state == crate::policy::State::Yes
                || policy.user_config_state() == crate::policy::State::Yes
            {
                target.configuration_access = ConfigurationAccess::Blocked;
            } else if policy.require_user_config_clear().is_ok() {
                let revision = match target.client {
                    ClientKind::Codex => self.paths.codex_configuration_hint(&target),
                    ClientKind::Claude => self.paths.claude_configuration_hint(&target),
                };
                if let Ok(revision) = revision {
                    target.configuration_access = ConfigurationAccess::Allowed;
                    target.configuration_access_revision = Some(match target.client {
                        ClientKind::Codex => {
                            configuration_access_revision(&revision, &policy.local_revision)
                        }
                        ClientKind::Claude => {
                            claude::access_revision(&revision, &policy.local_revision)
                        }
                    });
                }
            }
        }
        if policy.state != crate::policy::State::No {
            target.loaded_evidence = None;
            target.connection_evidence = None;
            target.loaded_state = if target.configuration_state == "saved" {
                "restart_required"
            } else {
                "unconfirmed"
            }
            .into();
            target.connection_state = "not_tested".into();
        }
        (target, policy)
    }
    fn pending_configuration_recovery(&self, client: ClientKind) -> Result<bool> {
        for journal in self.store.list::<Journal>("journal", None)? {
            if journal.client == client
                && (["publishing", "conflict"].contains(&journal.phase.as_str())
                    || journal.phase == "committed"
                        && self.store.get::<Journal>("backup", &journal.id)?.is_none())
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn coordinate_configuration_recovery(&self) -> Result<()> {
        for journal in self.store.list::<Journal>("journal", None)? {
            if journal.client == ClientKind::Claude {
                self.coordinate_claude_journal(&journal)?;
                continue;
            }
            if !["publishing", "conflict", "committed"].contains(&journal.phase.as_str())
                || journal.phase == "committed"
                    && self.store.get::<Journal>("backup", &journal.id)?.is_some()
            {
                continue;
            }
            let observation = self.observe_configuration(journal.client)?;
            // Unknown provenance/policy cannot access Vault or mutate recovery state.
            if journal.client != ClientKind::Codex
                || self
                    .require_configuration(&observation, None, None)
                    .is_err()
            {
                continue;
            }
            let target = &observation.target;
            let qualified = target.version == journal.target_version
                && target.bundle_id.is_some()
                && target.bundle_id == journal.target_bundle_id
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
                self.validate_configuration(
                    journal.client,
                    &observation.policy,
                    observation.configuration_target.as_ref(),
                )
            };
            let context_guard = |context: Option<&CodexContext>, restore: bool| {
                self.validate_codex_context(context, restore)
            };
            let recovered = qualified
                && journal.phase != "conflict"
                && self
                    .codex_engine(&guard, &context_guard)
                    .recover(&journal.id)
                    .is_ok();
            guard()?;
            let revision = if recovered {
                codex_configuration_revision(&self.paths.codex_config, &self.paths.codex_catalog())?
            } else {
                None
            };
            for client in self
                .clients
                .lock()
                .map_err(|_| fail("state_unavailable"))?
                .iter_mut()
                .filter(|c| c.client == journal.client)
            {
                client.configuration_state = if recovered { "saved" } else { "conflict" }.into();
                if recovered {
                    client.configuration_revision = revision.clone();
                }
                client.loaded_evidence = None;
                client.connection_evidence = None;
                client.loaded_state = if recovered {
                    "restart_required"
                } else {
                    "unconfirmed"
                }
                .into();
                client.connection_state = "not_tested".into();
                client.manual_steps.push(if recovered { "已协调此前用户确认的整组本地操作；尚需重启，未证明加载或连通。" }
                    else { "整组配置操作尚待恢复：先核验执行来源、完整账户目录上下文、托管策略与安全存储并协调外部修改；同客户端新应用已阻止。" }.into());
                self.store
                    .put("client-state", client.client.as_str(), "local", client)?;
            }
        }
        Ok(())
    }
    /// Refresh only after the native, read-only configuration gate succeeds.
    pub async fn preview_client_config_refreshed(
        &self,
        id: &str,
        revision: u64,
        client: ClientKind,
        model_id: &str,
    ) -> Result<ConfigPreview> {
        self.configuration_preflight(client)?;
        self.refresh_account_cache(id, false).await?;
        self.configuration_preflight(client)?;
        let _ = self.list_models(id).await?;
        self.preview_client_config(id, revision, client, model_id)
    }
    pub fn preview_client_config(
        &self,
        id: &str,
        revision: u64,
        client: ClientKind,
        model_id: &str,
    ) -> Result<ConfigPreview> {
        if client == ClientKind::Claude {
            return self.preview_claude_config(id, revision, model_id);
        }
        self.preview_client_config_legacy(id, revision, client, model_id)
    }
    #[cfg(test)]
    fn preview_claude_diagnostic(
        &self,
        id: &str,
        revision: u64,
        model_id: &str,
    ) -> Result<ConfigPreview> {
        self.preview_client_config_legacy(id, revision, ClientKind::Claude, model_id)
    }
    fn preview_client_config_legacy(
        &self,
        id: &str,
        revision: u64,
        client: ClientKind,
        model_id: &str,
    ) -> Result<ConfigPreview> {
        let _operation = self.begin_mutation()?;
        let observation = self.observe_configuration(client)?;
        let gate_error = self.require_configuration(&observation, None, None).err();
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let (record, cache) = self.current_connection(id, revision)?;
        let model = cache
            .models
            .iter()
            .find(|m| {
                m.id == model_id && m.category == "text" && m.provider_id == record.view.provider_id
            })
            .ok_or_else(|| fail("model_unavailable"))?;
        let context = self.context(&record, &cache, model)?;
        let planned = if client == ClientKind::Codex {
            Some(self.plan_codex(&record, &cache, model_id)?)
        } else {
            None
        };
        let catalog_context = planned
            .as_ref()
            .zip(observation.configuration_target.as_ref())
            .map(|(planned, configuration_target)| {
                self.freeze_codex(&record, &cache, planned, configuration_target)
            })
            .transpose()?;
        let target = &observation.target;
        let policy = &observation.policy;
        let configuration_target = observation.configuration_target.as_ref();
        let guard = || {
            if client == ClientKind::Claude {
                self.validate_claude_configuration(
                    policy,
                    observation.claude_configuration_target.as_ref(),
                )
            } else {
                self.validate_configuration(client, policy, configuration_target)
            }
        };
        let context_guard = |current: Option<&CodexContext>, restore: bool| {
            if restore || current != catalog_context.as_ref() {
                return Err(fail("codex_catalog_context_unverified"));
            }
            self.validate_codex_context(current, false)
        };
        let now = chrono::Utc::now();
        let mut preview = ConfigPreview {
            id: uuid(), client, operation: "apply".into(), target_version: target.version.clone(),
            paths: target.config_paths.clone(), model_id: Some(model_id.into()), context: Some(context), restore_owner: None, restore_source: None, restore_coordination: None,
            impact: match client {
                ClientKind::Codex => vec![
                    "修改用户默认模型、推理默认值、Orivelle provider 与独立模型目录；Codex、CLI 与 IDE 可能共用此配置。".into(),
                    "应用后需重启；保存不证明客户端载入或连通，也不约束其他启动参数与 plan mode 等调用路径。".into(),
                    "默认 .codex 目录缺失时仅在确认后创建为用户私有目录；恢复文件后该目录可保留为空目录。".into(),
                ],
                ClientKind::Claude => vec![
                    "第三方模式影响 Claude 整个桌面应用，包括 Chat、Code 与 Cowork。".into(),
                    "保留官方模式选择、组织策略和原登录资料。".into(),
                ],
            },
            redacted_diff: "No file mutation is permitted for this preview.".into(),
            expires_at: (now + chrono::Duration::minutes(5)).to_rfc3339(), can_apply: false,
            can_export_import: false, blocking_reason: None,
            codex_catalog: planned.as_ref().map(|planned| CodexCatalogPreview {
                profile_id: crate::codex_binding::PROFILE_ID.into(), cli_version: crate::codex_catalog::SUPPORTED_CLI_VERSION.into(),
                source_sha256: crate::codex_catalog::SOURCE_SHA256.into(), sha256: planned.catalog().sha256().into(),
                scope_revision: catalog_context.as_ref().map(|context| context.scope_revision.clone())
                    .unwrap_or_else(|| planned.scope_revision().into()),
                path: self.paths.codex_catalog().display().to_string(), bytes: planned.catalog().bytes().len() as u64,
                aliases: planned.catalog().main_aliases().to_vec(), default_alias: planned.catalog().default_alias().into(),
                default_reasoning_effort: planned.default_reasoning_effort().into(),
            }),
            codex_runtime: observation.runtime_view.clone(),
            codex_configuration_target: configuration_target_view(&observation),
            claude_configuration_target: claude::target_view(&observation),
        };
        let mut journal_id = None;
        let mut import_candidate = None;
        let mut claude_diagnostic_scope = None;
        if let Some(error) = gate_error {
            preview.blocking_reason = Some(error.code);
        } else if self.pending_configuration_recovery(client)? {
            preview.blocking_reason = Some("configuration_recovery_required".into());
        } else if !self.paths.helper_available() {
            preview.blocking_reason = Some("helper_unavailable".into());
        } else if !model.capabilities.iter().any(|c| {
            c == if client == ClientKind::Codex {
                "openai.responses"
            } else {
                "anthropic.messages"
            }
        }) {
            preview.blocking_reason = Some("model_unsupported".into());
        } else {
            guard()?;
            if client == ClientKind::Claude {
                claude_diagnostic_scope =
                    Some(self.claude_diagnostic_scope(id, revision, model_id)?);
            }
            let install = self.installation(&record)?;
            let binding_id = uuid();
            // Finish the catalog/owned patch before any helper credential or durable mutation.
            let changes = if let Some(planned) = &planned {
                self.validate_codex_context(catalog_context.as_ref(), false)?;
                let before = read_file(&self.paths.codex_config)?;
                let after = codex_catalog_patch(
                    before.as_deref(),
                    record
                        .view
                        .edge_base_url
                        .as_deref()
                        .ok_or_else(|| fail("node_unavailable"))?,
                    model_id,
                    &self.paths.helper,
                    &binding_id,
                    &self.paths.codex_catalog(),
                    planned.default_reasoning_effort(),
                )?;
                preview.redacted_diff =
                    crate::config::codex_redacted_diff(before.as_deref(), &after)?;
                Some(vec![
                    FileChange {
                        path: self.paths.codex_catalog().display().to_string(),
                        before: read_file(&self.paths.codex_catalog())?,
                        after: Some(planned.catalog().bytes().to_vec()),
                    },
                    FileChange {
                        path: self.paths.codex_config.display().to_string(),
                        before,
                        after: Some(after),
                    },
                ])
            } else {
                None
            };
            let binding = HelperBinding {
                id: binding_id.clone(),
                client,
                origin: record.view.panel_origin.clone(),
                account_id: record.view.account_id.clone(),
                installation_id: record.installation_id.clone(),
                provider_id: record.view.provider_id.clone(),
                credential_revision: record
                    .view
                    .credential_revision
                    .clone()
                    .ok_or_else(|| fail("credential_missing"))?,
                secret_reference: record
                    .credential_reference
                    .clone()
                    .ok_or_else(|| fail("credential_missing"))?,
                enabled: false,
                expires_at: record.credential_expires_at,
                authorization_epoch: record.authorization_epoch,
            };
            let mut checked = binding.clone();
            checked.enabled = true;
            guard()?;
            if client == ClientKind::Codex {
                self.validate_codex_context(catalog_context.as_ref(), false)?;
            }
            let _ = bound_helper_token(
                &checked,
                client,
                &binding_id,
                &self.store,
                self.vault.as_ref(),
                now.timestamp(),
            )?;
            guard()?;
            if client == ClientKind::Codex {
                self.validate_codex_context(catalog_context.as_ref(), false)?;
            }
            self.store
                .put("binding", &binding_id, &record.view.account_id, &binding)?;
            if client == ClientKind::Claude {
                let base = cache
                    .claude_endpoints
                    .get(
                        record
                            .view
                            .node_id
                            .as_ref()
                            .ok_or_else(|| fail("node_unavailable"))?,
                    )
                    .ok_or_else(|| fail("node_unavailable"))?;
                let body = crate::config::claude_import_diagnostic(
                    base,
                    &self.paths.helper,
                    &binding_id,
                    model_id,
                )?;
                let bytes =
                    serde_json::to_vec_pretty(&body).map_err(|_| fail("serialization_failed"))?;
                preview.redacted_diff = std::str::from_utf8(&bytes)
                    .map_err(|_| fail("serialization_failed"))?
                    .into();
                preview.can_export_import = true;
                preview.blocking_reason = Some("schema_unverified".into());
                import_candidate = Some((bytes, binding_id));
            } else {
                let previous = self
                    .store
                    .list::<HelperBinding>("binding", None)?
                    .into_iter()
                    .filter(|b| b.client == client && b.enabled)
                    .map(|b| b.id)
                    .collect();
                let engine = self.codex_engine(&guard, &context_guard);
                let mut journal = engine.prepare_codex(
                    &record.view.account_id,
                    &install,
                    changes.ok_or_else(|| fail("codex_catalog_context_unverified"))?,
                    Some(binding_id),
                    previous,
                    catalog_context
                        .clone()
                        .ok_or_else(|| fail("codex_catalog_context_unverified"))?,
                )?;
                journal.account_label = cache.account.label.clone();
                engine.freeze_target(
                    &mut journal,
                    target.version.clone(),
                    target.bundle_id.clone(),
                )?;
                engine.freeze_policy(&mut journal, &policy.revision)?;
                journal_id = Some(journal.id);
                preview.can_apply = true;
            }
        }
        if client == ClientKind::Claude {
            preview.impact.push("仅生成官方 V2 正文诊断候选供核查；不承诺该候选可导入，不写配置库，尚无真实加载或连通证据。".into());
        }
        self.previews
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .insert(
                preview.id.clone(),
                PreviewEntry {
                    view: preview.clone(),
                    journal_id,
                    import_candidate,
                    policy: observation.policy,
                    configuration_target: observation.configuration_target,
                    claude_configuration_target: observation.claude_configuration_target,
                    claude_diagnostic_scope,
                    claude_context: None,
                    catalog_context,
                    _mapping_reservation: None,
                },
            );
        Ok(preview)
    }
    fn take_preview(&self, id: &str, operation: &str) -> Result<PreviewEntry> {
        validate_uuid(id)?;
        let mut previews = self
            .previews
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let entry = previews.remove(id).ok_or_else(|| fail("preview_expired"))?;
        if entry.view.operation != operation
            || chrono::DateTime::parse_from_rfc3339(&entry.view.expires_at)
                .map_err(|_| fail("preview_expired"))?
                <= chrono::Utc::now()
        {
            return Err(fail("preview_expired"));
        }
        if !entry.view.can_apply {
            return Err(fail(
                entry
                    .view
                    .blocking_reason
                    .as_deref()
                    .unwrap_or("configuration_unavailable"),
            ));
        }
        Ok(entry)
    }
    fn validate_preview_context(
        &self,
        preview: &ConfigPreview,
        policy: &crate::policy::Snapshot,
        configuration_target: Option<&CodexConfigurationTarget>,
    ) -> Result<()> {
        let observation = self.observe_configuration(preview.client)?;
        self.require_configuration(&observation, Some(policy), configuration_target)?;
        if observation.target.version != preview.target_version {
            return Err(fail("client_version_changed"));
        }
        if preview.operation == "restore" {
            let owner = preview
                .restore_owner
                .as_ref()
                .ok_or_else(|| fail("backup_invalid"))?;
            let install = self
                .store
                .installation(&owner.panel_origin)?
                .ok_or_else(|| fail("backup_invalid"))?;
            if install.id != owner.installation_id {
                return Err(fail("account_scope_mismatch"));
            }
            self.ensure_installation_available(&install.id)?;
            self.validate_configuration(preview.client, policy, configuration_target)?;
            let _ = self.vault.get(&install.backup_reference)?;
            return Ok(());
        }
        let context = preview
            .context
            .as_ref()
            .ok_or_else(|| fail("preview_expired"))?;
        let (record, cache) =
            self.current_connection(&context.connection_id, context.connection_revision)?;
        let model = cache
            .models
            .iter()
            .find(|m| {
                Some(&m.id) == preview.model_id.as_ref() && m.revision == context.model_revision
            })
            .ok_or_else(|| fail("connection_conflict"))?;
        let current = self.context(&record, &cache, model)?;
        if current.credential_revision != context.credential_revision
            || current.node_id != context.node_id
            || record.view.account_id != context.account_id
        {
            return Err(fail("connection_conflict"));
        }
        Ok(())
    }
    pub async fn apply_client_config_refreshed(
        &self,
        id: &str,
        confirmed: bool,
    ) -> Result<ConfigResult> {
        if !confirmed {
            return Err(fail("confirmation_required"));
        }
        validate_uuid(id)?;
        let (view, policy, configuration_target, claude_context, journal_id) = {
            let previews = self
                .previews
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            let entry = previews.get(id).ok_or_else(|| fail("preview_expired"))?;
            (
                entry.view.clone(),
                entry.policy.clone(),
                entry.configuration_target.clone(),
                entry.claude_context.clone(),
                entry.journal_id.clone(),
            )
        };
        let result = async {
            if view.operation != "apply"
                || !view.can_apply
                || parse_timestamp(&view.expires_at)? <= chrono::Utc::now().timestamp()
            {
                return Err(fail(
                    view.blocking_reason.as_deref().unwrap_or("preview_expired"),
                ));
            }
            // This gate precedes fresh_session(), including its Vault and cache writes.
            let authority = || {
                if view.client == ClientKind::Claude {
                    self.validate_claude_context(
                        claude_context
                            .as_ref()
                            .ok_or_else(|| fail("claude_configuration_context_unverified"))?,
                        false,
                    )?;
                    self.ensure_no_other_claude_recovery(
                        journal_id
                            .as_deref()
                            .ok_or_else(|| fail("preview_expired"))?,
                    )
                } else {
                    self.validate_configuration(view.client, &policy, configuration_target.as_ref())
                }
            };
            authority()?;
            let context = view
                .context
                .as_ref()
                .ok_or_else(|| fail("preview_expired"))?;
            self.refresh_account_cache(&context.connection_id, false)
                .await?;
            if view.client == ClientKind::Claude {
                self.validate_claude_refresh_context(
                    claude_context
                        .as_ref()
                        .ok_or_else(|| fail("claude_configuration_context_unverified"))?,
                )?;
                self.ensure_no_other_claude_recovery(
                    journal_id
                        .as_deref()
                        .ok_or_else(|| fail("preview_expired"))?,
                )?;
            } else {
                authority()?;
            }
            if view.client == ClientKind::Codex {
                self.current_connection(&context.connection_id, context.connection_revision)
                    .map_err(|_| fail("codex_catalog_scope_changed"))?;
            }
            let _ = self.list_models(&context.connection_id).await?;
            if view.client == ClientKind::Claude {
                authority()?;
            }
            self.apply_client_config(id, true, false)
        }
        .await;
        if result.is_err() {
            self.previews
                .lock()
                .map_err(|_| fail("state_unavailable"))?
                .remove(id);
        }
        result
    }
    pub fn apply_client_config(
        &self,
        id: &str,
        confirmed: bool,
        restore: bool,
    ) -> Result<ConfigResult> {
        let _operation = self.begin_mutation()?;
        if !confirmed {
            return Err(fail("confirmation_required"));
        }
        if !self.paths.writes_authorized {
            return Err(fail("integration_authorization_required"));
        }
        let entry = self.take_preview(id, if restore { "restore" } else { "apply" })?;
        if entry.view.client == ClientKind::Claude {
            return self.apply_claude_entry(entry, restore);
        }
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let guard = || {
            self.validate_configuration(
                entry.view.client,
                &entry.policy,
                entry.configuration_target.as_ref(),
            )
        };
        guard()?;
        // Compare all scoped candidates and output after refresh, under the actor lock.
        if entry.view.client == ClientKind::Codex && !restore {
            let frozen = entry
                .catalog_context
                .as_ref()
                .ok_or_else(|| fail("codex_catalog_context_unverified"))?;
            let current = (|| {
                let (record, cache) =
                    self.current_connection(&frozen.connection_id, frozen.connection_revision)?;
                let planned = self.plan_codex(&record, &cache, &frozen.default_alias)?;
                self.freeze_codex(
                    &record,
                    &cache,
                    &planned,
                    entry
                        .configuration_target
                        .as_ref()
                        .ok_or_else(|| fail("codex_configuration_target_unverified"))?,
                )
            })()
            .map_err(|_| fail("codex_catalog_scope_changed"))?;
            if current != *frozen {
                return Err(fail("codex_catalog_scope_changed"));
            }
            self.validate_codex_context(Some(frozen), false)?;
        }
        self.validate_preview_context(
            &entry.view,
            &entry.policy,
            entry.configuration_target.as_ref(),
        )?;
        let context_guard = |current: Option<&CodexContext>, restoring: bool| {
            if restoring != restore || current != entry.catalog_context.as_ref() {
                return Err(fail("codex_catalog_context_unverified"));
            }
            self.validate_codex_context(current, restoring)
        };
        let journal = self.codex_engine(&guard, &context_guard).apply(
            entry
                .journal_id
                .as_deref()
                .ok_or_else(|| fail("preview_expired"))?,
        )?;
        let revision =
            codex_configuration_revision(&self.paths.codex_config, &self.paths.codex_catalog())?;
        for client in self
            .clients
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .iter_mut()
        {
            if client.client == journal.client {
                client.configuration_state =
                    if restore { "not_configured" } else { "saved" }.into();
                client.configuration_revision = revision.clone();
                client.loaded_state = "restart_required".into();
                client.loaded_evidence = None;
                client.connection_evidence = None;
                client.connection_state = "not_tested".into();
                if restore {
                    client.manual_steps.push("配置字节已恢复；已退出、撤销或失效的旧helper授权不会复活，请重新登录并应用接入。".into());
                }
                self.store
                    .put("client-state", client.client.as_str(), "local", client)?;
            }
        }
        self.changed();
        Ok(ConfigResult {
            client: journal.client,
            state: if restore { "restored" } else { "saved" }.into(),
            loaded_state: "restart_required".into(),
            backup_group_id: Some(journal.id),
        })
    }
    pub fn export_claude_import(&self, id: &str, confirmed: bool, path: &Path) -> Result<()> {
        let _operation = self.begin_mutation()?;
        if !confirmed {
            return Err(fail("confirmation_required"));
        }
        if !self.paths.writes_authorized {
            return Err(fail("integration_authorization_required"));
        }
        validate_uuid(id)?;
        let entry = self
            .previews
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .remove(id)
            .ok_or_else(|| fail("preview_expired"))?;
        if entry.view.client != ClientKind::Claude
            || entry.view.operation != "apply"
            || !entry.view.can_export_import
            || parse_timestamp(&entry.view.expires_at)? <= chrono::Utc::now().timestamp()
        {
            return Err(fail("preview_expired"));
        }
        let destination = self.claude_diagnostic_destination(path)?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let guard = || {
            self.validate_claude_configuration(
                &entry.policy,
                entry.claude_configuration_target.as_ref(),
            )?;
            if !self.paths.helper_available() {
                return Err(fail("helper_unavailable"));
            }
            let context = entry
                .view
                .context
                .as_ref()
                .ok_or_else(|| fail("preview_expired"))?;
            let current = self.claude_diagnostic_scope(
                &context.connection_id,
                context.connection_revision,
                entry
                    .view
                    .model_id
                    .as_deref()
                    .ok_or_else(|| fail("preview_expired"))?,
            )?;
            if entry.claude_diagnostic_scope.as_ref() != Some(&current) {
                return Err(fail("connection_conflict"));
            }
            if self.claude_diagnostic_destination(path)? != destination
                || self.claude_diagnostic_destination(&destination)? != destination
            {
                return Err(fail("invalid_path"));
            }
            Ok(())
        };
        guard()?;
        self.validate_preview_context(
            &entry.view,
            &entry.policy,
            entry.configuration_target.as_ref(),
        )?;
        if !self.paths.helper_available() {
            return Err(fail("helper_unavailable"));
        }
        let (bytes, binding_id) = entry
            .import_candidate
            .ok_or_else(|| fail("preview_expired"))?;
        let mut binding = self
            .store
            .get::<HelperBinding>("binding", &binding_id)?
            .ok_or_else(|| fail("binding_disabled"))?;
        binding.enabled = true;
        guard()?;
        let _ = bound_helper_token(
            &binding,
            ClientKind::Claude,
            &binding_id,
            &self.store,
            self.vault.as_ref(),
            chrono::Utc::now().timestamp(),
        )?;
        guard()?;
        crate::config::atomic_write(&destination, &bytes)?;
        guard()?;
        self.store
            .put("binding", &binding_id, &binding.account_id, &binding)?;
        // Exporting a diagnostic candidate is not applying, loading, or observing
        // a Claude configuration. Public client evidence therefore remains absent.
        self.store.put("manual-import",&binding_id,&binding.account_id,&json!({"client":"claude","origin":binding.origin,"installation_id":binding.installation_id,"configuration_hash":sha256(&bytes),"state":"diagnostic_exported","observed_at":chrono::Utc::now().to_rfc3339()}))?;
        self.changed();
        Ok(())
    }
    pub fn preview_restore(&self, backup_id: &str) -> Result<ConfigPreview> {
        self.preview_restore_with_coordination(backup_id, None)
    }
    pub fn preview_restore_with_coordination(
        &self,
        backup_id: &str,
        coordination: Option<ClaudeRestoreCoordination>,
    ) -> Result<ConfigPreview> {
        let _operation = self.begin_mutation()?;
        validate_uuid(backup_id)?;
        let backup = self
            .store
            .get::<Journal>("backup", backup_id)?
            .or(self
                .store
                .get::<Journal>("journal", backup_id)?
                .filter(|j| ["publishing", "conflict", "committed"].contains(&j.phase.as_str())))
            .ok_or_else(|| fail("backup_invalid"))?;
        if coordination.is_some()
            && (backup.client != ClientKind::Claude
                || backup.phase != "committed"
                || !self
                    .store
                    .get::<Journal>("journal", backup_id)?
                    .is_some_and(|source| {
                        source.client == ClientKind::Claude && source.phase == "committed"
                    }))
        {
            // Client/phase rejection precedes every target observation or
            // Vault access. This mode never changes Codex restoration.
            return Err(fail("configuration_conflict"));
        }
        if backup.client == ClientKind::Claude {
            return self.preview_claude_restore(backup_id, backup, coordination);
        }
        let observation = self.observe_configuration(backup.client)?;
        let mut blocked = self
            .require_configuration(&observation, None, None)
            .err()
            .map(|error| error.code);
        if backup.client != ClientKind::Codex {
            blocked = Some("schema_unverified".into());
        }
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let install = self
            .store
            .installation(&backup.installation.origin)?
            .ok_or_else(|| fail("backup_invalid"))?;
        if install.id != backup.installation.id {
            return Err(fail("account_scope_mismatch"));
        }
        self.ensure_installation_available(&install.id)?;
        let guard = || {
            self.validate_configuration(
                backup.client,
                &observation.policy,
                observation.configuration_target.as_ref(),
            )
        };
        let context_guard =
            |context: Option<&CodexContext>, _: bool| self.validate_codex_context(context, true);
        let engine = self.codex_engine(&guard, &context_guard);
        let mut catalog_context = None;
        let journal_id = if blocked.is_none() {
            catalog_context = engine.codex_context(&backup)?;
            self.validate_codex_context(catalog_context.as_ref(), true)?;
            let mut journal = engine.prepare_restore(backup_id)?;
            engine.freeze_target(
                &mut journal,
                observation.target.version.clone(),
                observation.target.bundle_id.clone(),
            )?;
            engine.freeze_policy(&mut journal, &observation.policy.revision)?;
            Some(journal.id)
        } else {
            None
        };
        let preview = ConfigPreview {
            id: uuid(),
            client: backup.client,
            operation: "restore".into(),
            target_version: observation.target.version.clone(),
            paths: backup.paths,
            model_id: None,
            context: None,
            restore_source: Some(RestoreSource {
                backup_group_id: backup_id.into(),
                expires_at: backup.expires_at.clone(),
            }),
            restore_coordination: None,
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
            impact: vec![
                "恢复所选原账户的操作前配置；客户端仍需重启。".into(),
                "恢复不依赖当前推理 key、节点或会话；第三方修改时整组停止覆盖，失效授权不会复活。"
                    .into(),
            ],
            redacted_diff:
                "Restore encrypted original configuration; original secrets stay redacted.".into(),
            expires_at: (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339(),
            can_apply: blocked.is_none(),
            can_export_import: false,
            codex_catalog: None,
            codex_runtime: observation.runtime_view.clone(),
            codex_configuration_target: configuration_target_view(&observation),
            claude_configuration_target: claude::target_view(&observation),
            blocking_reason: blocked,
        };
        self.previews
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .insert(
                preview.id.clone(),
                PreviewEntry {
                    view: preview.clone(),
                    journal_id,
                    import_candidate: None,
                    policy: observation.policy,
                    configuration_target: observation.configuration_target,
                    claude_configuration_target: observation.claude_configuration_target,
                    claude_diagnostic_scope: None,
                    claude_context: None,
                    catalog_context,
                    _mapping_reservation: None,
                },
            );
        Ok(preview)
    }
    pub fn import_inputs(&self, id: &str, paths: &[PathBuf]) -> Result<Vec<ImageInput>> {
        let _operation = self.begin_mutation()?;
        let _permit = self.image_memory_permit()?;
        let record = self.record(id)?;
        let _ = self.current_connection(id, record.view.revision)?;
        if paths.is_empty() || paths.len() > 5 {
            return Err(fail("image_limit_exceeded"));
        }
        paths
            .iter()
            .map(|p| images::import_input(&self.store, p, &record.view))
            .collect()
    }
    pub fn use_history_as_input(
        &self,
        image_id: &str,
        id: &str,
        revision: u64,
    ) -> Result<ImageInput> {
        let _operation = self.begin_mutation()?;
        let _permit = self.image_memory_permit()?;
        let (record, _) = self.current_connection(id, revision)?;
        let (image, snapshot) = self.scoped_image(image_id)?;
        if snapshot.panel_origin != record.view.panel_origin
            || image.account_id != record.view.account_id
        {
            return Err(fail("account_scope_mismatch"));
        }
        let filename = self
            .store
            .get::<String>("image-file", image_id)?
            .ok_or_else(|| fail("local_image_missing"))?;
        let path = images::owned_image_path(&self.store, &filename)?;
        images::import_input(&self.store, &path, &record.view)
    }
    pub fn create_image_task(
        self: &Arc<Self>,
        id: &str,
        revision: u64,
        model_id: &str,
        prompt: String,
        parameters: ImageParameters,
        input_ids: Vec<String>,
    ) -> Result<TaskView> {
        let _operation = self.begin_mutation()?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let (record, cache) = self.current_connection(id, revision)?;
        let model = cache
            .models
            .iter()
            .find(|m| m.id == model_id && m.provider_id == record.view.provider_id)
            .ok_or_else(|| fail("model_unavailable"))?;
        let node_id = record
            .view
            .node_id
            .clone()
            .ok_or_else(|| fail("node_unavailable"))?;
        let node = cache
            .nodes
            .iter()
            .find(|n| n.id == node_id && n.available)
            .ok_or_else(|| fail("node_unavailable"))?;
        let (maximum_request_bytes, maximum_results) = images::validate_parameters(
            model,
            &parameters,
            &prompt,
            input_ids.len(),
            node.maximum_request_bytes,
        )?;
        let capability = if input_ids.is_empty() {
            "openai.images.generations"
        } else {
            "openai.images.edits"
        };
        if !model.capabilities.iter().any(|c| c == capability) {
            return Err(fail("unsupported_image_model"));
        }
        let active_tasks = self
            .store
            .list::<TaskView>("task", None)?
            .into_iter()
            .filter(|t| ["queued", "sending", "waiting", "saving"].contains(&t.state.as_str()))
            .count();
        let retained = self
            .received_memory
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .len();
        if active_tasks + retained >= 2 {
            return Err(fail("image_busy"));
        }
        let mut inputs = vec![];
        let mut unique = HashSet::new();
        for input_id in input_ids {
            validate_uuid(&input_id)?;
            if !unique.insert(input_id.clone()) {
                return Err(fail("invalid_input"));
            }
            let input = self
                .store
                .get::<InputRecord>("input", &input_id)?
                .ok_or_else(|| fail("local_image_missing"))?;
            if input.account_id != record.view.account_id
                || input.connection_id != record.view.id
                || input.connection_revision != revision
            {
                return Err(fail("input_scope_mismatch"));
            }
            inputs.push(input);
        }
        let task = TaskView {
            id: uuid(),
            account_id: record.view.account_id.clone(),
            connection_id: id.into(),
            connection_revision: revision,
            credential_revision: record
                .view
                .credential_revision
                .clone()
                .ok_or_else(|| fail("credential_missing"))?,
            node_id,
            kind: if inputs.is_empty() {
                "generation"
            } else {
                "edit"
            }
            .into(),
            requested_model: model_id.into(),
            prompt,
            parameters,
            state: "queued".into(),
            local_save_available: false,
            started_at: Some(chrono::Utc::now().to_rfc3339()),
            request_id: None,
            result_state: "unconfirmed".into(),
            outcome: None,
            charge_state: "unavailable".into(),
            charges: vec![],
            image_ids: vec![],
            error: None,
        };
        let snapshot = TaskSnapshot {
            task: task.clone(),
            panel_origin: record.view.panel_origin.clone(),
            edge_base_url: node.base_url.clone(),
            provider_id: record.view.provider_id.clone(),
            installation_id: record.installation_id,
            secret_reference: record
                .credential_reference
                .ok_or_else(|| fail("credential_missing"))?,
            model_revision: model.revision.clone(),
            maximum_request_bytes,
            maximum_results,
            inputs,
        };
        {
            let _permit = self.image_memory_permit()?;
            let _ = images::request_bytes(&self.store, &snapshot)?;
        }
        self.persist_task(&snapshot)?;
        let service = self.clone();
        tokio::spawn(async move {
            service.execute_image(snapshot).await;
        });
        Ok(task)
    }
    async fn execute_image(self: Arc<Self>, mut snapshot: TaskSnapshot) {
        let Ok(_permit) = self.image_workers.clone().acquire_owned().await else {
            return;
        };
        if self
            .cancelled
            .lock()
            .is_ok_and(|c| c.contains(&snapshot.task.id))
        {
            return;
        }
        let result = self.send_image_once(&mut snapshot).await;
        if let Err(error) = result {
            snapshot.task.state = if error.code == "task_cancelled" {
                "cancelled"
            } else if snapshot.task.image_ids.is_empty() && error.code != "local_save_failed" {
                if [
                    "credential_missing",
                    "binding_disabled",
                    "connection_conflict",
                    "image_limit_exceeded",
                ]
                .contains(&error.code.as_str())
                {
                    "failed"
                } else {
                    "unconfirmed"
                }
            } else {
                "save_failed"
            }
            .into();
            snapshot.task.error = Some(error);
        }
        if self
            .cancelled
            .lock()
            .is_ok_and(|c| c.contains(&snapshot.task.id))
            && !["completed", "cancelled"].contains(&snapshot.task.state.as_str())
        {
            snapshot.task.state = "unconfirmed".into();
            snapshot.task.error = Some(fail("result_unconfirmed"));
        }
        let _ = self.persist_task(&snapshot);
        let _ = self.reconcile_task(&mut snapshot).await;
    }
    async fn send_image_once(&self, snapshot: &mut TaskSnapshot) -> Result<()> {
        let record = self.record(&snapshot.task.connection_id)?;
        self.ensure_installation_available(&record.installation_id)?;
        if record.view.panel_origin != snapshot.panel_origin
            || record.view.account_id != snapshot.task.account_id
            || record.view.revision != snapshot.task.connection_revision
            || record.view.credential_revision.as_deref()
                != Some(&snapshot.task.credential_revision)
            || record.session_status != "authenticated"
        {
            return Err(fail("connection_conflict"));
        }
        let binding = HelperBinding {
            id: uuid(),
            client: ClientKind::Codex,
            origin: snapshot.panel_origin.clone(),
            account_id: snapshot.task.account_id.clone(),
            installation_id: snapshot.installation_id.clone(),
            provider_id: snapshot.provider_id.clone(),
            credential_revision: snapshot.task.credential_revision.clone(),
            secret_reference: snapshot.secret_reference.clone(),
            enabled: true,
            expires_at: record.credential_expires_at,
            authorization_epoch: record.authorization_epoch,
        };
        let token = bound_helper_token(
            &binding,
            ClientKind::Codex,
            &binding.id,
            &self.store,
            self.vault.as_ref(),
            chrono::Utc::now().timestamp(),
        )?;
        let body = {
            let _permit = self
                .image_memory_workers
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| fail("image_busy"))?;
            images::request_bytes(&self.store, snapshot)?
        };
        let mut url = self.http.validate_url(&snapshot.edge_base_url)?;
        if url.query().is_some() {
            return Err(fail("invalid_remote_url"));
        }
        let base = url.path().trim_end_matches('/');
        let target = if snapshot.task.kind == "edit" {
            "images/edits"
        } else {
            "images/generations"
        };
        url.set_path(&format!("{base}/{target}"));
        self.claim_image_send(snapshot)?;
        let observation = Arc::new(Mutex::new(None));
        let response = self
            .http
            .request_observed(
                Method::POST,
                url.as_str(),
                body,
                vec![
                    ("Authorization".into(), format!("Bearer {}", &*token)),
                    ("Content-Type".into(), "application/json".into()),
                    ("Accept".into(), "application/json".into()),
                ],
                images::MAX_RESPONSE_BYTES,
                Duration::from_secs(300),
                Some(observation.clone()),
            )
            .await;
        if let Some((_, request_id)) = observation
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .clone()
        {
            snapshot.task.request_id = request_id;
            self.persist_task(snapshot)?;
        }
        let response = response?;
        snapshot.task.request_id = response.request_id;
        snapshot.task.state = "waiting".into();
        self.persist_task(snapshot)?;
        if !(200..300).contains(&response.status) {
            return Err(fail(if (300..400).contains(&response.status) {
                "generation_redirect_rejected"
            } else {
                "generation_response_failed"
            }));
        }
        let _memory_permit = self
            .image_memory_workers
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| fail("image_busy"))?;
        let received = images::parse_response(&response.bytes, snapshot.maximum_results)?;
        snapshot.task.state = "saving".into();
        snapshot.task.local_save_available = true;
        self.persist_task(snapshot)?;
        let pending = match images::stage_received(&self.store, snapshot, &received) {
            Ok(p) => p,
            Err(_) => {
                self.received_memory
                    .lock()
                    .map_err(|_| fail("state_unavailable"))?
                    .insert(snapshot.task.id.clone(), received);
                return Err(fail("local_save_failed"));
            }
        };
        snapshot.task.image_ids = pending.iter().map(|p| p.id.clone()).collect();
        self.persist_task(snapshot)?;
        let images = images::finish_local_save(&self.store, snapshot)
            .map_err(|_| fail("local_save_failed"))?;
        snapshot.task.image_ids = images.into_iter().map(|i| i.id).collect();
        snapshot.task.state = "completed".into();
        snapshot.task.local_save_available = false;
        snapshot.task.error = None;
        Ok(())
    }
    fn claim_image_send(&self, snapshot: &mut TaskSnapshot) -> Result<()> {
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        self.ensure_mutations_available()?;
        let current = self
            .store
            .get::<TaskView>("task", &snapshot.task.id)?
            .ok_or_else(|| fail("task_unavailable"))?;
        if current.state == "cancelled"
            || self
                .cancelled
                .lock()
                .map_err(|_| fail("state_unavailable"))?
                .contains(&snapshot.task.id)
        {
            return Err(fail("task_cancelled"));
        }
        if current.state != "queued" {
            return Err(fail("task_state_conflict"));
        }
        let record = self.record(&snapshot.task.connection_id)?;
        self.ensure_installation_available(&record.installation_id)?;
        if record.view.account_id != snapshot.task.account_id
            || record.view.panel_origin != snapshot.panel_origin
            || record.view.revision != snapshot.task.connection_revision
            || record.view.credential_revision.as_deref()
                != Some(&snapshot.task.credential_revision)
            || record.session_status != "authenticated"
            || record.credential_state != "active"
            || record
                .session_expires_at
                .is_none_or(|at| at <= chrono::Utc::now().timestamp())
            || record.view.permission_state != "allowed"
            || record
                .credential_expires_at
                .is_some_and(|at| at <= chrono::Utc::now().timestamp())
            || self
                .store
                .meta(&authorization_epoch_reference(
                    &record.view.panel_origin,
                    &record.installation_id,
                    &record.view.account_id,
                ))?
                .and_then(|s| s.parse::<u64>().ok())
                != Some(record.authorization_epoch)
        {
            return Err(fail("connection_conflict"));
        }
        snapshot.task.state = "sending".into();
        self.persist_task(snapshot)
    }
    async fn reconcile_task(&self, snapshot: &mut TaskSnapshot) -> Result<()> {
        let _operation = self.begin_mutation()?;
        let lock = self.network_lock(&format!("task/{}", snapshot.task.id))?;
        let _serial = lock.lock().await;
        let Some(request_id) = &snapshot.task.request_id else {
            return Ok(());
        };
        validate_uuid(request_id)?;
        let (record, installation, session) =
            self.fresh_session(&snapshot.task.connection_id).await?;
        if record.view.panel_origin != snapshot.panel_origin
            || record.view.account_id != snapshot.task.account_id
        {
            return Err(fail("account_scope_mismatch"));
        }
        let target = format!("/api/v1/orbit/usage/records/{request_id}");
        let fact: Value = self
            .http
            .panel(
                &snapshot.panel_origin,
                &target,
                Method::GET,
                vec![],
                Some((&installation, &session, self.vault.as_ref())),
                true,
            )
            .await?;
        if fact["request_id"].as_str() != Some(request_id.as_str()) {
            return Err(fail("account_scope_mismatch"));
        }
        if fact["observation_state"] != "observed"
            || fact["record"].is_null()
            || fact["execution_status"].is_null()
        {
            return Ok(());
        }
        let mut facts = snapshot.task.clone();
        apply_server_fact(&mut facts, &fact)?;
        {
            let _gate = self
                .account_gate
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            let current = self.record(&record.view.id)?;
            self.ensure_installation_available(&record.installation_id)?;
            if current.session_status != "authenticated"
                || current.view.revision != record.view.revision
                || current.authorization_epoch != record.authorization_epoch
                || current.installation_id != record.installation_id
                || current.view.account_id != record.view.account_id
                || current.view.panel_origin != record.view.panel_origin
            {
                return Err(fail("account_scope_mismatch"));
            }
            *snapshot = self.store.merge_task_facts(snapshot, &facts)?;
        }
        self.changed();
        Ok(())
    }
    pub fn cancel_image_task(&self, id: &str) -> Result<TaskView> {
        let _operation = self.begin_mutation()?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let mut snapshot = self.scoped_task(id)?;
        if [
            "completed",
            "failed",
            "cancelled",
            "save_failed",
            "unconfirmed",
        ]
        .contains(&snapshot.task.state.as_str())
        {
            return Ok(snapshot.task);
        }
        self.cancelled
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .insert(id.into());
        snapshot.task.state = if snapshot.task.state == "queued" {
            "cancelled"
        } else {
            "unconfirmed"
        }
        .into();
        snapshot.task.error = if snapshot.task.state == "unconfirmed" {
            Some(fail("result_unconfirmed"))
        } else {
            None
        };
        self.persist_task(&snapshot)?;
        Ok(snapshot.task)
    }
    pub fn retry_local_save(&self, id: &str) -> Result<TaskView> {
        let _operation = self.begin_mutation()?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let _permit = self.image_memory_permit()?;
        let mut snapshot = self.scoped_task(id)?;
        if !["save_failed", "unconfirmed"].contains(&snapshot.task.state.as_str())
            || !snapshot.task.local_save_available
        {
            return Err(fail("task_state_conflict"));
        }
        {
            let mut received = self
                .received_memory
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            if let Some(images) = received.get(id) {
                images::stage_received(&self.store, &snapshot, images)?;
                received.remove(id);
            }
        }
        let images = images::finish_local_save(&self.store, &snapshot)?;
        snapshot.task.image_ids = images.into_iter().map(|i| i.id).collect();
        snapshot.task.state = "completed".into();
        snapshot.task.local_save_available = false;
        snapshot.task.error = None;
        self.persist_task(&snapshot)?;
        Ok(snapshot.task)
    }
    fn image_export_material(&self, id: &str) -> Result<(HistoryImage, PathBuf, String)> {
        let (image, snapshot) = self.scoped_image(id)?;
        let name = self
            .store
            .get::<String>("image-file", id)?
            .ok_or_else(|| fail("local_image_missing"))?;
        let manifest = self
            .store
            .get::<images::SaveManifest>("local-save-manifest", &image.task_id)?
            .ok_or_else(|| fail("local_image_corrupt"))?;
        let matches = manifest
            .images
            .iter()
            .filter(|item| item.id == id)
            .collect::<Vec<_>>();
        if manifest.phase != "completed"
            || manifest.task_id != image.task_id
            || manifest.account_id != image.account_id
            || manifest.panel_origin != snapshot.panel_origin
            || manifest.images.is_empty()
            || manifest.images.len() > snapshot.maximum_results as usize
            || matches.len() != 1
        {
            return Err(fail("local_image_corrupt"));
        }
        let material = matches[0];
        if material.task_id != image.task_id
            || material.account_id != image.account_id
            || material.file_name != name
            || material.mime != image.mime
            || material.width != image.width
            || material.height != image.height
            || material.bytes != image.bytes
            || material.hash.len() != 64
            || !material
                .hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(fail("local_image_corrupt"));
        }
        Ok((
            image,
            images::owned_image_path(&self.store, &name)?,
            material.hash.clone(),
        ))
    }
    pub fn begin_image_export(self: &Arc<Self>, id: &str) -> Result<ImageExportPlan> {
        // Acquire this lease before the account gate; begin_mutation uses that gate.
        // Retain both leases through decode and the clipboard write so installation
        // cannot start while a save dialog or export worker is still in flight.
        let operation = self.begin_mutation()?;
        let permit = self.image_memory_permit()?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        self.ensure_mutations_available()?;
        let record = self
            .active_record()?
            .ok_or_else(|| fail("session_expired"))?;
        self.validate_image_export_session(&record)?;
        let (image, source, expected_hash) = self.image_export_material(id)?;
        Ok(ImageExportPlan {
            service: self.clone(),
            scope: ImageExportScope {
                connection_id: record.view.id,
                connection_revision: record.view.revision,
                panel_origin: record.view.panel_origin,
                account_id: record.view.account_id,
                installation_id: record.installation_id,
                authorization_epoch: record.authorization_epoch,
            },
            image,
            source,
            expected_hash,
            _operation: operation,
            _permit: permit,
        })
    }
    fn validate_image_export_session(&self, record: &ConnectionRecord) -> Result<()> {
        self.ensure_installation_available(&record.installation_id)?;
        if record.session_status != "authenticated"
            || record
                .session_expires_at
                .is_none_or(|at| at <= chrono::Utc::now().timestamp())
        {
            return Err(fail("session_expired"));
        }
        if self
            .store
            .meta(&authorization_epoch_reference(
                &record.view.panel_origin,
                &record.installation_id,
                &record.view.account_id,
            ))?
            .and_then(|s| s.parse::<u64>().ok())
            != Some(record.authorization_epoch)
        {
            return Err(fail("connection_conflict"));
        }
        Ok(())
    }
    fn commit_image_export(
        &self,
        plan: &ImageExportPlan,
        write: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        self.ensure_mutations_available()?;
        let record = self
            .active_record()?
            .ok_or_else(|| fail("session_expired"))?;
        self.validate_image_export_session(&record)?;
        let scope = &plan.scope;
        if record.view.id != scope.connection_id
            || record.view.revision != scope.connection_revision
            || record.view.panel_origin != scope.panel_origin
            || record.view.account_id != scope.account_id
            || record.installation_id != scope.installation_id
            || record.authorization_epoch != scope.authorization_epoch
        {
            return Err(fail("connection_conflict"));
        }
        let (image, source, expected_hash) = self.image_export_material(&plan.image.id)?;
        if image.task_id != plan.image.task_id
            || image.account_id != plan.image.account_id
            || image.mime != plan.image.mime
            || image.width != plan.image.width
            || image.height != plan.image.height
            || image.bytes != plan.image.bytes
            || source != plan.source
        {
            return Err(fail("account_scope_mismatch"));
        }
        if expected_hash != plan.expected_hash {
            return Err(fail("local_image_corrupt"));
        }
        // Both native outputs write synchronously while logout, connection selection,
        // revocation and maintenance are excluded by this same account gate.
        write()
    }
    pub fn delete_history(&self, ids: &[String], confirmed: bool) -> Result<u32> {
        let _operation = self.begin_mutation()?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        if self.image_workers.available_permits() != 2 {
            return Err(fail("image_busy"));
        }
        let _files = self.store.image_files_lock()?;
        if !confirmed || ids.is_empty() || ids.len() > 100 {
            return Err(fail("confirmation_required"));
        }
        let unique = ids.iter().collect::<HashSet<_>>();
        if unique.len() != ids.len() {
            return Err(fail("invalid_identifier"));
        }
        let selected = ids
            .iter()
            .map(|id| self.scoped_image(id))
            .collect::<Result<Vec<_>>>()?;
        if selected.iter().any(|(_, s)| {
            ["queued", "sending", "waiting", "saving"].contains(&s.task.state.as_str())
        }) {
            return Err(fail("image_busy"));
        }
        for (image, mut snapshot) in selected {
            if let Some(name) = self.store.get::<String>("image-file", &image.id)? {
                let path = images::owned_image_path(&self.store, &name)?;
                if path.exists() {
                    std::fs::remove_file(path).map_err(|_| fail("local_delete_failed"))?;
                }
            }
            let preview = self
                .store
                .root
                .join("previews")
                .join(format!("{}.png", image.id));
            if preview.exists() {
                std::fs::remove_file(preview).map_err(|_| fail("local_delete_failed"))?;
            }
            self.store.delete("image", &image.id)?;
            self.store.delete("image-file", &image.id)?;
            self.store.delete("pending-image", &image.id)?;
            if let Some(mut manifest) = self
                .store
                .get::<images::SaveManifest>("local-save-manifest", &snapshot.task.id)?
            {
                if manifest.phase == "completed" {
                    manifest.images.retain(|i| i.id != image.id);
                    self.store.put(
                        "local-save-manifest",
                        &snapshot.task.id,
                        &snapshot.task.account_id,
                        &manifest,
                    )?;
                }
            }
            let task = self
                .store
                .get::<TaskView>("task", &snapshot.task.id)?
                .ok_or_else(|| fail("task_unavailable"))?;
            snapshot.task = task;
            snapshot.task.image_ids.retain(|id| id != &image.id);
            self.persist_task(&snapshot)?; /* Retain safe task metadata; deletion never changes server usage or charges. */
        }
        self.changed();
        Ok(ids.len() as u32)
    }
    pub fn reuse_task_parameters(&self, id: &str) -> Result<Value> {
        let snapshot = self.scoped_task(id)?;
        let (record, cache) = self.current_connection(
            &snapshot.task.connection_id,
            snapshot.task.connection_revision,
        )?;
        if record.view.credential_revision.as_deref() != Some(&snapshot.task.credential_revision)
            || !cache.models.iter().any(|m| {
                m.id == snapshot.task.requested_model && m.revision == snapshot.model_revision
            })
        {
            return Err(fail("connection_conflict"));
        }
        for input in &snapshot.inputs {
            let path = self.store.root.join("inputs").join(&input.file_name);
            if sha256(&images::bounded_read(&path, images::MAX_IMAGE_BYTES)?) != input.hash {
                return Err(fail("input_changed"));
            }
        }
        Ok(
            json!({"prompt":snapshot.task.prompt,"model_id":snapshot.task.requested_model,"parameters":snapshot.task.parameters,"inputs":snapshot.inputs.into_iter().map(|i|i.view).collect::<Vec<_>>()}),
        )
    }
    pub fn update_settings(&self, patch: Value) -> Result<SettingsView> {
        let _operation = self.begin_mutation()?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let mut current = serde_json::to_value(self.store.settings()?)
            .map_err(|_| fail("serialization_failed"))?;
        let object = patch.as_object().ok_or_else(|| fail("invalid_settings"))?;
        let target = current
            .as_object_mut()
            .ok_or_else(|| fail("invalid_settings"))?;
        for (key, value) in object {
            if !target.contains_key(key) {
                return Err(fail("invalid_settings"));
            }
            target.insert(key.clone(), value.clone());
        }
        let settings: SettingsView =
            serde_json::from_value(current).map_err(|_| fail("invalid_settings"))?;
        if !["system", "light", "dark"].contains(&settings.theme.as_str())
            || !["zh-CN", "en"].contains(&settings.locale.as_str())
            || !["platform_default", "keep_running", "quit"]
                .contains(&settings.close_behavior.as_str())
            || settings.close_behavior == "keep_running" && !cfg!(target_os = "macos")
            || settings.image_disk_limit_bytes < 32 * 1024 * 1024
            || settings.image_disk_limit_bytes > 64 * 1024 * 1024 * 1024
            || settings.preview_cache_limit_bytes > 1024 * 1024 * 1024
            || settings.backup_retention_days == 0
            || settings.backup_retention_days > 30
            || settings.backup_max_groups == 0
            || settings.backup_max_groups > 5
        {
            return Err(fail("invalid_settings"));
        }
        self.store.put("settings", "local", "local", &settings)?;
        for client in [ClientKind::Codex, ClientKind::Claude] {
            self.config_engine().prune_backups(client)?;
        }
        self.changed();
        Ok(settings)
    }
    pub fn prepare_diagnostics(&self) -> Result<DiagnosticsPreview> {
        let _operation = self.begin_mutation()?;
        let bootstrap = self.bootstrap()?;
        let text = format!(
            "Orivelle Orbit 0.1.0\nPlatform: {} / {}\nSchema: {}\nState revision: {}\nSession state: {}\nVault mode: {}\nConnections: {}\nLocal tasks: {}\nClient states: {}\nImages, prompts, account identifiers, URLs, tokens and configuration contents excluded.\n",
            std::env::consts::OS,
            std::env::consts::ARCH,
            bootstrap.schema_version,
            bootstrap.state_revision,
            bootstrap.session.status,
            if self.paths.isolated {
                "isolated/denied"
            } else {
                "native/fail-closed"
            },
            bootstrap.connections.len(),
            bootstrap.tasks.len(),
            bootstrap
                .clients
                .iter()
                .map(|c| format!(
                    "{}={}/{}/{}",
                    c.client.as_str(),
                    c.configuration_state,
                    c.loaded_state,
                    c.connection_state
                ))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let preview = DiagnosticsPreview {
            id: uuid(),
            text,
            contains_images: false,
            uploaded: false,
        };
        self.diagnostics
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .insert(preview.id.clone(), preview.clone());
        Ok(preview)
    }
    pub fn export_diagnostics(&self, id: &str, path: &Path) -> Result<()> {
        let _operation = self.begin_mutation()?;
        validate_uuid(id)?;
        let diagnostics = self
            .diagnostics
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .remove(id)
            .ok_or_else(|| fail("preview_expired"))?;
        crate::config::atomic_write(path, diagnostics.text.as_bytes())
    }
    fn installation_journals(&self, installation: &Installation) -> Result<Vec<Journal>> {
        let mut groups = HashMap::new();
        for kind in ["journal", "backup"] {
            for journal in self.store.list::<Journal>(kind, None)? {
                if journal.installation.id == installation.id
                    && journal.installation.origin == installation.origin
                    && (kind == "backup"
                        || ["publishing", "conflict", "committed"]
                            .contains(&journal.phase.as_str()))
                {
                    groups.insert(journal.id.clone(), journal);
                }
            }
        }
        Ok(groups.into_values().collect())
    }
    pub fn preview_forget_installation(&self, id: &str) -> Result<ForgetInstallationPreview> {
        let _operation = self.begin_mutation()?;
        validate_uuid(id)?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let installation = self
            .store
            .list::<Installation>("installation", None)?
            .into_iter()
            .find(|i| i.id == id)
            .ok_or_else(|| fail("installation_identity_mismatch"))?;
        let backup_records = self.installation_journals(&installation)?;
        let mut accounts = self
            .store
            .list::<ConnectionRecord>("connection", None)?
            .into_iter()
            .filter(|r| r.installation_id == id && r.view.panel_origin == installation.origin)
            .map(|r| r.view.label)
            .chain(backup_records.iter().map(|b| b.account_label.clone()))
            .collect::<Vec<_>>();
        accounts.sort();
        accounts.dedup();
        let mut account_ids = self
            .store
            .list::<ConnectionRecord>("connection", None)?
            .into_iter()
            .filter(|r| r.installation_id == id && r.view.panel_origin == installation.origin)
            .map(|r| r.view.account_id)
            .chain(backup_records.iter().map(|b| b.account_id.clone()))
            .collect::<Vec<_>>();
        account_ids.sort();
        account_ids.dedup();
        let mut backup_ids = backup_records
            .iter()
            .map(|b| b.id.clone())
            .collect::<Vec<_>>();
        backup_ids.sort();
        let preview = ForgetInstallationPreview {
            id: uuid(),
            panel_origin: installation.origin,
            affected_account_labels: accounts,
            backup_group_count: backup_records.len() as u32,
            consequences: vec![
                "删除此 Panel origin 的安装签名密钥、备份加密密钥和本地账户/推理凭据。".into(),
                "现有加密备份将永久失去恢复能力。".into(),
                "不撤销服务器推理 key，不修改原客户端配置，不保证缓存客户端立即退出。".into(),
            ],
        };
        self.forgets
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .insert(
                preview.id.clone(),
                ForgetEntry {
                    view: preview.clone(),
                    installation_id: id.into(),
                    account_ids,
                    backup_ids,
                    expires_at: chrono::Utc::now().timestamp() + 300,
                },
            );
        Ok(preview)
    }
    pub async fn forget_installation(
        &self,
        id: &str,
        acknowledged: bool,
        confirmed: bool,
    ) -> Result<BootstrapView> {
        let _operation = self.begin_mutation()?;
        if !confirmed || !acknowledged {
            return Err(fail("confirmation_required"));
        }
        let entry = self
            .forgets
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .remove(id)
            .ok_or_else(|| fail("preview_expired"))?;
        if entry.expires_at <= chrono::Utc::now().timestamp() {
            return Err(fail("preview_expired"));
        }
        let (installation, records) = {
            let _gate = self
                .account_gate
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            let installation = self
                .store
                .installation(&entry.view.panel_origin)?
                .ok_or_else(|| fail("installation_identity_mismatch"))?;
            if installation.id != entry.installation_id {
                return Err(fail("installation_identity_mismatch"));
            }
            let records = self
                .store
                .list::<ConnectionRecord>("connection", None)?
                .into_iter()
                .filter(|r| {
                    r.installation_id == installation.id
                        && r.view.panel_origin == entry.view.panel_origin
                })
                .collect::<Vec<_>>();
            let current_backups = self.installation_journals(&installation)?;
            let mut backup_ids = current_backups
                .iter()
                .map(|b| b.id.clone())
                .collect::<Vec<_>>();
            backup_ids.sort();
            let mut current_accounts = records
                .iter()
                .map(|r| r.view.account_id.clone())
                .chain(current_backups.iter().map(|b| b.account_id.clone()))
                .collect::<Vec<_>>();
            current_accounts.sort();
            current_accounts.dedup();
            if current_accounts != entry.account_ids || backup_ids != entry.backup_ids {
                return Err(fail("connection_conflict"));
            }
            self.store
                .set_meta(&format!("installation_forgetting/{}", installation.id), "1")?;
            self.abort_pending_locked()?;
            (installation, records)
        };
        for record in records {
            self.logout(&record.view.id).await?;
            self.store.delete("connection", &record.view.id)?;
            self.store.delete("account-cache", &record.view.id)?;
            if self.store.meta("active_connection")?.as_deref() == Some(&record.view.id) {
                self.store.set_meta("active_connection", "")?;
            }
        }
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        for mut binding in self.store.list::<HelperBinding>("binding", None)? {
            if binding.installation_id == installation.id && binding.origin == installation.origin {
                binding.enabled = false;
                self.store
                    .put("binding", &binding.id, &binding.account_id, &binding)?;
                self.vault.delete(&binding.secret_reference)?;
            }
        }
        for mapping in self
            .store
            .list::<crate::claude_mapping::MappingRecord>("claude-mapping", None)?
        {
            if mapping.context.installation_id == installation.id
                && mapping.context.panel_origin == installation.origin
            {
                let plan = mapping.plan()?;
                self.claude_mappings.stop(&plan.binding_id);
                if matches!(
                    crate::claude_mapping::verify_context_secret(&mapping, self.vault.as_ref())?,
                    crate::claude_mapping::LocalSecretPresence::Authenticated
                ) {
                    self.vault.delete(&plan.token_reference())?;
                }
                self.store.delete("claude-mapping", &plan.binding_id)?;
                self.store
                    .delete("claude-mapping-outcome", &plan.binding_id)?;
            }
        }
        // Keep recovery identity intact if a mapped secret cannot be read or
        // authenticated. All matching inference grants are already disabled.
        self.vault.delete(&installation.signing_reference)?;
        self.vault.delete(&installation.backup_reference)?;
        for kind in ["backup", "journal"] {
            for journal in self.store.list::<Journal>(kind, None)? {
                if journal.installation.id == installation.id {
                    self.store.delete(kind, &journal.id)?;
                }
            }
        }
        self.store
            .delete("installation", &sha256(installation.origin.as_bytes()))?;
        self.invalidate_evidence()?;
        self.changed();
        self.bootstrap()
    }
}

fn text(value: &Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 2048 && !v.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or_else(|| fail("invalid_panel_response"))
}
fn nullable_text(value: &Value, key: &str) -> Result<Option<String>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) if v.len() <= 2048 && !v.chars().any(char::is_control) => {
            Ok(Some(v.clone()))
        }
        _ => Err(fail("invalid_panel_response")),
    }
}
fn parse_timestamp(value: &str) -> Result<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|v| v.timestamp())
        .map_err(|_| fail("invalid_panel_response"))
}
fn decimal(value: Option<&Value>) -> Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v))
            if v.len() <= 80
                && v.bytes()
                    .all(|b| b.is_ascii_digit() || b == b'.' || b == b'-')
                && v.bytes().any(|b| b.is_ascii_digit()) =>
        {
            Ok(Some(v.clone()))
        }
        _ => Err(fail("invalid_panel_response")),
    }
}
fn map_price(value: &Value, reference: bool) -> Result<Option<PriceView>> {
    if value.is_null() {
        return Ok(None);
    }
    let status = value["status"].as_str().unwrap_or("unknown");
    let status = match status {
        "configured" => "configured",
        "partial" if reference => "partial",
        "unpriced_free" if !reference => "unpriced_free",
        "unavailable" => "unavailable",
        _ => "unknown",
    };
    let source = if reference {
        match value["source"].as_str() {
            Some("official") => "official",
            Some("operator_estimate") => "operator_estimate",
            Some("configured_reference") => "configured_reference",
            _ => "unknown",
        }
    } else {
        "sales"
    };
    let mut rates = vec![];
    if let Some(meters) = value["meters"].as_array() {
        for meter in meters {
            let name = text(meter, "meter")?;
            if ![
                "input",
                "cached_input",
                "cache_write",
                "output",
                "reasoning",
                "request",
                "image",
                "text_input",
                "cached_text_input",
                "image_input",
                "cached_image_input",
                "image_output",
            ]
            .contains(&name.as_str())
            {
                return Err(fail("invalid_panel_response"));
            }
            let unit_size = meter["unit_size"]
                .as_u64()
                .filter(|n| *n > 0)
                .ok_or_else(|| fail("invalid_panel_response"))?;
            rates.push(PriceRate {
                meter: name,
                unit_size,
                unit_price: decimal(meter.get("unit_price"))?,
            });
        }
    }
    Ok(Some(PriceView {
        currency: nullable_text(value, "currency")?,
        source: source.into(),
        status: status.into(),
        rates,
    }))
}
fn map_model(value: &Value, revision: &str) -> Result<ModelView> {
    let id = text(value, "id")?;
    let original_category = text(value, "model_category")?;
    let image = original_category == "image";
    let category = if image { "image" } else { "text" };
    let capabilities = value["capabilities"]
        .as_array()
        .ok_or_else(|| fail("invalid_panel_response"))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| fail("invalid_panel_response"))
        })
        .collect::<Result<Vec<_>>>()?;
    let reasoning = &value["reasoning"];
    let efforts = if reasoning["capability_status"] == "supported" {
        let supported = reasoning["supported_efforts"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some(allowed) = reasoning["allowed_efforts"].as_array() {
            supported
                .into_iter()
                .filter(|s| allowed.iter().any(|a| a.as_str() == Some(s)))
                .collect()
        } else {
            supported
        }
    } else {
        vec![]
    };
    let constraints = if image && !value["image_parameters"].is_null() {
        Some(
            serde_json::from_value::<ImageParameterConstraints>(value["image_parameters"].clone())
                .map_err(|_| fail("invalid_panel_response"))?,
        )
    } else {
        None
    };
    Ok(ModelView {
        codex_catalog_binding: crate::codex_binding::normalize_binding(
            value.get("codex_catalog_binding"),
            &id,
            &original_category,
            &capabilities,
        ),
        reasoning: crate::codex_binding::normalize_reasoning(value.get("reasoning")),
        id,
        label: text(value, "display_name")?,
        provider_id: text(value, "provider_id")?,
        revision: sha256(
            format!(
                "{revision}\n{}",
                serde_json::to_string(value).map_err(|_| fail("invalid_panel_response"))?
            )
            .as_bytes(),
        ),
        category: category.into(),
        capabilities,
        execution_profile: nullable_text(value, "execution_profile")?,
        image_billing_mode: nullable_text(value, "image_billing_mode")?,
        reasoning_efforts: efforts,
        support_status: if constraints.as_ref().is_some_and(|c| c.verified) {
            "verified"
        } else {
            "unverified"
        }
        .into(),
        image_parameters: constraints,
        sales_price: map_price(&value["sales_price"], false)?,
        official_price: map_price(&value["reference_price"], true)?,
    })
}
fn map_account_cache(value: &Value, record: &ConnectionRecord) -> Result<AccountCache> {
    let account = &value["account"];
    if text(account, "id")? != record.view.account_id
        || text(&value["meta"], "account_id")? != record.view.account_id
        || text(&value["meta"], "installation_id")? != record.installation_id
    {
        return Err(fail("account_scope_mismatch"));
    }
    let member = text(account, "kind")? == "member";
    let currency = nullable_text(&value["wallet"], "currency")?;
    let wallet = if member || value["wallet"].is_null() {
        None
    } else {
        Some(Money {
            amount: decimal(value["wallet"].get("amount"))?,
            currency,
            status: "known".into(),
        })
    };
    let budget = if value["budget"].is_null() {
        None
    } else {
        let b = &value["budget"];
        let mode = text(b, "mode")?;
        let currency = nullable_text(b, "currency")?;
        Some(BudgetView {
            available: Money {
                amount: decimal(b.get("remaining"))?,
                currency: currency.clone(),
                status: if mode == "unlimited" {
                    "unlimited"
                } else if mode == "unconfigured" {
                    "unknown"
                } else {
                    "known"
                }
                .into(),
            },
            limit: Some(Money {
                amount: decimal(b.get("monthly_limit"))?,
                currency,
                status: if mode == "unlimited" {
                    "unlimited"
                } else {
                    "known"
                }
                .into(),
            }),
            mode,
            period: nullable_text(b, "period_started_at")?,
        })
    };
    let revision = text(&value["meta"], "catalog_revision")?;
    let nodes = value["nodes"]
        .as_array()
        .ok_or_else(|| fail("invalid_panel_response"))?
        .iter()
        .map(|node| {
            let base = text(node, "connection_url")?;
            let url = url::Url::parse(&base).map_err(|_| fail("invalid_panel_response"))?;
            if url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
                || url.query().is_some()
            {
                return Err(fail("invalid_panel_response"));
            }
            let maximum = node["maximum_request_bytes"]
                .as_u64()
                .filter(|n| *n > 0)
                .unwrap_or(0)
                .min(images::MAX_REQUEST_BYTES);
            let policy =
                node["maximum_request_bytes_source"].as_str() == Some("panel_admission_policy");
            let capabilities = node["capabilities"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            Ok(NodeView {
                id: text(node, "id")?,
                label: text(node, "name")?,
                base_url: base.clone(),
                available: policy
                    && maximum > 0
                    && matches!(
                        node["service_state"].as_str(),
                        Some("available" | "degraded")
                    ),
                maximum_request_bytes: maximum,
                capabilities,
                revision:sha256(serde_json::to_string(&json!({"id":node["id"],"url":base,"claude_base_url":node["claude_base_url"],"state":node["service_state"],"maximum_request_bytes":maximum,"maximum_request_bytes_source":node["maximum_request_bytes_source"],"capabilities":node["capabilities"],"capability_source":node["capability_source"]})).map_err(|_|fail("invalid_panel_response"))?.as_bytes()),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let models = value["models"]
        .as_array()
        .ok_or_else(|| fail("invalid_panel_response"))?
        .iter()
        .map(|v| map_model(v, &revision))
        .collect::<Result<Vec<_>>>()?;
    Ok(AccountCache {
        account: AccountView {
            id: record.view.account_id.clone(),
            label: text(account, "name")?,
            email: nullable_text(account, "email")?,
            updated_at: None,
            member,
            wallet,
            budget,
        },
        nodes,
        models,
        catalog_revision: revision,
        claude_endpoints: value["nodes"]
            .as_array()
            .ok_or_else(|| fail("invalid_panel_response"))?
            .iter()
            .filter_map(|node| {
                let id = node["id"].as_str()?;
                let base = node["claude_base_url"].as_str()?;
                let url = url::Url::parse(base).ok()?;
                (url.scheme() == "https"
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none())
                .then(|| (id.into(), base.into()))
            })
            .collect(),
        inference_allowed: value["inference_credential_allowed"]
            .as_bool()
            .unwrap_or(false),
        keys: value["inference_keys"]
            .as_array()
            .ok_or_else(|| fail("invalid_panel_response"))?
            .iter()
            .map(|k| {
                Ok(KeyMetadata {
                    provider_id: text(k, "provider_id")?,
                    api_key_id: nullable_text(k, "api_key_id")?,
                    key_revision: nullable_text(k, "key_revision")?,
                    state: text(k, "state")?,
                    expires_at: nullable_text(k, "expires_at")?
                        .as_deref()
                        .map(parse_timestamp)
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
    })
}
fn apply_server_fact(task: &mut TaskView, fact: &Value) -> Result<()> {
    if fact["request_id"].as_str() != task.request_id.as_deref() {
        return Err(fail("account_scope_mismatch"));
    }
    if fact["observation_state"] != "observed"
        || fact["execution_status"].is_null()
        || fact["record"].is_null()
    {
        return Ok(());
    }
    let record = &fact["record"];
    let correlation = &fact["correlation"];
    if correlation["lookup_request_id"].as_str() != task.request_id.as_deref()
        || correlation["actor_id"].as_str() != Some(&task.account_id)
        || correlation["representative_request_id"] != record["request_id"]
        || !matches!(
            correlation["kind"].as_str(),
            Some("exact" | "logical_chain")
        )
        || (correlation["kind"] == "exact"
            && record["request_id"].as_str() != task.request_id.as_deref())
        || (correlation["kind"] == "logical_chain"
            && correlation["logical_request_id"] != record["logical_request_id"])
    {
        return Err(fail("invalid_panel_response"));
    }
    let result = text(record, "result_state")?;
    let charge = text(record, "charge_state")?;
    if ![
        "historical",
        "confirmed",
        "incomplete",
        "conflict",
        "unconfirmed",
    ]
    .contains(&result.as_str())
        || ![
            "incomplete",
            "reconciliation_pending",
            "charged",
            "recorded",
            "pending",
            "unbilled",
            "unavailable",
        ]
        .contains(&charge.as_str())
    {
        return Err(fail("invalid_panel_response"));
    }
    let mut charges = vec![];
    for entry in record["charges"]
        .as_array()
        .ok_or_else(|| fail("invalid_panel_response"))?
    {
        charges.push(Charge {
            amount: decimal(entry.get("amount"))?,
            currency: nullable_text(entry, "currency")?,
            request_id: nullable_text(entry, "request_id")?,
        });
    }
    task.result_state = result;
    task.charge_state = charge;
    task.outcome = nullable_text(record, "outcome")?;
    task.charges = charges;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    mod macos_config_roundtrip;
    #[cfg(target_os = "macos")]
    mod panel_acceptance;
    use crate::{
        identity::{ProofInput, canonical_proof},
        vault::FakeVault,
    };
    const GOLDEN: &str = include_str!("../tests/fixtures/public-wire.json");
    fn golden() -> Value {
        assert_eq!(
            sha256(GOLDEN.as_bytes()),
            "507077fe227e5ea6fedfc8b03381a74c3c5b6b066aa908a29042b963e7f60289"
        );
        serde_json::from_str(GOLDEN).unwrap()
    }
    pub(super) struct PolicyVault {
        inner: Arc<FakeVault>,
        pub(super) calls: AtomicU64,
        pub(super) mutations: AtomicU64,
        pub(super) pause:
            Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
    }
    impl Vault for PolicyVault {
        fn get(&self, reference: &str) -> Result<zeroize::Zeroizing<Vec<u8>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let pause = self.pause.lock().unwrap().take();
            if let Some((entered, resume)) = pause {
                entered.send(()).map_err(|_| fail("synthetic_barrier"))?;
                resume
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .map_err(|_| fail("synthetic_barrier"))?;
            }
            self.inner.get(reference)
        }
        fn get_optional(&self, reference: &str) -> Result<Option<zeroize::Zeroizing<Vec<u8>>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let pause = self.pause.lock().unwrap().take();
            if let Some((entered, resume)) = pause {
                entered.send(()).map_err(|_| fail("synthetic_barrier"))?;
                resume
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .map_err(|_| fail("synthetic_barrier"))?;
            }
            self.inner.get_optional(reference)
        }
        fn put(&self, reference: &str, bytes: &[u8]) -> Result<()> {
            self.mutations.fetch_add(1, Ordering::SeqCst);
            self.inner.put(reference, bytes)
        }
        fn create_claude_mapping_secret(&self, binding_id: &str, bytes: &[u8]) -> Result<()> {
            self.mutations.fetch_add(1, Ordering::SeqCst);
            self.inner.create_claude_mapping_secret(binding_id, bytes)
        }
        fn delete(&self, reference: &str) -> Result<()> {
            self.mutations.fetch_add(1, Ordering::SeqCst);
            self.inner.delete(reference)
        }
    }
    pub(super) struct PolicyFixture {
        _dir: tempfile::TempDir,
        pub(super) service: Arc<Service>,
        pub(super) vault: Arc<PolicyVault>,
        pub(super) record: ConnectionRecord,
        pub(super) epoch: Arc<AtomicU64>,
        pub(super) model: String,
        client: ClientKind,
        pub(super) target: Arc<AtomicU64>,
    }
    impl PolicyFixture {
        fn preview(&self) -> ConfigPreview {
            self.service
                .preview_client_config(
                    &self.record.view.id,
                    self.record.view.revision,
                    self.client,
                    &self.model,
                )
                .unwrap()
        }
        fn diagnostic_preview(&self) -> ConfigPreview {
            self.service
                .preview_claude_diagnostic(
                    &self.record.view.id,
                    self.record.view.revision,
                    &self.model,
                )
                .unwrap()
        }
        fn reset(&self) {
            self.vault.calls.store(0, Ordering::SeqCst);
            self.vault.mutations.store(0, Ordering::SeqCst);
        }
        fn bindings(&self) -> Value {
            serde_json::to_value(
                self.service
                    .store
                    .list::<HelperBinding>("binding", None)
                    .unwrap(),
            )
            .unwrap()
        }
    }
    pub(super) fn policy_fixture(client: ClientKind) -> PolicyFixture {
        let (dir, original, vault, mut record, install) = setup();
        original
            .accept_credential(
                &mut record,
                &install,
                serde_json::from_value(golden()["credential_reply"]["data"].clone()).unwrap(),
            )
            .unwrap();
        let mut paths = original.paths.clone();
        paths.writes_authorized = true;
        let home = paths
            .codex_config
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        paths.codex_user_home = Some(home.clone());
        paths.codex_config = home.join(".codex/config.toml");
        std::fs::create_dir(paths.codex_config.parent().unwrap()).unwrap();
        std::fs::write(&paths.codex_config, b"model='old'\n").unwrap();
        let helper = b"synthetic helper fixture never executed";
        std::fs::write(&paths.helper, helper).unwrap();
        paths.helper_expected_sha256 = Some(sha256(helper));
        let mut clients = original.clients.lock().unwrap().clone();
        let target = clients.iter_mut().find(|c| c.client == client).unwrap();
        target.installed = true;
        target.platform_supported = true;
        target.version = Some("2.19675.0-synthetic".into());
        target.bundle_id = Some(format!("synthetic.{}", client.as_str()));
        if client == ClientKind::Claude {
            target.version = Some(crate::claude_config::SUPPORTED_APP_VERSION.into());
            target.bundle_id = Some("com.anthropic.claudefordesktop".into());
            target.app_path = Some("synthetic-installed-Claude.app".into());
        }
        target.managed = "no".into();
        paths.fixture_clients = Some(clients);
        let epoch = Arc::new(AtomicU64::new(0));
        let signal = epoch.clone();
        paths.fixture_policy = Some(Arc::new(move |target| {
            let value = signal.load(Ordering::SeqCst);
            crate::policy::Snapshot::fixture_user_config(
                target,
                match value {
                    2 => crate::policy::State::Yes,
                    3 => crate::policy::State::Unknown,
                    _ => crate::policy::State::No,
                },
                &format!("synthetic-source-{value}"),
            )
        }));
        let target = Arc::new(AtomicU64::new(0));
        if client == ClientKind::Claude {
            qualify_claude_paths(&mut paths, target.clone());
        }
        let target_signal = target.clone();
        let target_paths = paths.clone();
        paths.fixture_configuration_target =
            Some(Arc::new(move |client, _full| {
                match target_signal.load(Ordering::SeqCst) {
                    2 => Err(fail("codex_configuration_target_unverified")),
                    value => crate::system::CodexConfigurationTarget::synthetic(
                        &target_paths,
                        client,
                        &format!("fixture-{value}"),
                    ),
                }
            }));
        let vault = Arc::new(PolicyVault {
            inner: vault,
            calls: AtomicU64::new(0),
            mutations: AtomicU64::new(0),
            pause: Mutex::new(None),
        });
        let service = Service::new(original.store.clone(), vault.clone(), paths).unwrap();
        let mut cache = service.account_cache(&record).unwrap();
        cache.models[0]
            .capabilities
            .extend(["openai.responses".into(), "anthropic.messages".into()]);
        if client == ClientKind::Claude {
            // Existing transaction tests exercise the direct Claude-shaped
            // contract. Mapping tests explicitly choose a non-Claude model.
            cache.models[0].id = "claude-sonnet-5".into();
        }
        // The immutable old wire is still a compatibility input. Only this
        // positive synthetic factory adds the newly required declarations.
        cache.models[0].codex_catalog_binding = Some(CodexCatalogBindingView::Declared {
            verified: false,
            profile_id: crate::codex_binding::PROFILE_ID.into(),
            cli_version: crate::codex_catalog::SUPPORTED_CLI_VERSION.into(),
            source_sha256: crate::codex_catalog::SOURCE_SHA256.into(),
            template_slug: "gpt-5.5".into(),
        });
        cache.models[0].reasoning = Some(ReasoningPolicyView {
            state: "restricted".into(),
            capability_status: "supported".into(),
            supported_efforts: vec!["low".into(), "medium".into(), "high".into()],
            allowed_efforts: Some(vec!["medium".into(), "high".into()]),
            default_effort: Some("medium".into()),
            revision: Some(1),
        });
        let model = cache.models[0].id.clone();
        service
            .store
            .put(
                "account-cache",
                &record.view.id,
                &record.view.account_id,
                &cache,
            )
            .unwrap();
        PolicyFixture {
            _dir: dir,
            service,
            vault,
            record,
            epoch,
            model,
            client,
            target,
        }
    }
    fn qualify_claude_paths(paths: &mut SystemPaths, signal: Arc<AtomicU64>) {
        let home = paths.codex_user_home.clone().unwrap_or_else(|| {
            paths
                .codex_config
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf()
        });
        std::fs::create_dir_all(home.join("Library/Application Support")).unwrap();
        let root = home.join("Library/Application Support/Claude-3p");
        paths.claude_library = root.join("configLibrary");
        paths.configuration_roots.push(root.clone());
        paths.fixture_claude_configuration_target = Some(Arc::new(move |target, _full| {
            if signal.load(Ordering::SeqCst) == 2
                || !target.installed
                || !target.platform_supported
                || target.client != ClientKind::Claude
                || target.version.as_deref() != Some(crate::claude_config::SUPPORTED_APP_VERSION)
                || target.bundle_id.as_deref() != Some("com.anthropic.claudefordesktop")
            {
                return Err(fail("claude_configuration_target_unverified"));
            }
            let mut receipt = crate::system::ClaudeConfigurationTarget::synthetic(root.clone())?;
            receipt.revision = sha256(
                format!("synthetic-Claude-target-{}", signal.load(Ordering::SeqCst)).as_bytes(),
            );
            Ok(receipt)
        }));
    }
    fn catalog_menu(f: &PolicyFixture) {
        let mut cache = f.service.account_cache(&f.record).unwrap();
        let mut menu = cache.models[0].clone();
        menu.id = "menu-alias".into();
        menu.revision = sha256(b"synthetic menu revision");
        menu.reasoning.as_mut().unwrap().default_effort = Some("high".into());
        let mut unbound = menu.clone();
        unbound.id = "unbound-alias".into();
        unbound.codex_catalog_binding = None;
        let mut foreign = menu.clone();
        foreign.id = "foreign-provider".into();
        foreign.provider_id = "anthropic".into();
        cache.models.extend([menu, unbound, foreign]);
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
    fn prepared_catalog(f: &PolicyFixture, preview: &ConfigPreview) -> Journal {
        let previews = f.service.previews.lock().unwrap();
        let id = previews
            .get(&preview.id)
            .unwrap()
            .journal_id
            .as_ref()
            .unwrap();
        f.service.store.get("journal", id).unwrap().unwrap()
    }
    #[test]
    fn desktop_config_flow_bootstrap_and_confirmation_share_qualification_without_running_runtime()
    {
        let f = policy_fixture(ClientKind::Codex);
        let mut paths = f.service.paths.clone();
        let receipt_paths = paths.clone();
        let full_reads = Arc::new(AtomicU64::new(0));
        let count = full_reads.clone();
        paths.fixture_configuration_target = Some(Arc::new(move |client, full| {
            if full {
                count.fetch_add(1, Ordering::SeqCst);
            }
            crate::system::CodexConfigurationTarget::synthetic(&receipt_paths, client, "fixture-0")
        }));
        let service = Service::new(f.service.store.clone(), f.vault.clone(), paths).unwrap();
        let first = service.bootstrap().unwrap();
        let second = service.bootstrap().unwrap();
        let target = first
            .clients
            .iter()
            .find(|client| client.client == ClientKind::Codex)
            .unwrap();
        assert_eq!(target.configuration_access, ConfigurationAccess::Allowed);
        assert_eq!(target.managed, "unknown");
        assert_eq!(
            target.configuration_access_revision,
            second
                .clients
                .iter()
                .find(|client| client.client == ClientKind::Codex)
                .unwrap()
                .configuration_access_revision
        );
        assert_eq!(
            full_reads.load(Ordering::SeqCst),
            0,
            "bootstrap cannot perform full signature/content verification"
        );
        let preview = service
            .preview_client_config(
                &f.record.view.id,
                f.record.view.revision,
                ClientKind::Codex,
                &f.model,
            )
            .unwrap();
        assert!(preview.can_apply);
        assert!(full_reads.load(Ordering::SeqCst) > 0);
        assert_eq!(preview.codex_runtime.as_ref().unwrap().state, "unverified");
        assert_eq!(
            Some(
                &preview
                    .codex_configuration_target
                    .as_ref()
                    .unwrap()
                    .revision
            ),
            target.configuration_access_revision.as_ref()
        );
        let saved = service
            .apply_client_config(&preview.id, true, false)
            .unwrap();
        assert_eq!(
            (saved.state.as_str(), saved.loaded_state.as_str()),
            ("saved", "restart_required")
        );
        let current = service
            .bootstrap()
            .unwrap()
            .clients
            .into_iter()
            .find(|client| client.client == ClientKind::Codex)
            .unwrap();
        assert_eq!(
            current.configuration_access_revision,
            target.configuration_access_revision
        );
        assert_eq!(current.managed, "unknown");
        assert!(current.loaded_evidence.is_none() && current.connection_evidence.is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn desktop_config_flow_first_root_is_created_only_by_confirmed_joint_journal() {
        use std::os::unix::fs::PermissionsExt;
        let f = policy_fixture(ClientKind::Codex);
        let root = f.service.paths.codex_config.parent().unwrap();
        std::fs::remove_file(&f.service.paths.codex_config).unwrap();
        std::fs::remove_dir(root).unwrap();
        let before = f
            .service
            .bootstrap()
            .unwrap()
            .clients
            .into_iter()
            .find(|client| client.client == ClientKind::Codex)
            .unwrap()
            .configuration_access_revision;
        let preview = f.preview();
        assert!(preview.can_apply && !root.exists());
        assert_eq!(
            f.service
                .apply_client_config(&preview.id, false, false)
                .unwrap_err()
                .code,
            "confirmation_required"
        );
        assert!(!root.exists());
        let saved = f
            .service
            .apply_client_config(&preview.id, true, false)
            .unwrap();
        assert!(
            f.service.paths.codex_config.is_file() && f.service.paths.codex_catalog().is_file()
        );
        assert_eq!(
            std::fs::metadata(root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            f.service
                .bootstrap()
                .unwrap()
                .clients
                .into_iter()
                .find(|client| client.client == ClientKind::Codex)
                .unwrap()
                .configuration_access_revision,
            before
        );
        f.service.commit_logout(&f.record.view.id).unwrap();
        let restore = f
            .service
            .preview_restore(saved.backup_group_id.as_ref().unwrap())
            .unwrap();
        assert!(restore.can_apply && restore.codex_configuration_target.is_some());
        f.service
            .apply_client_config(&restore.id, true, true)
            .unwrap();
        assert!(
            !f.service.paths.codex_config.exists() && !f.service.paths.codex_catalog().exists()
        );
        assert!(root.is_dir());
        assert!(
            f.service
                .store
                .list::<HelperBinding>("binding", None)
                .unwrap()
                .iter()
                .all(|binding| !binding.enabled)
        );
    }

    #[test]
    fn desktop_config_flow_legacy_runtime_context_never_becomes_a_target_receipt() {
        let f = policy_fixture(ClientKind::Codex);
        let preview = f.preview();
        let mut journal = prepared_catalog(&f, &preview);
        let engine = f.service.config_engine();
        let context = engine.codex_context(&journal).unwrap().unwrap();
        let mut legacy = serde_json::to_value(&context).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("configuration_target");
        legacy["runtime"] = json!({"cli_version":crate::codex_catalog::SUPPORTED_CLI_VERSION,"revision":"a".repeat(64)});
        let decoded: CodexContext = serde_json::from_value(legacy).unwrap();
        assert!(decoded.configuration_target.is_none());
        engine
            .synthetic_without_configuration_target(&mut journal)
            .unwrap();
        journal.phase = "publishing".into();
        f.service
            .store
            .put("journal", &journal.id, &journal.account_id, &journal)
            .unwrap();
        let revision = f.service.store.snapshot().unwrap().revision;
        let bindings = f.bindings();
        f.reset();
        assert_eq!(
            f.service.preview_restore(&journal.id).unwrap_err().code,
            "codex_catalog_context_unverified"
        );
        assert_eq!(f.service.store.snapshot().unwrap().revision, revision);
        assert_eq!(f.bindings(), bindings);
        assert_eq!(
            std::fs::read(&f.service.paths.codex_config).unwrap(),
            b"model='old'\n"
        );
        assert!(!f.service.paths.codex_catalog().exists());
    }

    #[test]
    fn desktop_config_flow_unknown_profile_root_and_local_sources_block_before_mutations() {
        for mode in ["profile", "root", "target", "local-unknown", "managed"] {
            let f = policy_fixture(ClientKind::Codex);
            let mut paths = f.service.paths.clone();
            if mode == "local-unknown" {
                f.epoch.store(3, Ordering::SeqCst);
            } else if mode == "managed" {
                f.epoch.store(2, Ordering::SeqCst);
            } else {
                let target_paths = paths.clone();
                paths.fixture_configuration_target = Some(Arc::new(move |client, _| {
                    if mode == "target" {
                        return Err(fail("codex_configuration_target_unverified"));
                    }
                    let mut target = crate::system::CodexConfigurationTarget::synthetic(
                        &target_paths,
                        client,
                        "fixture-0",
                    )?;
                    if mode == "profile" {
                        target.profile_id = "unsupported".into();
                    } else {
                        target.config_root = target.config_root.with_file_name("other-root");
                    }
                    Ok(target)
                }));
            }
            let service = Service::new(f.service.store.clone(), f.vault.clone(), paths).unwrap();
            let revision = service.store.snapshot().unwrap().revision;
            let bindings = f.bindings();
            f.reset();
            assert!(service.configuration_preflight(ClientKind::Codex).is_err());
            let preview = service
                .preview_client_config(
                    &f.record.view.id,
                    f.record.view.revision,
                    ClientKind::Codex,
                    &f.model,
                )
                .unwrap();
            assert!(!preview.can_apply);
            assert_eq!(service.store.snapshot().unwrap().revision, revision);
            assert_eq!(f.bindings(), bindings);
            assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0);
            assert_eq!(f.vault.mutations.load(Ordering::SeqCst), 0);
            assert_eq!(
                std::fs::read(&service.paths.codex_config).unwrap(),
                b"model='old'\n"
            );
            assert!(!service.paths.codex_catalog().exists());
        }
    }
    #[test]
    fn codex_config_joint_menu_patch_revision_and_single_use() {
        let f = policy_fixture(ClientKind::Codex);
        catalog_menu(&f);
        std::fs::write(&f.service.paths.codex_config,
            b"model='old'\nmodel_reasoning_effort='ultra'\nplan_mode_reasoning_effort='low'\n[features]\ncustom=true\n").unwrap();
        let preview = f.preview();
        assert!(preview.can_apply);
        let summary = preview.codex_catalog.as_ref().unwrap();
        assert_eq!(summary.aliases, [f.model.clone(), "menu-alias".into()]);
        assert_eq!(summary.default_alias, f.model);
        assert_eq!(summary.default_reasoning_effort, "medium");
        assert!(preview.redacted_diff.contains("model_reasoning_effort"));
        assert!(preview.redacted_diff.contains("model_catalog_json"));
        assert!(!preview.redacted_diff.contains("instructions_template"));
        let journal = prepared_catalog(&f, &preview);
        let files = f.service.config_engine().synthetic_files(&journal).unwrap();
        assert_eq!(
            journal.paths,
            [
                f.service.paths.codex_catalog().display().to_string(),
                f.service.paths.codex_config.display().to_string()
            ]
        );
        assert!(!f.service.paths.codex_catalog().exists());
        let result = f
            .service
            .apply_client_config(&preview.id, true, false)
            .unwrap();
        for file in &files {
            assert_eq!(read_file(Path::new(&file.path)).unwrap(), file.after);
        }
        let catalog: Value = serde_json::from_slice(files[0].after.as_ref().unwrap()).unwrap();
        let models = catalog["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["slug"], f.model);
        assert_eq!(models[1]["slug"], "menu-alias");
        assert_eq!(models[1]["default_reasoning_level"], "high");
        assert_eq!(
            models[1]["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|option| option["effort"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["medium", "high"]
        );
        let document = std::str::from_utf8(files[1].after.as_ref().unwrap())
            .unwrap()
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        assert_eq!(document["model_reasoning_effort"].as_str(), Some("medium"));
        assert_eq!(document["plan_mode_reasoning_effort"].as_str(), Some("low"));
        assert_eq!(document["features"]["custom"].as_bool(), Some(true));
        assert_eq!(result.backup_group_id, Some(journal.id));
        assert_eq!(
            f.service
                .apply_client_config(&preview.id, true, false)
                .unwrap_err()
                .code,
            "preview_expired"
        );
        let saved = f
            .service
            .bootstrap()
            .unwrap()
            .clients
            .into_iter()
            .find(|client| client.client == ClientKind::Codex)
            .unwrap();
        let snapshot_revision = f.service.store.snapshot().unwrap().revision;
        std::fs::write(f.service.paths.codex_catalog(), b"external catalog").unwrap();
        let changed = f
            .service
            .bootstrap()
            .unwrap()
            .clients
            .into_iter()
            .find(|client| client.client == ClientKind::Codex)
            .unwrap();
        assert_ne!(changed.configuration_revision, saved.configuration_revision);
        assert_eq!(changed.configuration_state, "conflict");
        assert_eq!(changed.loaded_state, "unconfirmed");
        assert_eq!(
            f.service.store.snapshot().unwrap().revision,
            snapshot_revision
        );
    }
    #[test]
    fn codex_config_full_menu_and_actor_changes_reject_before_vault() {
        for change in [
            "menu-revision",
            "menu-removed",
            "menu-binding",
            "menu-reasoning",
            "node",
            "provider",
            "account",
            "epoch",
            "active",
        ] {
            let f = policy_fixture(ClientKind::Codex);
            catalog_menu(&f);
            let preview = f.preview();
            let bindings = f.bindings();
            let mut cache = f.service.account_cache(&f.record).unwrap();
            let mut record = f.record.clone();
            let menu = cache
                .models
                .iter_mut()
                .find(|model| model.id == "menu-alias")
                .unwrap();
            match change {
                "menu-revision" => menu.revision = sha256(b"late revision"),
                "menu-removed" => cache.models.retain(|model| model.id != "menu-alias"),
                "menu-binding" => menu.codex_catalog_binding = None,
                "menu-reasoning" => {
                    menu.reasoning.as_mut().unwrap().default_effort = Some("medium".into())
                }
                "node" => {
                    cache
                        .nodes
                        .iter_mut()
                        .find(|node| Some(&node.id) == record.view.node_id.as_ref())
                        .unwrap()
                        .revision = sha256(b"late node")
                }
                "provider" => record.view.provider_id = "anthropic".into(),
                "account" => record.view.account_id = "other-actor".into(),
                "epoch" => f
                    .service
                    .store
                    .set_meta(
                        &authorization_epoch_reference(
                            &record.view.panel_origin,
                            &record.installation_id,
                            &record.view.account_id,
                        ),
                        "999",
                    )
                    .unwrap(),
                "active" => f.service.store.set_meta("active_connection", "").unwrap(),
                _ => unreachable!(),
            }
            f.service
                .store
                .put(
                    "account-cache",
                    &record.view.id,
                    &record.view.account_id,
                    &cache,
                )
                .unwrap();
            f.service
                .store
                .put(
                    "connection",
                    &record.view.id,
                    &record.view.account_id,
                    &record,
                )
                .unwrap();
            f.reset();
            assert_eq!(
                f.service
                    .apply_client_config(&preview.id, true, false)
                    .unwrap_err()
                    .code,
                "codex_catalog_scope_changed",
                "{change}"
            );
            assert_eq!(
                std::fs::read(&f.service.paths.codex_config).unwrap(),
                b"model='old'\n"
            );
            assert!(!f.service.paths.codex_catalog().exists());
            assert_eq!(f.bindings(), bindings);
            assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0, "{change}");
            assert_eq!(f.vault.mutations.load(Ordering::SeqCst), 0);
        }
    }
    #[tokio::test]
    async fn codex_config_target_and_policy_gate_precede_command_refresh_and_service_mutations() {
        for gate in ["target", "policy"] {
            let f = policy_fixture(ClientKind::Codex);
            if gate == "target" {
                f.target.store(2, Ordering::SeqCst);
            } else {
                f.epoch.store(3, Ordering::SeqCst);
            }
            let expected = if gate == "target" {
                "codex_configuration_target_unverified"
            } else {
                "managed_policy_unverified"
            };
            let revision = f.service.store.snapshot().unwrap().revision;
            let bindings = f.bindings();
            f.reset();
            assert_eq!(
                f.service
                    .preview_client_config_refreshed(
                        &f.record.view.id,
                        f.record.view.revision,
                        ClientKind::Codex,
                        &f.model
                    )
                    .await
                    .unwrap_err()
                    .code,
                expected
            );
            let preview = f.preview();
            assert_eq!(preview.blocking_reason.as_deref(), Some(expected));
            assert!(!preview.can_apply);
            assert_eq!(f.service.store.snapshot().unwrap().revision, revision);
            assert_eq!(f.bindings(), bindings);
            assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0);
            assert_eq!(f.vault.mutations.load(Ordering::SeqCst), 0);
            assert!(!f.service.paths.codex_catalog().exists());
        }
        for runtime in [1, 2] {
            let f = policy_fixture(ClientKind::Codex);
            let preview = f.preview();
            let bindings = f.bindings();
            let revision = f.service.store.snapshot().unwrap().revision;
            f.target.store(runtime, Ordering::SeqCst);
            f.reset();
            assert_eq!(
                f.service
                    .apply_client_config_refreshed(&preview.id, true)
                    .await
                    .unwrap_err()
                    .code,
                if runtime == 1 {
                    "codex_configuration_target_changed"
                } else {
                    "codex_configuration_target_unverified"
                }
            );
            assert_eq!(f.service.store.snapshot().unwrap().revision, revision);
            assert_eq!(f.bindings(), bindings);
            assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0);
            f.target.store(0, Ordering::SeqCst);
            assert_eq!(
                f.service
                    .apply_client_config_refreshed(&preview.id, true)
                    .await
                    .unwrap_err()
                    .code,
                "preview_expired"
            );
            assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0);
        }
    }
    #[tokio::test]
    async fn codex_config_bootstrap_refresh_rejects_late_node_permission_and_key_changes() {
        for change in ["node-url", "node-state", "permission", "key"] {
            let f = policy_fixture(ClientKind::Codex);
            let mut wire = golden()["bootstrap_reply"].clone();
            let model = &f.service.account_cache(&f.record).unwrap().models[0];
            wire["data"]["models"][0]["codex_catalog_binding"] =
                serde_json::to_value(&model.codex_catalog_binding).unwrap();
            wire["data"]["models"][0]["reasoning"] =
                serde_json::to_value(&model.reasoning).unwrap();
            let stage = Arc::new(AtomicU64::new(0));
            let remote_stage = stage.clone();
            let calls = Arc::new(Mutex::new(Vec::new()));
            let requests = calls.clone();
            f.service.http.scripted(Arc::new(move |request| {
                let mut reply = wire.clone();
                let is_bootstrap = request.url.ends_with("/api/v1/orbit/bootstrap");
                requests.lock().unwrap().push(if is_bootstrap { "bootstrap" } else { "models" });
                let response = if is_bootstrap {
                    if remote_stage.load(Ordering::SeqCst) == 1 {
                        match change {
                            "node-url" => reply["data"]["nodes"][0]["connection_url"] = json!("https://new-edge.example.test/v1"),
                            "node-state" => reply["data"]["nodes"][0]["service_state"] = json!("unavailable"),
                            "permission" => reply["data"]["inference_credential_allowed"] = json!(false),
                            "key" => reply["data"]["inference_keys"][0]["state"] = json!("revoked"),
                            _ => unreachable!(),
                        }
                    }
                    reply
                } else {
                    assert!(request.url.contains("/api/v1/orbit/models?node_id="));
                    json!({"data": reply["data"]["models"], "meta": {"revision": reply["data"]["meta"]["catalog_revision"]}})
                };
                Box::pin(async move { Ok(crate::network::RemoteResponse { status: 200, request_id: None,
                    bytes: serde_json::to_vec(&response).unwrap() }) })
            }));
            let preview = f
                .service
                .preview_client_config_refreshed(
                    &f.record.view.id,
                    f.record.view.revision,
                    ClientKind::Codex,
                    &f.model,
                )
                .await
                .unwrap();
            assert!(preview.can_apply);
            let journal = prepared_catalog(&f, &preview);
            let bindings = f.bindings();
            stage.store(1, Ordering::SeqCst);
            assert_eq!(
                f.service
                    .apply_client_config_refreshed(&preview.id, true)
                    .await
                    .unwrap_err()
                    .code,
                "codex_catalog_scope_changed",
                "{change}"
            );
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                ["bootstrap", "models", "bootstrap"]
            );
            assert_eq!(
                std::fs::read(&f.service.paths.codex_config).unwrap(),
                b"model='old'\n"
            );
            assert!(!f.service.paths.codex_catalog().exists());
            assert_eq!(f.bindings(), bindings);
            let after: Journal = f
                .service
                .store
                .get("journal", &journal.id)
                .unwrap()
                .unwrap();
            assert_eq!(after.phase, "prepared");
            assert_eq!(after.ciphertext, journal.ciphertext);
            assert!(
                f.service
                    .store
                    .list::<Journal>("backup", None)
                    .unwrap()
                    .is_empty()
            );
            stage.store(0, Ordering::SeqCst);
            assert_eq!(
                f.service
                    .apply_client_config_refreshed(&preview.id, true)
                    .await
                    .unwrap_err()
                    .code,
                "preview_expired"
            );
            assert_eq!(calls.lock().unwrap().len(), 3);
        }
    }
    #[test]
    fn codex_config_target_or_menu_late_during_vault_read_stops_first_file_write() {
        for late in ["target", "menu"] {
            let f = policy_fixture(ClientKind::Codex);
            catalog_menu(&f);
            let preview = f.preview();
            let bindings = f.bindings();
            let (entered, waiting) = std::sync::mpsc::channel();
            let (resume, released) = std::sync::mpsc::channel();
            *f.vault.pause.lock().unwrap() = Some((entered, released));
            let worker = f.service.clone();
            let thread =
                std::thread::spawn(move || worker.apply_client_config(&preview.id, true, false));
            waiting.recv_timeout(Duration::from_secs(5)).unwrap();
            if late == "target" {
                f.target.store(1, Ordering::SeqCst);
            } else {
                let mut cache = f.service.account_cache(&f.record).unwrap();
                cache
                    .models
                    .iter_mut()
                    .find(|model| model.id == "menu-alias")
                    .unwrap()
                    .revision = sha256(b"after vault began");
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
            resume.send(()).unwrap();
            assert_eq!(
                thread.join().unwrap().unwrap_err().code,
                if late == "target" {
                    "codex_configuration_target_changed"
                } else {
                    "codex_catalog_scope_changed"
                }
            );
            assert_eq!(
                std::fs::read(&f.service.paths.codex_config).unwrap(),
                b"model='old'\n"
            );
            assert!(!f.service.paths.codex_catalog().exists());
            assert_eq!(f.bindings(), bindings);
        }
    }
    #[test]
    fn codex_config_context_change_after_pair_publish_never_enables_final_grant() {
        for late in ["target", "menu"] {
            let f = policy_fixture(ClientKind::Codex);
            catalog_menu(&f);
            let mut paths = f.service.paths.clone();
            let config = paths.codex_config.clone();
            if late == "target" {
                let receipt_paths = paths.clone();
                paths.fixture_configuration_target = Some(Arc::new(move |client, _| {
                    crate::system::CodexConfigurationTarget::synthetic(
                        &receipt_paths,
                        client,
                        if std::fs::read(&config).unwrap() == b"model='old'\n" {
                            "fixture-0"
                        } else {
                            "late-target"
                        },
                    )
                }));
            } else {
                let store = f.service.store.clone();
                let id = f.record.view.id.clone();
                let account = f.record.view.account_id.clone();
                let once = AtomicBool::new(false);
                paths.fixture_policy = Some(Arc::new(move |target| {
                    if std::fs::read(&config).unwrap() != b"model='old'\n"
                        && !once.swap(true, Ordering::SeqCst)
                    {
                        let mut cache: AccountCache =
                            store.get("account-cache", &id).unwrap().unwrap();
                        cache
                            .models
                            .iter_mut()
                            .find(|model| model.id == "menu-alias")
                            .unwrap()
                            .revision = sha256(b"late final grant scope");
                        store.put("account-cache", &id, &account, &cache).unwrap();
                    }
                    crate::policy::Snapshot::fixture_user_config(
                        target,
                        crate::policy::State::No,
                        "synthetic-source-0",
                    )
                }));
            }
            let service = Service::new(f.service.store.clone(), f.vault.clone(), paths).unwrap();
            let preview = service
                .preview_client_config(
                    &f.record.view.id,
                    f.record.view.revision,
                    ClientKind::Codex,
                    &f.model,
                )
                .unwrap();
            let bindings = serde_json::to_value(
                service
                    .store
                    .list::<HelperBinding>("binding", None)
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                service
                    .apply_client_config(&preview.id, true, false)
                    .unwrap_err()
                    .code,
                if late == "target" {
                    "codex_configuration_target_changed"
                } else {
                    "codex_catalog_scope_changed"
                }
            );
            assert_ne!(
                std::fs::read(&service.paths.codex_config).unwrap(),
                b"model='old'\n"
            );
            assert!(service.paths.codex_catalog().exists());
            assert_eq!(
                serde_json::to_value(
                    service
                        .store
                        .list::<HelperBinding>("binding", None)
                        .unwrap()
                )
                .unwrap(),
                bindings
            );
            assert!(
                service
                    .store
                    .list::<HelperBinding>("binding", None)
                    .unwrap()
                    .iter()
                    .all(|binding| !binding.enabled)
            );
            assert!(
                service
                    .pending_configuration_recovery(ClientKind::Codex)
                    .unwrap()
            );
            assert!(
                service
                    .store
                    .list::<Journal>("backup", None)
                    .unwrap()
                    .is_empty()
            );
        }
    }
    #[test]
    fn codex_config_old_null_declaration_and_profile_overrides_have_no_credential_side_effects() {
        for mode in ["old-null", "null-default", "profile", "profile-table"] {
            let f = policy_fixture(ClientKind::Codex);
            if ["old-null", "null-default"].contains(&mode) {
                let mut cache = f.service.account_cache(&f.record).unwrap();
                if mode == "old-null" {
                    cache.models[0].codex_catalog_binding = None;
                } else {
                    cache.models[0].reasoning.as_mut().unwrap().default_effort = None;
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
            } else {
                std::fs::write(
                    &f.service.paths.codex_config,
                    if mode == "profile" {
                        "profile='custom'\n[profiles.custom]\nmodel='other'\n"
                    } else {
                        "[profiles.custom]\nmodel_reasoning_effort='ultra'\n"
                    },
                )
                .unwrap();
            }
            let before = std::fs::read(&f.service.paths.codex_config).unwrap();
            let revision = f.service.store.snapshot().unwrap().revision;
            f.reset();
            let error = f
                .service
                .preview_client_config(
                    &f.record.view.id,
                    f.record.view.revision,
                    ClientKind::Codex,
                    &f.model,
                )
                .unwrap_err();
            assert_eq!(
                error.code,
                match mode {
                    "old-null" => "codex_binding_unavailable",
                    "null-default" => "codex_reasoning_unavailable",
                    _ => "codex_config_override_unverified",
                }
            );
            assert_eq!(f.service.store.snapshot().unwrap().revision, revision);
            assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0);
            assert_eq!(f.vault.mutations.load(Ordering::SeqCst), 0);
            assert_eq!(
                std::fs::read(&f.service.paths.codex_config).unwrap(),
                before
            );
            assert!(!f.service.paths.codex_catalog().exists());
        }
    }
    #[test]
    fn codex_config_restore_original_pair_after_logout_never_revives_prior_grant() {
        let f = policy_fixture(ClientKind::Codex);
        catalog_menu(&f);
        let first = f
            .service
            .apply_client_config(&f.preview().id, true, false)
            .unwrap();
        let original_config = std::fs::read(&f.service.paths.codex_config).unwrap();
        let original_catalog = std::fs::read(f.service.paths.codex_catalog()).unwrap();
        let second = f
            .service
            .apply_client_config(&f.preview().id, true, false)
            .unwrap();
        assert_ne!(first.backup_group_id, second.backup_group_id);
        f.service.commit_logout(&f.record.view.id).unwrap();
        let restore = f
            .service
            .preview_restore(second.backup_group_id.as_ref().unwrap())
            .unwrap();
        assert!(restore.can_apply);
        f.service
            .apply_client_config(&restore.id, true, true)
            .unwrap();
        assert_eq!(
            std::fs::read(&f.service.paths.codex_config).unwrap(),
            original_config
        );
        assert_eq!(
            std::fs::read(f.service.paths.codex_catalog()).unwrap(),
            original_catalog
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
    #[test]
    fn codex_config_unverified_recovery_has_no_vault_or_store_side_effects() {
        for gate in ["target", "policy"] {
            let f = policy_fixture(ClientKind::Codex);
            let mut journal = prepared_catalog(&f, &f.preview());
            journal.phase = "publishing".into();
            f.service
                .store
                .put("journal", &journal.id, &journal.account_id, &journal)
                .unwrap();
            if gate == "target" {
                f.target.store(2, Ordering::SeqCst);
            } else {
                f.epoch.store(3, Ordering::SeqCst);
            }
            let revision = f.service.store.snapshot().unwrap().revision;
            f.reset();
            f.service.coordinate_configuration_recovery().unwrap();
            assert_eq!(f.service.store.snapshot().unwrap().revision, revision);
            assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0);
            assert_eq!(f.vault.mutations.load(Ordering::SeqCst), 0);
            assert!(!f.service.paths.codex_catalog().exists());
            assert_eq!(
                std::fs::read(&f.service.paths.codex_config).unwrap(),
                b"model='old'\n"
            );
        }
    }
    #[test]
    fn changed_or_unknown_policy_rejects_apply_before_vault_files_or_binding_changes() {
        for epoch in [1, 2, 3] {
            let f = policy_fixture(ClientKind::Codex);
            let preview = f.preview();
            assert!(preview.can_apply);
            let bindings = f.bindings();
            f.reset();
            f.epoch.store(epoch, Ordering::SeqCst);
            assert_eq!(
                f.service
                    .apply_client_config(&preview.id, true, false)
                    .unwrap_err()
                    .code,
                "managed_policy_changed"
            );
            assert_eq!(
                std::fs::read(&f.service.paths.codex_config).unwrap(),
                b"model='old'\n"
            );
            assert_eq!(f.bindings(), bindings);
            assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0);
            assert_eq!(f.vault.mutations.load(Ordering::SeqCst), 0);
            assert!(
                f.service
                    .store
                    .list::<Journal>("backup", None)
                    .unwrap()
                    .is_empty()
            );
        }
    }
    #[test]
    fn policy_change_while_vault_read_is_blocked_prevents_first_apply_or_export_write() {
        for client in [ClientKind::Codex, ClientKind::Claude] {
            let f = policy_fixture(client);
            let preview = if client == ClientKind::Claude {
                f.diagnostic_preview()
            } else {
                f.preview()
            };
            assert!(if client == ClientKind::Codex {
                preview.can_apply
            } else {
                preview.can_export_import
            });
            let target = f.service.store.root.join("synthetic-diagnostic.json");
            let worker_target = target.clone();
            let bindings = f.bindings();
            f.reset();
            let (entered, waiting) = std::sync::mpsc::channel();
            let (resume, released) = std::sync::mpsc::channel();
            *f.vault.pause.lock().unwrap() = Some((entered, released));
            let worker = f.service.clone();
            let thread = std::thread::spawn(move || {
                if client == ClientKind::Codex {
                    worker
                        .apply_client_config(&preview.id, true, false)
                        .map(|_| ())
                } else {
                    worker.export_claude_import(&preview.id, true, &worker_target)
                }
            });
            waiting
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            f.epoch.store(1, Ordering::SeqCst);
            resume.send(()).unwrap();
            assert_eq!(
                thread.join().unwrap().unwrap_err().code,
                "managed_policy_changed"
            );
            assert_eq!(
                std::fs::read(&f.service.paths.codex_config).unwrap(),
                b"model='old'\n"
            );
            assert!(!target.exists());
            assert_eq!(f.bindings(), bindings);
            assert_eq!(f.vault.calls.load(Ordering::SeqCst), 1);
            assert_eq!(f.vault.mutations.load(Ordering::SeqCst), 0);
        }
    }
    #[test]
    fn restore_and_claude_export_recheck_the_frozen_policy_before_any_secret_access() {
        let f = policy_fixture(ClientKind::Codex);
        let preview = f.preview();
        let result = f
            .service
            .apply_client_config(&preview.id, true, false)
            .unwrap();
        let after = std::fs::read(&f.service.paths.codex_config).unwrap();
        let restore = f
            .service
            .preview_restore(result.backup_group_id.as_ref().unwrap())
            .unwrap();
        assert!(restore.can_apply);
        let bindings = f.bindings();
        f.reset();
        f.epoch.store(1, Ordering::SeqCst);
        assert_eq!(
            f.service
                .apply_client_config(&restore.id, true, true)
                .unwrap_err()
                .code,
            "managed_policy_changed"
        );
        assert_eq!(std::fs::read(&f.service.paths.codex_config).unwrap(), after);
        assert_eq!(f.bindings(), bindings);
        assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0);
        let f = policy_fixture(ClientKind::Claude);
        let preview = f.diagnostic_preview();
        assert!(preview.can_export_import);
        let target = f.service.store.root.join("synthetic-diagnostic.json");
        std::fs::write(&target, b"keep original").unwrap();
        let bindings = f.bindings();
        f.reset();
        f.epoch.store(2, Ordering::SeqCst);
        assert_eq!(
            f.service
                .export_claude_import(&preview.id, true, &target)
                .unwrap_err()
                .code,
            "managed_policy_changed"
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"keep original");
        assert_eq!(f.bindings(), bindings);
        assert_eq!(f.vault.calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn policy_arriving_after_file_publish_never_enables_the_final_helper_grant() {
        let f = policy_fixture(ClientKind::Codex);
        let before = f.bindings();
        let mut paths = f.service.paths.clone();
        let path = paths.codex_config.clone();
        paths.fixture_policy = Some(Arc::new(move |target| {
            let changed = std::fs::read(&path).unwrap() != b"model='old'\n";
            crate::policy::Snapshot::fixture_user_config(
                target,
                crate::policy::State::No,
                if changed {
                    "policy-arrived"
                } else {
                    "synthetic-source-0"
                },
            )
        }));
        let service = Service::new(f.service.store.clone(), f.vault.clone(), paths).unwrap();
        let preview = service
            .preview_client_config(
                &f.record.view.id,
                f.record.view.revision,
                ClientKind::Codex,
                &f.model,
            )
            .unwrap();
        assert!(preview.can_apply);
        let bindings = serde_json::to_value(
            service
                .store
                .list::<HelperBinding>("binding", None)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            service
                .apply_client_config(&preview.id, true, false)
                .unwrap_err()
                .code,
            "managed_policy_changed"
        );
        assert_ne!(
            std::fs::read(&service.paths.codex_config).unwrap(),
            b"model='old'\n"
        );
        assert_eq!(
            serde_json::to_value(
                service
                    .store
                    .list::<HelperBinding>("binding", None)
                    .unwrap()
            )
            .unwrap(),
            bindings
        );
        assert!(
            service
                .store
                .list::<HelperBinding>("binding", None)
                .unwrap()
                .iter()
                .all(|b| !b.enabled)
        );
        assert!(
            service
                .pending_configuration_recovery(ClientKind::Codex)
                .unwrap()
        );
        assert!(before.as_array().unwrap().is_empty());
    }
    #[tokio::test]
    async fn production_panel_origin_rejects_legacy_login_and_saved_account_before_vault() {
        let mut fixture = policy_fixture(ClientKind::Claude);
        let http = Http::new().unwrap();
        let calls = Arc::new(AtomicU64::new(0));
        let sent = calls.clone();
        http.scripted(Arc::new(move |_| {
            sent.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(fail("unexpected_transport")) })
        }));
        // Weak mapper ownership must not change this production-origin fixture.
        let mut service = Arc::try_unwrap(fixture.service)
            .unwrap_or_else(|_| panic!("fixture service must be uniquely owned"));
        service.http = http;
        fixture.service = Arc::new_cyclic(|self_reference| {
            service.self_reference = self_reference.clone();
            service
        });
        fixture.vault.calls.store(0, Ordering::SeqCst);
        fixture.vault.mutations.store(0, Ordering::SeqCst);
        let revision = fixture.service.store.snapshot().unwrap().revision;
        assert_eq!(
            fixture
                .service
                .start_login(&fixture.record.view.panel_origin)
                .await
                .unwrap_err()
                .code,
            "panel_origin_not_allowed"
        );
        assert_eq!(
            fixture.service.session(&fixture.record).err().unwrap().code,
            "panel_origin_not_allowed"
        );
        assert_eq!(
            fixture
                .service
                .refresh_account(&fixture.record.view.id, fixture.record.view.revision)
                .await
                .unwrap_err()
                .code,
            "panel_origin_not_allowed"
        );
        assert_eq!(
            fixture
                .service
                .list_models(&fixture.record.view.id)
                .await
                .unwrap_err()
                .code,
            "panel_origin_not_allowed"
        );
        assert_eq!(fixture.vault.calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.vault.mutations.load(Ordering::SeqCst), 0);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.service.store.snapshot().unwrap().revision, revision);
    }

    fn setup() -> (
        tempfile::TempDir,
        Arc<Service>,
        Arc<FakeVault>,
        ConnectionRecord,
        Installation,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("data")).unwrap());
        let vault = Arc::new(FakeVault::default());
        let paths = SystemPaths::isolated(&store.root).unwrap();
        let service = Service::new(store.clone(), vault.clone(), paths).unwrap();
        let wire = golden();
        let install = Installation {
            id: wire["start_request"]["installation_id"]
                .as_str()
                .unwrap()
                .into(),
            origin: wire["issuer"].as_str().unwrap().into(),
            public_key: wire["start_request"]["public_key"].as_str().unwrap().into(),
            signing_reference: "installation/11111111-1111-4111-8111-111111111111/ed25519".into(),
            backup_reference: "fixture-backup".into(),
        };
        vault.put(&install.backup_reference, &[42; 32]).unwrap();
        vault.put(&install.signing_reference,&serde_json::to_vec(&json!({"origin":install.origin,"installation_id":install.id,"seed":URL_SAFE_NO_PAD.encode((0u8..32).collect::<Vec<_>>())})).unwrap()).unwrap();
        store
            .put(
                "installation",
                &sha256(install.origin.as_bytes()),
                &install.origin,
                &install,
            )
            .unwrap();
        let response: TokenResponse =
            serde_json::from_value(wire["token_reply"]["data"].clone()).unwrap();
        let (account, session) = response.into_session(&install).unwrap();
        let record = ConnectionRecord {
            view: ConnectionView {
                id: uuid(),
                label: "fixture".into(),
                panel_origin: install.origin.clone(),
                account_id: account.clone(),
                node_id: Some("01ARZ3NDEKTSV4RRFFQ69G5FAY".into()),
                node_label: Some("Synthetic node".into()),
                edge_base_url: Some("https://edge.example.test/v1".into()),
                provider_id: "openai".into(),
                revision: 1,
                credential_revision: None,
                permission_state: "allowed".into(),
            },
            installation_id: install.id.clone(),
            session_reference: "fixture-session".into(),
            credential_reference: None,
            session_status: "authenticated".into(),
            expires_at: session.expires_at,
            session_expires_at: Some(parse_timestamp(&session.session_expires_at).unwrap()),
            refresh_attempt: None,
            credential_expires_at: None,
            credential_state: "unavailable".into(),
            api_key_id: None,
            authorization_epoch: 1,
            cleanup_references: vec![],
        };
        store
            .put("connection", &record.view.id, &account, &record)
            .unwrap();
        store
            .set_meta("active_connection", &record.view.id)
            .unwrap();
        store
            .set_meta(
                &authorization_epoch_reference(&install.origin, &install.id, &account),
                "1",
            )
            .unwrap();
        service.save_session(&record, session).unwrap();
        let cache = map_account_cache(&wire["bootstrap_reply"]["data"], &record).unwrap();
        store
            .put("account-cache", &record.view.id, &account, &cache)
            .unwrap();
        (dir, service, vault, record, install)
    }
    fn task(record: &ConnectionRecord) -> TaskView {
        TaskView {
            id: uuid(),
            account_id: record.view.account_id.clone(),
            connection_id: record.view.id.clone(),
            connection_revision: record.view.revision,
            credential_revision: record
                .view
                .credential_revision
                .clone()
                .unwrap_or_else(|| "f".repeat(64)),
            node_id: record.view.node_id.clone().unwrap(),
            kind: "generation".into(),
            requested_model: "synthetic-image".into(),
            prompt: "synthetic prompt".into(),
            parameters: ImageParameters::default(),
            state: "completed".into(),
            local_save_available: false,
            started_at: None,
            request_id: Some("33333333-3333-4333-8333-333333333333".into()),
            result_state: "unconfirmed".into(),
            outcome: None,
            charge_state: "unavailable".into(),
            charges: vec![],
            image_ids: vec![],
            error: None,
        }
    }
    fn task_snapshot(record: &ConnectionRecord) -> TaskSnapshot {
        TaskSnapshot {
            task: task(record),
            panel_origin: record.view.panel_origin.clone(),
            edge_base_url: record.view.edge_base_url.clone().unwrap(),
            provider_id: record.view.provider_id.clone(),
            installation_id: record.installation_id.clone(),
            secret_reference: record
                .credential_reference
                .clone()
                .unwrap_or_else(|| "synthetic-credential".into()),
            model_revision: "synthetic-model-revision".into(),
            maximum_request_bytes: images::MAX_REQUEST_BYTES,
            maximum_results: 1,
            inputs: vec![],
        }
    }
    fn export_png(width: u32, height: u32, value: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new_with_quality(
            &mut bytes,
            image::codecs::png::CompressionType::Uncompressed,
            image::codecs::png::FilterType::NoFilter,
        );
        image::ImageEncoder::write_image(
            encoder,
            &vec![value; width as usize * height as usize * 4],
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        bytes
    }
    fn clipboard_fixture(service: &Service, record: &ConnectionRecord) -> HistoryImage {
        let bytes = export_png(2, 2, 0);
        let mut snapshot = task_snapshot(record);
        service.persist_task(&snapshot).unwrap();
        let received = images::ReceivedImage {
            id: uuid(),
            info: images::validate_image(&bytes).unwrap(),
            bytes,
        };
        images::stage_received(&service.store, &snapshot, &[received]).unwrap();
        let image = images::finish_local_save(&service.store, &snapshot)
            .unwrap()
            .remove(0);
        snapshot.task.image_ids = vec![image.id.clone()];
        service.persist_task(&snapshot).unwrap();
        image
    }
    fn export_target(service: &Service, existing: bool) -> PathBuf {
        let directory = service.store.root.join("export-fixture");
        std::fs::create_dir_all(&directory).unwrap();
        let target = directory.join("result.png");
        if existing {
            std::fs::write(&target, b"existing fixture content").unwrap();
        }
        target
    }
    #[test]
    fn image_save_commits_exact_received_bytes_once_and_releases_dialog_lease() {
        for existing in [false, true] {
            let (_dir, service, _vault, record, _install) = setup();
            let image = clipboard_fixture(&service, &record);
            let target = export_target(&service, existing);
            let plan = service.begin_image_export(&image.id).unwrap();
            assert_eq!(
                service.begin_maintenance().err().unwrap().code,
                "update_busy"
            );
            let read = plan.read().unwrap();
            assert_eq!(
                service.begin_maintenance().err().unwrap().code,
                "update_busy"
            );
            read.save(&target).unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), export_png(2, 2, 0));
            assert_eq!(
                std::fs::read_dir(target.parent().unwrap()).unwrap().count(),
                1
            );
            assert_eq!(service.image_memory_workers.available_permits(), 2);
            drop(service.begin_maintenance().unwrap());
            // Cancelling a save dialog drops its single-use plan without output.
            let cancelled = service.begin_image_export(&image.id).unwrap();
            assert_eq!(
                service.begin_maintenance().err().unwrap().code,
                "update_busy"
            );
            drop(cancelled);
            drop(service.begin_maintenance().unwrap());
        }
    }
    #[test]
    fn image_save_read_barrier_rejects_late_scope_changes_before_any_target_write() {
        for existing in [false, true] {
            for change in [
                "logout",
                "account",
                "origin-same-id",
                "revision",
                "reauthorized",
                "forgetting",
                "maintenance",
            ] {
                let (_dir, service, _vault, mut record, _install) = setup();
                let image = clipboard_fixture(&service, &record);
                let target = export_target(&service, existing);
                let plan = service.begin_image_export(&image.id).unwrap();
                let (read_tx, read_rx) = std::sync::mpsc::sync_channel(0);
                let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
                let output = target.clone();
                let worker = std::thread::spawn(move || {
                    let read = plan.read().unwrap();
                    read_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    read.save(&output)
                });
                read_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                let expected = match change {
                    "logout" => {
                        service.commit_logout(&record.view.id).unwrap();
                        "session_expired"
                    }
                    "account" => {
                        let mut other = record.clone();
                        other.view.id = uuid();
                        other.view.account_id = "another-account".into();
                        let mut cache = service.account_cache(&record).unwrap();
                        cache.account.id = other.view.account_id.clone();
                        service
                            .store
                            .put("connection", &other.view.id, &other.view.account_id, &other)
                            .unwrap();
                        service
                            .store
                            .put(
                                "account-cache",
                                &other.view.id,
                                &other.view.account_id,
                                &cache,
                            )
                            .unwrap();
                        service
                            .store
                            .set_meta(
                                &authorization_epoch_reference(
                                    &other.view.panel_origin,
                                    &other.installation_id,
                                    &other.view.account_id,
                                ),
                                "1",
                            )
                            .unwrap();
                        service.select_connection(&other.view.id).unwrap();
                        "connection_conflict"
                    }
                    "revision" => {
                        service
                            .select_node(
                                &record.view.id,
                                record.view.node_id.as_ref().unwrap(),
                                record.view.revision,
                            )
                            .unwrap();
                        "connection_conflict"
                    }
                    _ => {
                        let _gate = service.account_gate.lock().unwrap();
                        match change {
                            "origin-same-id" => {
                                record.view.panel_origin =
                                    "https://other-panel.example.test".into();
                                record.installation_id = uuid();
                                service
                                    .store
                                    .set_meta(
                                        &authorization_epoch_reference(
                                            &record.view.panel_origin,
                                            &record.installation_id,
                                            &record.view.account_id,
                                        ),
                                        "1",
                                    )
                                    .unwrap();
                                service
                                    .store
                                    .put(
                                        "connection",
                                        &record.view.id,
                                        &record.view.account_id,
                                        &record,
                                    )
                                    .unwrap();
                                "connection_conflict"
                            }
                            "reauthorized" => {
                                record.authorization_epoch =
                                    service.advance_authorization_epoch(&record).unwrap();
                                service
                                    .store
                                    .put(
                                        "connection",
                                        &record.view.id,
                                        &record.view.account_id,
                                        &record,
                                    )
                                    .unwrap();
                                "connection_conflict"
                            }
                            "forgetting" => {
                                service
                                    .store
                                    .set_meta(
                                        &format!(
                                            "installation_forgetting/{}",
                                            record.installation_id
                                        ),
                                        "1",
                                    )
                                    .unwrap();
                                "installation_forgetting"
                            }
                            "maintenance" => {
                                service.maintenance_active.store(true, Ordering::SeqCst);
                                "update_busy"
                            }
                            _ => unreachable!(),
                        }
                    }
                };
                release_tx.send(()).unwrap();
                assert_eq!(
                    worker.join().unwrap().unwrap_err().code,
                    expected,
                    "{change}/{existing}"
                );
                if existing {
                    assert_eq!(std::fs::read(&target).unwrap(), b"existing fixture content");
                } else {
                    assert!(!target.exists());
                }
                // No transient plaintext target is left by a rejected commit.
                assert_eq!(
                    std::fs::read_dir(target.parent().unwrap()).unwrap().count(),
                    usize::from(existing)
                );
                assert_eq!(service.image_memory_workers.available_permits(), 2);
                assert_eq!(service.mutation_count.load(Ordering::SeqCst), 0);
            }
        }
    }
    #[test]
    fn image_exports_bind_completed_manifest_and_reject_same_length_valid_replacement() {
        for output in ["copy", "save"] {
            for replacement in [export_png(2, 2, 255), export_png(3, 3, 255)] {
                let (_dir, service, _vault, record, _install) = setup();
                let image = clipboard_fixture(&service, &record);
                let plan = service.begin_image_export(&image.id).unwrap();
                let actual = images::validate_image(&replacement).unwrap();
                if actual.width == 2 {
                    assert_eq!(replacement.len() as u64, image.bytes);
                }
                assert_eq!(actual.mime, image.mime);
                std::fs::write(&plan.source, &replacement).unwrap();
                let mut sink = FakeImageClipboard::default();
                let target = export_target(&service, false);
                let result = if output == "copy" {
                    plan.decode().and_then(|decoded| decoded.commit(&mut sink))
                } else {
                    plan.read().and_then(|read| read.save(&target))
                };
                assert_eq!(result.unwrap_err().code, "local_image_corrupt");
                assert!(sink.frames.is_empty());
                assert!(!target.exists());
            }
        }
        for defect in ["missing", "phase", "duplicate", "metadata"] {
            let (_dir, service, _vault, record, _install) = setup();
            let image = clipboard_fixture(&service, &record);
            let mut manifest = service
                .store
                .get::<images::SaveManifest>("local-save-manifest", &image.task_id)
                .unwrap()
                .unwrap();
            if defect == "missing" {
                service
                    .store
                    .delete("local-save-manifest", &image.task_id)
                    .unwrap();
            } else {
                match defect {
                    "phase" => manifest.phase = "received".into(),
                    "duplicate" => manifest.images.push(manifest.images[0].clone()),
                    "metadata" => manifest.images[0].file_name = "another-file.png".into(),
                    _ => unreachable!(),
                }
                service
                    .store
                    .put(
                        "local-save-manifest",
                        &image.task_id,
                        &image.account_id,
                        &manifest,
                    )
                    .unwrap();
            }
            assert_eq!(
                service.begin_image_export(&image.id).err().unwrap().code,
                "local_image_corrupt",
                "{defect}"
            );
        }
    }
    #[test]
    fn image_exports_keep_verified_bytes_and_reject_manifest_changes_at_commit() {
        for output in ["copy", "save"] {
            for change_manifest in [false, true] {
                let (_dir, service, _vault, record, _install) = setup();
                let image = clipboard_fixture(&service, &record);
                let plan = service.begin_image_export(&image.id).unwrap();
                let source = plan.source.clone();
                let target = export_target(&service, false);
                let mut sink = FakeImageClipboard::default();
                let read = plan.read().unwrap();
                let decoded = if output == "copy" {
                    Some(ClipboardImage::decode(&read.bytes, (image.width, image.height)).unwrap())
                } else {
                    None
                };
                let replacement = export_png(2, 2, 255);
                std::fs::write(&source, &replacement).unwrap();
                if change_manifest {
                    let mut manifest = service
                        .store
                        .get::<images::SaveManifest>("local-save-manifest", &image.task_id)
                        .unwrap()
                        .unwrap();
                    manifest.images[0].hash = sha256(&replacement);
                    service
                        .store
                        .put(
                            "local-save-manifest",
                            &image.task_id,
                            &image.account_id,
                            &manifest,
                        )
                        .unwrap();
                }
                let result = if let Some(pixels) = decoded {
                    DecodedImageCopy {
                        plan: read.plan,
                        pixels,
                    }
                    .commit(&mut sink)
                } else {
                    read.save(&target)
                };
                if change_manifest {
                    assert_eq!(result.unwrap_err().code, "local_image_corrupt");
                    assert!(sink.frames.is_empty());
                    assert!(!target.exists());
                } else {
                    result.unwrap();
                    if output == "copy" {
                        assert_eq!(sink.frames, vec![(2, 2, vec![0; 16])]);
                    } else {
                        assert_eq!(std::fs::read(&target).unwrap(), export_png(2, 2, 0));
                    }
                }
            }
        }
    }
    #[derive(Default)]
    struct FakeImageClipboard {
        frames: Vec<(usize, usize, Vec<u8>)>,
        denied: bool,
    }
    impl ImageClipboardSink for FakeImageClipboard {
        fn set_image(&mut self, image: ClipboardImage) -> Result<()> {
            if self.denied {
                return Err(fail("clipboard_unavailable"));
            }
            self.frames
                .push((image.width(), image.height(), image.into_bytes()));
            Ok(())
        }
    }
    #[test]
    fn clipboard_copy_uses_bounded_pixels_and_releases_leases_on_success_or_failure() {
        let (_dir, service, _vault, record, _install) = setup();
        let image = clipboard_fixture(&service, &record);
        let mut sink = FakeImageClipboard::default();
        service
            .begin_image_export(&image.id)
            .unwrap()
            .decode()
            .unwrap()
            .commit(&mut sink)
            .unwrap();
        assert_eq!(sink.frames, vec![(2, 2, vec![0; 16])]);
        assert_eq!(service.image_memory_workers.available_permits(), 2);
        drop(service.begin_maintenance().unwrap());
        sink.denied = true;
        assert_eq!(
            service
                .begin_image_export(&image.id)
                .unwrap()
                .decode()
                .unwrap()
                .commit(&mut sink)
                .unwrap_err()
                .code,
            "clipboard_unavailable"
        );
        assert_eq!(sink.frames.len(), 1);
        assert_eq!(service.image_memory_workers.available_permits(), 2);
        drop(service.begin_maintenance().unwrap());
        let plan = service.begin_image_export(&image.id).unwrap();
        std::fs::write(&plan.source, b"invalid synthetic image").unwrap();
        assert_eq!(plan.decode().err().unwrap().code, "local_image_corrupt");
        assert_eq!(service.image_memory_workers.available_permits(), 2);
        drop(service.begin_maintenance().unwrap());
    }
    #[test]
    fn clipboard_copy_late_logout_selection_and_revision_never_reach_sink() {
        for change in ["logout", "account", "origin", "connection", "revision"] {
            let (_dir, service, _vault, record, _install) = setup();
            let image = clipboard_fixture(&service, &record);
            // Freeze at the IPC entry before queueing the background decode.
            let plan = service.begin_image_export(&image.id).unwrap();
            let (decoded_tx, decoded_rx) = std::sync::mpsc::sync_channel(0);
            let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
            let worker = std::thread::spawn(move || {
                let decoded = plan.decode().unwrap();
                decoded_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                let mut sink = FakeImageClipboard::default();
                (decoded.commit(&mut sink), sink)
            });
            decoded_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let expected = match change {
                "logout" => {
                    service.commit_logout(&record.view.id).unwrap();
                    "session_expired"
                }
                "revision" => {
                    service
                        .select_node(
                            &record.view.id,
                            record.view.node_id.as_ref().unwrap(),
                            record.view.revision,
                        )
                        .unwrap();
                    "connection_conflict"
                }
                _ => {
                    let mut other = record.clone();
                    other.view.id = uuid();
                    if change == "account" {
                        other.view.account_id = "another-account".into();
                    } else if change == "origin" {
                        other.view.panel_origin = "https://other-panel.example.test".into();
                        other.installation_id = uuid();
                    }
                    let mut cache = service.account_cache(&record).unwrap();
                    cache.account.id = other.view.account_id.clone();
                    service
                        .store
                        .put("connection", &other.view.id, &other.view.account_id, &other)
                        .unwrap();
                    service
                        .store
                        .put(
                            "account-cache",
                            &other.view.id,
                            &other.view.account_id,
                            &cache,
                        )
                        .unwrap();
                    service
                        .store
                        .set_meta(
                            &authorization_epoch_reference(
                                &other.view.panel_origin,
                                &other.installation_id,
                                &other.view.account_id,
                            ),
                            &other.authorization_epoch.to_string(),
                        )
                        .unwrap();
                    service.select_connection(&other.view.id).unwrap();
                    "connection_conflict"
                }
            };
            release_tx.send(()).unwrap();
            let (result, sink) = worker.join().unwrap();
            assert_eq!(result.unwrap_err().code, expected, "{change}");
            assert!(sink.frames.is_empty(), "{change}");
            assert_eq!(service.image_memory_workers.available_permits(), 2);
        }
    }
    #[test]
    fn clipboard_copy_rechecks_epoch_expiry_forget_and_history_after_decode() {
        for change in ["epoch", "reauthorized", "expired", "forgetting", "deleted"] {
            let (_dir, service, _vault, mut record, _install) = setup();
            let image = clipboard_fixture(&service, &record);
            let decoded = service
                .begin_image_export(&image.id)
                .unwrap()
                .decode()
                .unwrap();
            let expected = if change == "deleted" {
                assert_eq!(
                    service.delete_history(&[image.id.clone()], true).unwrap(),
                    1
                );
                "local_image_missing"
            } else {
                let _gate = service.account_gate.lock().unwrap();
                match change {
                    "epoch" | "reauthorized" => {
                        let next = service.advance_authorization_epoch(&record).unwrap();
                        if change == "reauthorized" {
                            record.authorization_epoch = next;
                            service
                                .store
                                .put(
                                    "connection",
                                    &record.view.id,
                                    &record.view.account_id,
                                    &record,
                                )
                                .unwrap();
                        }
                        "connection_conflict"
                    }
                    "expired" => {
                        record.session_expires_at = Some(chrono::Utc::now().timestamp() - 1);
                        service
                            .store
                            .put(
                                "connection",
                                &record.view.id,
                                &record.view.account_id,
                                &record,
                            )
                            .unwrap();
                        "session_expired"
                    }
                    "forgetting" => {
                        service
                            .store
                            .set_meta(
                                &format!("installation_forgetting/{}", record.installation_id),
                                "1",
                            )
                            .unwrap();
                        "installation_forgetting"
                    }
                    _ => unreachable!(),
                }
            };
            let mut sink = FakeImageClipboard::default();
            assert_eq!(
                decoded.commit(&mut sink).unwrap_err().code,
                expected,
                "{change}"
            );
            assert!(sink.frames.is_empty(), "{change}");
        }
    }
    #[test]
    fn clipboard_copy_and_update_maintenance_exclude_each_other() {
        let (_dir, service, _vault, record, _install) = setup();
        let image = clipboard_fixture(&service, &record);
        let plan = service.begin_image_export(&image.id).unwrap();
        assert_eq!(
            service.begin_maintenance().err().unwrap().code,
            "update_busy"
        );
        let decoded = plan.decode().unwrap();
        assert_eq!(
            service.begin_maintenance().err().unwrap().code,
            "update_busy"
        );
        let mut sink = FakeImageClipboard::default();
        decoded.commit(&mut sink).unwrap();
        let maintenance = service.begin_maintenance().unwrap();
        assert_eq!(
            service.begin_image_export(&image.id).err().unwrap().code,
            "update_busy"
        );
        assert_eq!(sink.frames.len(), 1);
        drop(maintenance);
        // Also fail closed if a maintenance gate is discovered at the final
        // commit (e.g. a persistent storage gate changed by recovery).
        let decoded = service
            .begin_image_export(&image.id)
            .unwrap()
            .decode()
            .unwrap();
        {
            let _gate = service.account_gate.lock().unwrap();
            service.maintenance_active.store(true, Ordering::SeqCst);
        }
        assert_eq!(decoded.commit(&mut sink).unwrap_err().code, "update_busy");
        assert_eq!(sink.frames.len(), 1);
        service.maintenance_active.store(false, Ordering::SeqCst);
        drop(service.begin_maintenance().unwrap());
    }
    #[test]
    fn clipboard_copy_final_write_is_serialized_with_logout() {
        struct BlockingSink {
            entered: std::sync::mpsc::SyncSender<()>,
            release: std::sync::mpsc::Receiver<()>,
            sink: FakeImageClipboard,
        }
        impl ImageClipboardSink for BlockingSink {
            fn set_image(&mut self, image: ClipboardImage) -> Result<()> {
                self.entered.send(()).unwrap();
                self.release.recv_timeout(Duration::from_secs(5)).unwrap();
                self.sink.set_image(image)
            }
        }
        let (_dir, service, _vault, record, _install) = setup();
        let image = clipboard_fixture(&service, &record);
        let decoded = service
            .begin_image_export(&image.id)
            .unwrap()
            .decode()
            .unwrap();
        let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(0);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
        let copying = std::thread::spawn(move || {
            let mut sink = BlockingSink {
                entered: entered_tx,
                release: release_rx,
                sink: FakeImageClipboard::default(),
            };
            decoded.commit(&mut sink).unwrap();
            sink.sink
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(0);
        let (done_tx, done_rx) = std::sync::mpsc::sync_channel(0);
        let logging_out = service.clone();
        let connection_id = record.view.id.clone();
        let logout = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            logging_out.commit_logout(&connection_id).unwrap();
            done_tx.send(()).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // Do not await logout from inside the sink: release its gate first.
        assert_eq!(
            done_rx.recv_timeout(Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        );
        release_tx.send(()).unwrap();
        assert_eq!(copying.join().unwrap().frames.len(), 1);
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        logout.join().unwrap();
        assert_eq!(
            service.record(&record.view.id).unwrap().session_status,
            "logged_out"
        );
    }
    struct DenyDeleteVault {
        base: Arc<FakeVault>,
        deny: std::sync::atomic::AtomicBool,
    }
    impl Vault for DenyDeleteVault {
        fn get(&self, k: &str) -> Result<Zeroizing<Vec<u8>>> {
            self.base.get(k)
        }
        fn put(&self, k: &str, v: &[u8]) -> Result<()> {
            self.base.put(k, v)
        }
        fn delete(&self, k: &str) -> Result<()> {
            if self.deny.load(Ordering::SeqCst) {
                Err(fail("vault_unavailable"))
            } else {
                self.base.delete(k)
            }
        }
    }
    #[test]
    fn reauthorization_without_delivery_preserves_complete_secret_cleanup_and_same_ref_reuse() {
        for denied in [false, true] {
            let (_dir, original, vault, mut record, install) = setup();
            let adapter = Arc::new(DenyDeleteVault {
                base: vault.clone(),
                deny: std::sync::atomic::AtomicBool::new(denied),
            });
            let service = Service::new(
                original.store.clone(),
                adapter.clone(),
                original.paths.clone(),
            )
            .unwrap();
            service
                .accept_credential(
                    &mut record,
                    &install,
                    serde_json::from_value(golden()["credential_reply"]["data"].clone()).unwrap(),
                )
                .unwrap();
            let old = record.credential_reference.clone().unwrap();
            assert!(
                service
                    .store
                    .list::<HelperBinding>("binding", None)
                    .unwrap()
                    .is_empty()
            );
            let (_, session) =
                serde_json::from_value::<TokenResponse>(golden()["token_reply"]["data"].clone())
                    .unwrap()
                    .into_session(&install)
                    .unwrap();
            let renewed = service
                .commit_authorization(&install, record.view.account_id.clone(), session, 0)
                .unwrap();
            assert!(renewed.credential_reference.is_none());
            assert!(renewed.authorization_epoch > record.authorization_epoch);
            assert_eq!(renewed.view.id, record.view.id);
            if denied {
                assert_eq!(renewed.session_status, "vault_unavailable");
                assert!(renewed.cleanup_references.contains(&old));
                assert!(adapter.get(&old).is_ok());
            } else {
                assert!(adapter.get(&old).is_err());
                assert!(renewed.cleanup_references.is_empty());
            }
            adapter.deny.store(false, Ordering::SeqCst);
            service.commit_logout(&record.view.id).unwrap();
            assert!(adapter.get(&old).is_err());
            assert!(
                service
                    .record(&record.view.id)
                    .unwrap()
                    .cleanup_references
                    .is_empty()
            );
        }
        let (_dir, service, vault, mut record, install) = setup();
        let delivery = golden()["credential_reply"]["data"].clone();
        service
            .accept_credential(
                &mut record,
                &install,
                serde_json::from_value(delivery.clone()).unwrap(),
            )
            .unwrap();
        let reference = record.credential_reference.clone().unwrap();
        record.cleanup_references.push(reference.clone());
        service
            .store
            .put(
                "connection",
                &record.view.id,
                &record.view.account_id,
                &record,
            )
            .unwrap();
        service
            .accept_credential(
                &mut record,
                &install,
                serde_json::from_value(delivery).unwrap(),
            )
            .unwrap();
        assert!(record.cleanup_references.is_empty());
        assert!(vault.get(&reference).is_ok());
        service.commit_logout(&record.view.id).unwrap();
        assert!(vault.get(&reference).is_err());
    }
    #[test]
    fn pruned_restore_source_is_rejected_before_original_files_are_changed() {
        let (_dir, service, _vault, record, install) = setup();
        let path = service.paths.codex_config.clone();
        std::fs::write(&path, b"model='before'\n").unwrap();
        let journal = service
            .config_engine()
            .prepare(
                ClientKind::Codex,
                &record.view.account_id,
                &install,
                vec![FileChange {
                    path: path.display().to_string(),
                    before: Some(b"model='before'\n".to_vec()),
                    after: Some(b"model='after'\n".to_vec()),
                }],
                None,
                vec![],
            )
            .unwrap();
        service.config_engine().apply(&journal.id).unwrap();
        let restore = service
            .config_engine()
            .prepare_restore(&journal.id)
            .unwrap();
        service.store.delete("backup", &journal.id).unwrap();
        service.store.delete("journal", &journal.id).unwrap();
        assert_eq!(
            service.config_engine().apply(&restore.id).unwrap_err().code,
            "backup_invalid"
        );
        assert_eq!(std::fs::read(path).unwrap(), b"model='after'\n");
    }
    #[test]
    fn denied_secret_delete_cannot_reopen_known_revoked_or_readonly_grants() {
        for state in ["revoked", "readonly", "expired", "remote-revoked"] {
            let (_dir, original, vault, mut record, install) = setup();
            let adapter = Arc::new(DenyDeleteVault {
                base: vault.clone(),
                deny: std::sync::atomic::AtomicBool::new(true),
            });
            let service = Service::new(
                original.store.clone(),
                adapter.clone(),
                original.paths.clone(),
            )
            .unwrap();
            service
                .accept_credential(
                    &mut record,
                    &install,
                    serde_json::from_value(golden()["credential_reply"]["data"].clone()).unwrap(),
                )
                .unwrap();
            let old_reference = record.credential_reference.clone().unwrap();
            let binding = HelperBinding {
                id: uuid(),
                client: ClientKind::Codex,
                origin: record.view.panel_origin.clone(),
                account_id: record.view.account_id.clone(),
                installation_id: record.installation_id.clone(),
                provider_id: record.view.provider_id.clone(),
                credential_revision: record.view.credential_revision.clone().unwrap(),
                secret_reference: old_reference.clone(),
                enabled: true,
                expires_at: None,
                authorization_epoch: record.authorization_epoch,
            };
            service
                .store
                .put("binding", &binding.id, &binding.account_id, &binding)
                .unwrap();
            if state == "remote-revoked" {
                service
                    .disable_inference_before_cleanup(&mut record, "revoked")
                    .unwrap();
            } else {
                let mut cache = service.account_cache(&record).unwrap();
                if state == "readonly" {
                    cache.inference_allowed = false;
                } else {
                    cache.keys[0].state = state.into();
                }
                service.apply_known_key_state(&mut record, &cache).unwrap();
            }
            let current = service.record(&record.view.id).unwrap();
            assert_eq!(current.session_status, "vault_unavailable");
            assert!(current.credential_reference.is_none());
            assert!(current.view.credential_revision.is_none());
            assert_ne!(current.view.permission_state, "allowed");
            assert!(current.cleanup_references.contains(&old_reference));
            assert!(adapter.get(&old_reference).is_ok());
            assert!(
                bound_helper_token(
                    &binding,
                    ClientKind::Codex,
                    &binding.id,
                    &service.store,
                    adapter.as_ref(),
                    chrono::Utc::now().timestamp()
                )
                .is_err()
            );
            assert!(
                service
                    .current_connection(&record.view.id, current.view.revision)
                    .is_err()
            );
            adapter.deny.store(false, Ordering::SeqCst);
            service.commit_logout(&record.view.id).unwrap();
            assert_eq!(
                service.record(&record.view.id).unwrap().session_status,
                "logged_out"
            );
            assert!(
                service
                    .record(&record.view.id)
                    .unwrap()
                    .cleanup_references
                    .is_empty()
            );
            assert!(adapter.get(&old_reference).is_err());
        }
    }
    #[test]
    fn forget_gate_between_token_and_claim_prevents_send_without_changing_task() {
        let (_dir, service, vault, mut record, install) = setup();
        service
            .accept_credential(
                &mut record,
                &install,
                serde_json::from_value(golden()["credential_reply"]["data"].clone()).unwrap(),
            )
            .unwrap();
        let mut snapshot = task_snapshot(&record);
        snapshot.task.state = "queued".into();
        snapshot.task.request_id = None;
        service.persist_task(&snapshot).unwrap();
        let binding = HelperBinding {
            id: uuid(),
            client: ClientKind::Codex,
            origin: record.view.panel_origin.clone(),
            account_id: record.view.account_id.clone(),
            installation_id: record.installation_id.clone(),
            provider_id: record.view.provider_id.clone(),
            credential_revision: record.view.credential_revision.clone().unwrap(),
            secret_reference: record.credential_reference.clone().unwrap(),
            enabled: true,
            expires_at: None,
            authorization_epoch: record.authorization_epoch,
        };
        let token = bound_helper_token(
            &binding,
            ClientKind::Codex,
            &binding.id,
            &service.store,
            vault.as_ref(),
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
        assert!(!token.is_empty());
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let other = service.clone();
        let gate = barrier.clone();
        let install_id = install.id.clone();
        let thread = std::thread::spawn(move || {
            let _lock = other.account_gate.lock().unwrap();
            other
                .store
                .set_meta(&format!("installation_forgetting/{install_id}"), "1")
                .unwrap();
            gate.wait();
        });
        barrier.wait();
        thread.join().unwrap();
        assert_eq!(
            service.claim_image_send(&mut snapshot).unwrap_err().code,
            "installation_forgetting"
        );
        assert_eq!(
            service.scoped_task(&snapshot.task.id).unwrap().task.state,
            "queued"
        );
        assert!(snapshot.task.request_id.is_none());
    }
    #[test]
    fn preview_protocol_and_bootstrap_enforce_current_actor_and_logout() {
        let (_dir, service, _vault, record, _install) = setup();
        let mut output = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2, 2)
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        let bytes = output.into_inner();
        let mut snapshot = task_snapshot(&record);
        service.persist_task(&snapshot).unwrap();
        let received = images::ReceivedImage {
            id: uuid(),
            info: images::validate_image(&bytes).unwrap(),
            bytes,
        };
        images::stage_received(&service.store, &snapshot, &[received]).unwrap();
        let image = images::finish_local_save(&service.store, &snapshot)
            .unwrap()
            .remove(0);
        snapshot.task.image_ids = vec![image.id.clone()];
        service.persist_task(&snapshot).unwrap();
        let path = url::Url::parse(&image.preview_url.clone().unwrap())
            .unwrap()
            .path()
            .to_owned();
        assert!(service.read_preview_asset(&path).is_ok());
        assert!(!path.contains("Users"));
        for other_origin in [
            record.view.panel_origin.clone(),
            "https://other-panel.example.test".into(),
        ] {
            let mut other = record.clone();
            other.view.id = uuid();
            other.view.account_id = if other_origin == record.view.panel_origin {
                "another-account".into()
            } else {
                record.view.account_id.clone()
            };
            other.view.panel_origin = other_origin;
            let mut cache = service.account_cache(&record).unwrap();
            cache.account.id = other.view.account_id.clone();
            service
                .store
                .put("connection", &other.view.id, &other.view.account_id, &other)
                .unwrap();
            service
                .store
                .put(
                    "account-cache",
                    &other.view.id,
                    &other.view.account_id,
                    &cache,
                )
                .unwrap();
            service.select_connection(&other.view.id).unwrap();
            assert!(service.read_preview_asset(&path).is_err());
            let wrong = url::Url::parse(&images::preview_url(&service.store, &image.id).unwrap())
                .unwrap()
                .path()
                .to_owned();
            assert!(service.read_preview_asset(&wrong).is_err());
            let bootstrap = service.bootstrap().unwrap();
            assert!(bootstrap.tasks.is_empty() && bootstrap.image_history.is_empty());
        }
        service.select_connection(&record.view.id).unwrap();
        assert!(service.read_preview_asset(&path).is_ok());
        service.commit_logout(&record.view.id).unwrap();
        assert!(service.read_preview_asset(&path).is_err());
        let view = service.bootstrap().unwrap();
        assert!(
            view.tasks.is_empty()
                && view.image_history.is_empty()
                && view.session.account.is_none()
        );
        assert!(service.begin_image_export(&image.id).is_err());
        assert!(service.read_preview_asset("/../../orbit.sqlite3").is_err());
    }
    #[test]
    fn codex_binding_projection_preserves_policy_and_revises_every_raw_change() {
        let mut wire = golden()["models_reply"]["data"][0].clone();
        wire["codex_catalog_binding"] = json!({"state":"declared","verified":false,
            "profile_id":crate::codex_binding::PROFILE_ID,
            "cli_version":crate::codex_catalog::SUPPORTED_CLI_VERSION,
            "source_sha256":crate::codex_catalog::SOURCE_SHA256,"template_slug":"gpt-5.5"});
        wire["reasoning"] = json!({"state":"restricted","capability_status":"supported",
            "supported_efforts":["low","medium","high"],"allowed_efforts":["medium","high"],
            "default_effort":"medium","revision":7});
        let mapped = map_model(&wire, "catalog-a").unwrap();
        assert_eq!(
            mapped.reasoning.as_ref().unwrap().default_effort.as_deref(),
            Some("medium")
        );
        assert_eq!(mapped.reasoning.as_ref().unwrap().revision, Some(7));
        assert_eq!(mapped.reasoning_efforts, ["medium", "high"]);
        assert!(matches!(
            mapped.codex_catalog_binding,
            Some(CodexCatalogBindingView::Declared {
                verified: false,
                ..
            })
        ));
        wire["codex_catalog_binding"]["profile_id"] = "unknown-do-not-echo".into();
        let changed = map_model(&wire, "catalog-a").unwrap();
        assert_ne!(mapped.revision, changed.revision);
        assert_eq!(
            serde_json::to_value(changed.codex_catalog_binding).unwrap(),
            json!({"state":"unavailable","verified":false})
        );
        assert!(changed.reasoning.is_some());
    }
    #[test]
    fn configured_reference_preserves_partial_unknown_and_decimal_values() {
        for state in ["partial", "unknown", "configured"] {
            let mut model = golden()["models_reply"]["data"][0].clone();
            model["reference_price"] = json!({"source":"configured_reference","status":state,"currency":"USD","meters":[{"meter":"input","unit_size":1000000,"unit_price":"0.000000001230000000"}]});
            let mapped = map_model(&model, "wire-revision")
                .unwrap()
                .official_price
                .unwrap();
            assert_eq!(mapped.source, "configured_reference");
            assert_eq!(mapped.status, state);
            assert_eq!(
                mapped.rates[0].unit_price.as_deref(),
                Some("0.000000001230000000")
            );
        }
    }
    #[test]
    fn codex_config_service_rebuild_coordinates_only_authenticated_pair_and_repairs_backup_projection()
     {
        for mode in [
            "publishing",
            "external-conflict",
            "committed-gap",
            "prepared",
            "forgetting",
            "legacy-no-policy",
            "missing-context",
            "cache-missing",
            "epoch",
            "logout",
        ] {
            let f = policy_fixture(ClientKind::Codex);
            catalog_menu(&f);
            let mut journal = prepared_catalog(&f, &f.preview());
            let engine = f.service.config_engine();
            let files = engine.synthetic_files(&journal).unwrap();
            if mode == "legacy-no-policy" {
                let context = engine.codex_context(&journal).unwrap().unwrap();
                journal = engine
                    .prepare(
                        ClientKind::Codex,
                        &f.record.view.account_id,
                        &journal.installation,
                        files.clone(),
                        None,
                        vec![],
                    )
                    .unwrap();
                engine
                    .freeze_target(
                        &mut journal,
                        Some("2.19675.0-synthetic".into()),
                        Some("synthetic.codex".into()),
                    )
                    .unwrap();
                engine.freeze_codex_context(&mut journal, context).unwrap();
            } else if mode == "missing-context" {
                engine.synthetic_without_context(&mut journal).unwrap();
            }
            if mode == "committed-gap" {
                engine.apply(&journal.id).unwrap();
                f.service.store.delete("backup", &journal.id).unwrap();
            } else if mode != "prepared" {
                journal.phase = "publishing".into();
                f.service
                    .store
                    .put("journal", &journal.id, &f.record.view.account_id, &journal)
                    .unwrap();
                if mode == "external-conflict" {
                    std::fs::write(&files[1].path, b"model='external'\n").unwrap();
                } else {
                    crate::config::atomic_write(
                        Path::new(&files[0].path),
                        files[0].after.as_ref().unwrap(),
                    )
                    .unwrap();
                }
            }
            if mode == "forgetting" {
                f.service
                    .store
                    .set_meta(
                        &format!("installation_forgetting/{}", journal.installation.id),
                        "1",
                    )
                    .unwrap();
            } else if mode == "cache-missing" {
                f.service
                    .store
                    .delete("account-cache", &f.record.view.id)
                    .unwrap();
            } else if mode == "epoch" {
                f.service
                    .store
                    .set_meta(
                        &authorization_epoch_reference(
                            &f.record.view.panel_origin,
                            &f.record.installation_id,
                            &f.record.view.account_id,
                        ),
                        "999",
                    )
                    .unwrap();
            } else if mode == "logout" {
                f.service.commit_logout(&f.record.view.id).unwrap();
            }
            let rebuilt = Service::new(
                f.service.store.clone(),
                f.vault.clone(),
                f.service.paths.clone(),
            )
            .unwrap();
            if ["publishing", "committed-gap"].contains(&mode) {
                for file in &files {
                    assert_eq!(
                        read_file(Path::new(&file.path)).unwrap(),
                        file.after,
                        "{mode}"
                    );
                }
                assert!(
                    rebuilt
                        .store
                        .get::<Journal>("backup", &journal.id)
                        .unwrap()
                        .is_some()
                );
                assert!(
                    !rebuilt
                        .pending_configuration_recovery(ClientKind::Codex)
                        .unwrap()
                );
                // A second recovery pass has nothing to repair or re-enable.
                let revision = rebuilt.store.snapshot().unwrap().revision;
                rebuilt.coordinate_configuration_recovery().unwrap();
                assert_eq!(rebuilt.store.snapshot().unwrap().revision, revision);
            } else if mode == "prepared" {
                for file in &files {
                    assert_eq!(read_file(Path::new(&file.path)).unwrap(), file.before);
                }
                assert!(
                    rebuilt
                        .store
                        .get::<Journal>("backup", &journal.id)
                        .unwrap()
                        .is_none()
                );
            } else {
                assert_eq!(
                    read_file(Path::new(&files[1].path)).unwrap(),
                    if mode == "external-conflict" {
                        Some(b"model='external'\n".to_vec())
                    } else {
                        files[1].before.clone()
                    },
                    "{mode}"
                );
                if mode == "external-conflict" {
                    assert_eq!(
                        read_file(Path::new(&files[0].path)).unwrap(),
                        files[0].before
                    );
                }
                assert!(
                    rebuilt
                        .pending_configuration_recovery(ClientKind::Codex)
                        .unwrap()
                );
                let view = rebuilt.bootstrap().unwrap();
                assert!(
                    view.backups
                        .iter()
                        .any(|backup| backup.id == journal.id && backup.state == "conflict")
                );
                assert_eq!(
                    view.clients
                        .iter()
                        .find(|client| client.client == ClientKind::Codex)
                        .unwrap()
                        .configuration_state,
                    "conflict"
                );
                assert!(
                    rebuilt
                        .store
                        .list::<HelperBinding>("binding", None)
                        .unwrap()
                        .iter()
                        .all(|binding| !binding.enabled)
                );
                if mode == "missing-context" {
                    assert_eq!(
                        rebuilt.preview_restore(&journal.id).unwrap_err().code,
                        "codex_catalog_context_unverified"
                    );
                }
            }
        }
    }
    #[test]
    fn cancellation_before_send_claim_preserves_cancelled_without_http() {
        let (_dir, service, _vault, record, _install) = setup();
        let mut snapshot = task_snapshot(&record);
        snapshot.task.state = "queued".into();
        snapshot.task.request_id = None;
        service.persist_task(&snapshot).unwrap();
        let cancelled = service.cancel_image_task(&snapshot.task.id).unwrap();
        assert_eq!(cancelled.state, "cancelled");
        assert_eq!(
            service.claim_image_send(&mut snapshot).unwrap_err().code,
            "task_cancelled"
        );
        let read = service.store.snapshot().unwrap();
        let task: TaskView = read.get("task", &snapshot.task.id).unwrap().unwrap();
        let snap: TaskSnapshot = read
            .get("task-snapshot", &snapshot.task.id)
            .unwrap()
            .unwrap();
        assert_eq!(task.state, "cancelled");
        assert_eq!(snap.task.state, "cancelled");
        assert!(task.request_id.is_none());
    }
    #[tokio::test]
    async fn concurrent_creation_cannot_exceed_two_and_all_cancel_before_http() {
        let (_dir, service, _vault, mut record, install) = setup();
        service
            .accept_credential(
                &mut record,
                &install,
                serde_json::from_value(golden()["credential_reply"]["data"].clone()).unwrap(),
            )
            .unwrap();
        let model = service
            .account_cache(&record)
            .unwrap()
            .models
            .into_iter()
            .find(|m| m.category == "image")
            .unwrap()
            .id;
        let first = service.image_workers.clone().acquire_owned().await.unwrap();
        let second = service.image_workers.clone().acquire_owned().await.unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(9));
        let mut threads = vec![];
        for _ in 0..8 {
            let worker = service.clone();
            let record = record.clone();
            let model = model.clone();
            let barrier = barrier.clone();
            let handle = tokio::runtime::Handle::current();
            threads.push(std::thread::spawn(move || {
                let _runtime = handle.enter();
                barrier.wait();
                worker.create_image_task(
                    &record.view.id,
                    record.view.revision,
                    &model,
                    "synthetic prompt".into(),
                    ImageParameters::default(),
                    vec![],
                )
            }));
        }
        barrier.wait();
        let mut created = vec![];
        for thread in threads {
            match thread.join().unwrap() {
                Ok(task) => created.push(task),
                Err(e) => assert_eq!(e.code, "image_busy"),
            }
        }
        assert_eq!(created.len(), 2);
        for task in &created {
            assert_eq!(
                service.cancel_image_task(&task.id).unwrap().state,
                "cancelled"
            );
        }
        drop((first, second));
        tokio::task::yield_now().await;
        for task in created {
            let persisted = service.scoped_task(&task.id).unwrap();
            assert_eq!(persisted.task.state, "cancelled");
            assert!(persisted.task.request_id.is_none());
        }
    }
    #[test]
    fn account_session_outlives_access_token_and_image_memory_is_bounded() {
        let (_dir, service, _vault, mut record, _install) = setup();
        record.expires_at = 1;
        service
            .store
            .put(
                "connection",
                &record.view.id,
                &record.view.account_id,
                &record,
            )
            .unwrap();
        assert_eq!(service.bootstrap().unwrap().session.status, "authenticated");
        record.session_expires_at = Some(1);
        service
            .store
            .put(
                "connection",
                &record.view.id,
                &record.view.account_id,
                &record,
            )
            .unwrap();
        assert_eq!(service.bootstrap().unwrap().session.status, "expired");
        let first = service.image_memory_permit().unwrap();
        let second = service.image_memory_permit().unwrap();
        assert_eq!(
            service.image_memory_permit().unwrap_err().code,
            "image_busy"
        );
        drop(first);
        assert!(service.image_memory_permit().is_ok());
        drop(second);
    }
    #[tokio::test]
    async fn forgetting_installation_blocks_new_login_before_network_or_key_generation() {
        let (_dir, service, _vault, record, install) = setup();
        service
            .store
            .set_meta(&format!("installation_forgetting/{}", install.id), "1")
            .unwrap();
        assert_eq!(
            service.start_login(&install.origin).await.unwrap_err().code,
            "installation_forgetting"
        );
        assert_eq!(
            service
                .current_connection(&record.view.id, record.view.revision)
                .err()
                .unwrap()
                .code,
            "installation_forgetting"
        );
        assert_eq!(
            service
                .store
                .installation(&install.origin)
                .unwrap()
                .unwrap()
                .id,
            install.id
        );
    }
    #[test]
    fn claude_official_body_export_is_gated_and_never_claims_config_applied() {
        let (_dir, original, vault, mut record, install) = setup();
        original
            .accept_credential(
                &mut record,
                &install,
                serde_json::from_value(golden()["credential_reply"]["data"].clone()).unwrap(),
            )
            .unwrap();
        let model = original.account_cache(&record).unwrap().models[0]
            .id
            .clone();
        let blocked = original
            .preview_claude_diagnostic(&record.view.id, record.view.revision, &model)
            .unwrap();
        assert!(!blocked.can_apply && !blocked.can_export_import);
        let mut paths = original.paths.clone();
        paths.writes_authorized = true;
        std::fs::write(&paths.helper, b"synthetic helper fixture - never executed").unwrap();
        paths.helper_expected_sha256 = Some(sha256(b"synthetic helper fixture - never executed"));
        qualify_claude_paths(&mut paths, Arc::new(AtomicU64::new(0)));
        let service = Service::new(original.store.clone(), vault.clone(), paths).unwrap();
        let mut cache = service.account_cache(&record).unwrap();
        cache.models[0]
            .capabilities
            .push("anthropic.messages".into());
        service
            .store
            .put(
                "account-cache",
                &record.view.id,
                &record.view.account_id,
                &cache,
            )
            .unwrap();
        {
            let mut clients = service.clients.lock().unwrap();
            let client = clients
                .iter_mut()
                .find(|c| c.client == ClientKind::Claude)
                .unwrap();
            client.installed = true;
            client.platform_supported = true;
            client.version = Some(crate::claude_config::SUPPORTED_APP_VERSION.into());
            client.bundle_id = Some("com.anthropic.claudefordesktop".into());
            client.app_path = Some("synthetic-installed-Claude.app".into());
            client.managed = "no".into();
            service
                .store
                .put("client-state", "claude", "local", client)
                .unwrap();
        }
        let preview = service
            .preview_claude_diagnostic(&record.view.id, record.view.revision, &model)
            .unwrap();
        assert!(!preview.can_apply && preview.can_export_import);
        assert_eq!(
            preview.blocking_reason.as_deref(),
            Some("schema_unverified")
        );
        // Diagnostic exports cannot replace files owned by either client writer.
        for protected in [
            service.paths.codex_config.clone(),
            service.paths.codex_catalog(),
            service
                .paths
                .claude_library
                .parent()
                .unwrap()
                .join("claude_desktop_config.json"),
        ] {
            let blocked = service
                .preview_claude_diagnostic(&record.view.id, record.view.revision, &model)
                .unwrap();
            assert!(blocked.can_export_import);
            let before = read_file(&protected).unwrap();
            let revision = service.store.snapshot().unwrap().revision;
            assert_eq!(
                service
                    .export_claude_import(&blocked.id, true, &protected)
                    .unwrap_err()
                    .code,
                "invalid_path"
            );
            assert_eq!(read_file(&protected).unwrap(), before);
            assert_eq!(service.store.snapshot().unwrap().revision, revision);
            assert!(
                service
                    .store
                    .list::<HelperBinding>("binding", None)
                    .unwrap()
                    .iter()
                    .all(|binding| !binding.enabled)
            );
        }
        let target = service.store.root.join("official-v2-diagnostic.json");
        service
            .export_claude_import(&preview.id, true, &target)
            .unwrap();
        let body: Value = serde_json::from_slice(&std::fs::read(&target).unwrap()).unwrap();
        assert_eq!(body["$schemaVersion"], 2);
        assert_eq!(body["inference"]["provider"], "gateway");
        assert_eq!(
            body["inference"]["baseUrl"],
            json!(cache.claude_endpoints[record.view.node_id.as_ref().unwrap()])
        );
        assert!(body.get("_meta").is_none());
        assert!(body.get("profiles").is_none());
        assert!(
            !std::fs::read_to_string(&target)
                .unwrap()
                .contains("orv_sk_")
        );
        let binding_id = body["inference"]["credential"]["args"][1].as_str().unwrap();
        let binding: HelperBinding = service.store.get("binding", binding_id).unwrap().unwrap();
        assert!(
            bound_helper_token(
                &binding,
                ClientKind::Claude,
                binding_id,
                &service.store,
                vault.as_ref(),
                chrono::Utc::now().timestamp()
            )
            .is_ok()
        );
        assert!(!service.paths.claude_library.exists());
        let client = service
            .bootstrap()
            .unwrap()
            .clients
            .into_iter()
            .find(|c| c.client == ClientKind::Claude)
            .unwrap();
        assert_ne!(client.configuration_state, "saved");
        assert_eq!(client.loaded_state, "unconfirmed");
        assert_eq!(client.connection_state, "not_tested");
        let next = service
            .preview_claude_diagnostic(&record.view.id, record.view.revision, &model)
            .unwrap();
        cache.models[0].revision = "changed-synthetic-capability".into();
        service
            .store
            .put(
                "account-cache",
                &record.view.id,
                &record.view.account_id,
                &cache,
            )
            .unwrap();
        let stale = service.store.root.join("stale-export.json");
        assert_eq!(
            service
                .export_claude_import(&next.id, true, &stale)
                .unwrap_err()
                .code,
            "connection_conflict"
        );
        assert!(!stale.exists());
    }
    #[test]
    fn common_panel_wire_proof_pkce_and_dto_acceptance() {
        let (_dir, service, _vault, mut record, install) = setup();
        let wire = golden();
        let proof = &wire["proof"];
        let input = ProofInput {
            method: proof["method"].as_str().unwrap(),
            request_target: proof["target"].as_str().unwrap(),
            binding_id: wire["start_reply"]["data"]["authorization_request_id"]
                .as_str()
                .unwrap(),
            timestamp: proof["timestamp"].as_str().unwrap().parse().unwrap(),
            nonce: wire["start_reply"]["data"]["proof_nonce"].as_str().unwrap(),
            request_id: proof["request_id"].as_str().unwrap(),
            body: proof["raw_body"].as_str().unwrap().as_bytes(),
            bearer: None,
        };
        assert_eq!(
            canonical_proof(&install, &input).unwrap(),
            proof["canonical"].as_str().unwrap()
        );
        assert_eq!(
            crate::identity::sign_proof(&install, &input, service.vault.as_ref()).unwrap(),
            proof["signature"].as_str().unwrap()
        );
        let body: Value = serde_json::from_str(proof["raw_body"].as_str().unwrap()).unwrap();
        assert_eq!(
            URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(
                body["code_verifier"].as_str().unwrap().as_bytes()
            )),
            wire["start_request"]["code_challenge"].as_str().unwrap()
        );
        assert!(
            body["redirect_uri"]
                .as_str()
                .unwrap()
                .ends_with("/orivelle-orbit/callback")
        );
        let delivery: CredentialDelivery =
            serde_json::from_value(wire["credential_reply"]["data"].clone()).unwrap();
        service
            .accept_credential(&mut record, &install, delivery)
            .unwrap();
        let cache = service.account_cache(&record).unwrap();
        service.apply_known_key_state(&mut record, &cache).unwrap();
        assert_eq!(record.credential_state, "active");
        assert_eq!(
            cache.account.wallet.unwrap().amount.as_deref(),
            Some("12.34567890")
        );
        assert_eq!(cache.nodes[0].maximum_request_bytes, 64 * 1024 * 1024);
        let image = &cache.models[1];
        assert!(!image.image_parameters.as_ref().unwrap().verified);
        assert!(
            images::validate_parameters(
                image,
                &ImageParameters {
                    quality: Some("high".into()),
                    ..Default::default()
                },
                "p",
                0,
                64 * 1024 * 1024
            )
            .is_err()
        );
    }
    #[test]
    fn logical_chain_preserves_lookup_charge_ids_and_real_zero() {
        let (_d, _s, _v, record, _i) = setup();
        let mut task = task(&record);
        let wire = golden();
        apply_server_fact(&mut task, &wire["usage_lookup_reply"]["data"]).unwrap();
        assert_eq!(
            task.request_id.as_deref(),
            Some("33333333-3333-4333-8333-333333333333")
        );
        assert_eq!(
            task.charges[0].request_id.as_deref(),
            Some("44444444-4444-4444-8444-444444444444")
        );
        assert_eq!(task.charges[0].amount.as_deref(), Some("0.00000000"));
        let mut wrong = wire["usage_lookup_reply"]["data"].clone();
        wrong["correlation"]["actor_id"] = json!("another-account");
        assert!(apply_server_fact(&mut task, &wrong).is_err());
        let before = task.charges.clone();
        let unconfirmed = json!({"request_id":task.request_id,"observation_state":"unconfirmed","execution_status":null,"correlation":null,"record":null});
        apply_server_fact(&mut task, &unconfirmed).unwrap();
        assert_eq!(task.charges[0].amount, before[0].amount);
    }
    #[test]
    fn model_and_node_content_changes_invalidate_revisions() {
        let (_d, service, _v, record, _i) = setup();
        let wire = golden();
        let old = service.account_cache(&record).unwrap();
        let mut changed = wire["bootstrap_reply"]["data"].clone();
        changed["models"][1]["image_parameters"]["max_results"] = json!(2);
        changed["nodes"][0]["connection_url"] = json!("https://edge-new.example.test/v1");
        let new = map_account_cache(&changed, &record).unwrap();
        assert_ne!(old.models[1].revision, new.models[1].revision);
        assert_ne!(old.nodes[0].revision, new.nodes[0].revision);
    }
    #[test]
    fn known_revocation_and_expiry_disable_grants_and_ready() {
        let (_d, service, vault, mut record, install) = setup();
        let wire = golden();
        service
            .accept_credential(
                &mut record,
                &install,
                serde_json::from_value(wire["credential_reply"]["data"].clone()).unwrap(),
            )
            .unwrap();
        let id = uuid();
        let binding = HelperBinding {
            id: id.clone(),
            client: ClientKind::Codex,
            origin: install.origin.clone(),
            account_id: record.view.account_id.clone(),
            installation_id: install.id,
            provider_id: "openai".into(),
            credential_revision: record.view.credential_revision.clone().unwrap(),
            secret_reference: record.credential_reference.clone().unwrap(),
            enabled: true,
            expires_at: Some(1),
            authorization_epoch: record.authorization_epoch,
        };
        service
            .store
            .put("binding", &id, &record.view.account_id, &binding)
            .unwrap();
        assert!(
            bound_helper_token(
                &binding,
                ClientKind::Codex,
                &id,
                &service.store,
                vault.as_ref(),
                2
            )
            .is_err()
        );
        let mut cache = service.account_cache(&record).unwrap();
        cache.keys[0].state = "revoked".into();
        service.apply_known_key_state(&mut record, &cache).unwrap();
        assert!(record.credential_reference.is_none());
        assert_eq!(record.credential_state, "revoked");
        assert!(
            !service
                .store
                .get::<HelperBinding>("binding", &id)
                .unwrap()
                .unwrap()
                .enabled
        );
    }
    #[tokio::test]
    async fn manual_refresh_uses_only_account_and_captured_fact_gets_and_keeps_as_of_cache() {
        let (_dir, service, _vault, record, _install) = setup();
        let snapshot = task_snapshot(&record);
        service.persist_task(&snapshot).unwrap();
        let calls = Arc::new(AtomicU64::new(0));
        let observed = calls.clone();
        let wire = golden();
        service.http.scripted(Arc::new(move |request| {
            let wire=wire.clone();let observed=observed.clone();
            Box::pin(async move {
                assert_eq!(request.method, Method::GET);
                assert!(request.body.is_empty());
                assert!(request.headers.iter().any(|(key,_)|key=="X-Orbit-Proof-Signature"));
                let data=if request.url.ends_with("/bootstrap") {wire["bootstrap_reply"].clone()} else {
                    assert!(request.url.ends_with("/usage/records/33333333-3333-4333-8333-333333333333"));
                    let count=observed.fetch_add(1, Ordering::SeqCst);
                    if count==0 {json!({"data":{"request_id":"33333333-3333-4333-8333-333333333333","observation_state":"unconfirmed","execution_status":null,"record":null,"correlation":null}})}
                    else {wire["usage_lookup_reply"].clone()}
                };
                Ok(crate::network::RemoteResponse{status:200,request_id:None,bytes:serde_json::to_vec(&data).unwrap()})
            })
        }));
        let first = service
            .refresh_account(&record.view.id, record.view.revision)
            .await
            .unwrap();
        assert_eq!(first.tasks[0].charge_state, "unavailable");
        assert!(first.session.account.as_ref().unwrap().updated_at.is_some());
        let second = service
            .refresh_account(&record.view.id, record.view.revision)
            .await
            .unwrap();
        assert_eq!(second.tasks[0].charge_state, "charged");
        assert_eq!(
            second.tasks[0].charges[0].request_id.as_deref(),
            Some("44444444-4444-4444-8444-444444444444")
        );
        assert_eq!(second.tasks[0].state, "completed");
        let captured = second.session.account.unwrap().updated_at;
        service.http.scripted(Arc::new(|_| {
            Box::pin(async { Err(fail("panel_unavailable")) })
        }));
        assert!(
            service
                .refresh_account(&record.view.id, record.view.revision)
                .await
                .is_err()
        );
        assert_eq!(
            service
                .bootstrap()
                .unwrap()
                .session
                .account
                .unwrap()
                .updated_at,
            captured
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
    #[tokio::test]
    async fn delayed_fact_merge_preserves_local_save_and_image_deletion_and_rejects_old_epoch() {
        for mode in ["local-save", "delete-image", "logout"] {
            let (_dir, service, _vault, record, _install) = setup();
            let mut snapshot = task_snapshot(&record);
            snapshot.task.state = "save_failed".into();
            snapshot.task.local_save_available = true;
            service.persist_task(&snapshot).unwrap();
            let mut output = std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgba8(2, 2)
                .write_to(&mut output, image::ImageFormat::Png)
                .unwrap();
            let bytes = output.into_inner();
            let received = images::ReceivedImage {
                id: uuid(),
                info: images::validate_image(&bytes).unwrap(),
                bytes,
            };
            images::stage_received(&service.store, &snapshot, &[received]).unwrap();
            if mode == "delete-image" {
                snapshot.task = service.retry_local_save(&snapshot.task.id).unwrap();
            }
            let started = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let (a, b) = (started.clone(), release.clone());
            let fact = golden()["usage_lookup_reply"].clone();
            service.http.scripted(Arc::new(move |request| {
                let (a, b, fact) = (a.clone(), b.clone(), fact.clone());
                Box::pin(async move {
                    assert_eq!(request.method, Method::GET);
                    a.notify_one();
                    b.notified().await;
                    Ok(crate::network::RemoteResponse {
                        status: 200,
                        request_id: None,
                        bytes: serde_json::to_vec(&fact).unwrap(),
                    })
                })
            }));
            let worker = service.clone();
            let mut captured = snapshot.clone();
            let task = tokio::spawn(async move { worker.reconcile_task(&mut captured).await });
            started.notified().await;
            if mode == "local-save" {
                service.retry_local_save(&snapshot.task.id).unwrap();
            } else if mode == "delete-image" {
                service
                    .delete_history(&snapshot.task.image_ids, true)
                    .unwrap();
            } else {
                service.commit_logout(&record.view.id).unwrap();
            }
            release.notify_one();
            let result = task.await.unwrap();
            let current = service
                .store
                .get::<TaskSnapshot>("task-snapshot", &snapshot.task.id)
                .unwrap()
                .unwrap();
            if mode == "logout" {
                assert!(result.is_err());
                assert_eq!(current.task.charge_state, "unavailable");
            } else {
                result.unwrap();
                assert_eq!(current.task.state, "completed");
                assert!(!current.task.local_save_available);
                assert_eq!(current.task.charge_state, "charged");
                assert_eq!(
                    current.task.image_ids.len(),
                    if mode == "delete-image" { 0 } else { 1 }
                );
                assert_eq!(current.task.prompt, "synthetic prompt");
            }
        }
    }
    #[tokio::test]
    async fn healthy_rotation_preserves_actor_and_draft_scope_but_failure_never_retries() {
        for fail_response in [false, true] {
            let (_dir, service, _vault, mut record, install) = setup();
            let mut session = service.session(&record).unwrap();
            session.expires_at = chrono::Utc::now().timestamp() - 1;
            record.expires_at = session.expires_at;
            service.save_session(&record, session).unwrap();
            service
                .store
                .put(
                    "connection",
                    &record.view.id,
                    &record.view.account_id,
                    &record,
                )
                .unwrap();
            let snapshot = task_snapshot(&record);
            service.persist_task(&snapshot).unwrap();
            let started = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let count = Arc::new(AtomicU64::new(0));
            let (a, b, c) = (started.clone(), release.clone(), count.clone());
            let token = golden()["token_reply"].clone();
            service.http.scripted(Arc::new(move |request| {
                let (a, b, c, token) = (a.clone(), b.clone(), c.clone(), token.clone());
                Box::pin(async move {
                    assert_eq!(request.method, Method::POST);
                    assert!(
                        !request
                            .headers
                            .iter()
                            .any(|(key, _)| key == "Authorization")
                    );
                    assert_eq!(
                        serde_json::from_slice::<Value>(&request.body).unwrap()["grant_type"],
                        "refresh_token"
                    );
                    c.fetch_add(1, Ordering::SeqCst);
                    a.notify_one();
                    b.notified().await;
                    if fail_response {
                        return Err(fail("result_unconfirmed"));
                    }
                    Ok(crate::network::RemoteResponse {
                        status: 200,
                        request_id: None,
                        bytes: serde_json::to_vec(&token).unwrap(),
                    })
                })
            }));
            let worker = service.clone();
            let id = record.view.id.clone();
            let task = tokio::spawn(async move { worker.fresh_session(&id).await.map(|_| ()) });
            started.notified().await;
            let view = service.bootstrap().unwrap();
            assert_eq!(view.session.status, "refreshing");
            assert_eq!(view.session.account.unwrap().id, record.view.account_id);
            assert_eq!(view.tasks[0].prompt, "synthetic prompt");
            assert!(
                service
                    .current_connection(&record.view.id, record.view.revision)
                    .is_err()
            );
            release.notify_one();
            let result = task.await.unwrap();
            if fail_response {
                assert!(result.is_err());
                assert_eq!(service.bootstrap().unwrap().session.status, "expired");
                assert!(service.fresh_session(&record.view.id).await.is_err());
            } else {
                result.unwrap();
                assert_eq!(service.bootstrap().unwrap().session.status, "authenticated");
                assert!(
                    service
                        .record(&record.view.id)
                        .unwrap()
                        .refresh_attempt
                        .is_none()
                );
            }
            assert_eq!(count.load(Ordering::SeqCst), 1);
            record = service.record(&record.view.id).unwrap();
            record.session_status = "refreshing".into();
            record.refresh_attempt = Some(uuid());
            service
                .store
                .put(
                    "connection",
                    &record.view.id,
                    &record.view.account_id,
                    &record,
                )
                .unwrap();
            let rebuilt = Service::new(
                service.store.clone(),
                service.vault.clone(),
                service.paths.clone(),
            )
            .unwrap();
            assert_eq!(
                rebuilt.record(&record.view.id).unwrap().session_status,
                "expired"
            );
            assert_eq!(rebuilt.installation(&record).unwrap().id, install.id);
        }
    }
    #[tokio::test]
    async fn finalized_facts_survive_following_local_save_delete_and_active_workers_defer_manual_lookup()
     {
        let (_dir, service, _vault, record, _install) = setup();
        let mut snapshot = task_snapshot(&record);
        snapshot.task.state = "save_failed".into();
        snapshot.task.local_save_available = true;
        service.persist_task(&snapshot).unwrap();
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2, 2)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        let bytes = out.into_inner();
        images::stage_received(
            &service.store,
            &snapshot,
            &[images::ReceivedImage {
                id: uuid(),
                info: images::validate_image(&bytes).unwrap(),
                bytes,
            }],
        )
        .unwrap();
        let wire = golden();
        let count = Arc::new(AtomicU64::new(0));
        let called = count.clone();
        service.http.scripted(Arc::new(move |request| {
            let (wire, called) = (wire.clone(), called.clone());
            Box::pin(async move {
                assert_eq!(request.method, Method::GET);
                let response = if request.url.ends_with("/bootstrap") {
                    wire["bootstrap_reply"].clone()
                } else {
                    called.fetch_add(1, Ordering::SeqCst);
                    wire["usage_lookup_reply"].clone()
                };
                Ok(crate::network::RemoteResponse {
                    status: 200,
                    request_id: None,
                    bytes: serde_json::to_vec(&response).unwrap(),
                })
            })
        }));
        // A cancelled public state cannot conceal a worker still waiting for its HTTP body.
        let permit = service.image_workers.clone().acquire_owned().await.unwrap();
        service
            .refresh_account(&record.view.id, record.view.revision)
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 0);
        drop(permit);
        service.reconcile_task(&mut snapshot).await.unwrap();
        assert_eq!(snapshot.task.charge_state, "charged");
        let saved = service.retry_local_save(&snapshot.task.id).unwrap();
        assert_eq!(saved.charge_state, "charged");
        service.delete_history(&saved.image_ids, true).unwrap();
        let current = service
            .store
            .get::<TaskSnapshot>("task-snapshot", &snapshot.task.id)
            .unwrap()
            .unwrap();
        assert_eq!(current.task.charge_state, "charged");
        assert!(current.task.image_ids.is_empty());
        assert_eq!(
            current.task.charges[0].amount.as_deref(),
            Some("0.00000000")
        );
    }
    #[tokio::test]
    async fn late_account_bootstrap_after_logout_or_reauthorization_cannot_replace_cache() {
        for reauthorize in [false, true] {
            let (_dir, service, _vault, record, install) = setup();
            let captured = service
                .account_cache(&record)
                .unwrap()
                .account
                .wallet
                .unwrap()
                .amount;
            let started = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let (a, b) = (started.clone(), release.clone());
            let mut reply = golden()["bootstrap_reply"].clone();
            reply["data"]["wallet"]["amount"] = json!("999.00000000");
            service.http.scripted(Arc::new(move |_| {
                let (a, b, reply) = (a.clone(), b.clone(), reply.clone());
                Box::pin(async move {
                    a.notify_one();
                    b.notified().await;
                    Ok(crate::network::RemoteResponse {
                        status: 200,
                        request_id: None,
                        bytes: serde_json::to_vec(&reply).unwrap(),
                    })
                })
            }));
            let worker = service.clone();
            let id = record.view.id.clone();
            let revision = record.view.revision;
            let task = tokio::spawn(async move { worker.refresh_account(&id, revision).await });
            started.notified().await;
            if reauthorize {
                let (_, session) = serde_json::from_value::<TokenResponse>(
                    golden()["token_reply"]["data"].clone(),
                )
                .unwrap()
                .into_session(&install)
                .unwrap();
                service
                    .commit_authorization(&install, record.view.account_id.clone(), session, 0)
                    .unwrap();
            } else {
                service.commit_logout(&record.view.id).unwrap();
            }
            release.notify_one();
            assert!(task.await.unwrap().is_err());
            assert_eq!(
                service
                    .account_cache(&record)
                    .unwrap()
                    .account
                    .wallet
                    .unwrap()
                    .amount,
                captured
            );
            assert!(
                service
                    .account_cache(&record)
                    .unwrap()
                    .account
                    .updated_at
                    .is_none()
            );
        }
    }
    #[tokio::test]
    async fn pending_login_cancel_terminal_events_and_late_callbacks_keep_existing_actor() {
        for mode in ["cancelled", "connected", "denied", "expired", "failed"] {
            let (_dir, service, vault, record, install) = setup();
            let events = Arc::new(Mutex::new(Vec::<AuthorizationEvent>::new()));
            let collected = events.clone();
            let finished = Arc::new(tokio::sync::Notify::new());
            let signal = finished.clone();
            service
                .observe_authorization(Arc::new(move |event| {
                    collected.lock().unwrap().push(event);
                    signal.notify_one();
                }))
                .unwrap();
            let captured = Arc::new(Mutex::new(None::<Value>));
            let request = captured.clone();
            let calls = Arc::new(AtomicU64::new(0));
            let call_count = calls.clone();
            let mut reply = golden()["start_reply"].clone();
            reply["data"]["expires_at"] = json!(
                (chrono::Utc::now()
                    + chrono::Duration::seconds(if mode == "expired" { 2 } else { 60 }))
                .to_rfc3339()
            );
            service.http.scripted(Arc::new(move |input| {
                let (request, call_count, reply) =
                    (request.clone(), call_count.clone(), reply.clone());
                Box::pin(async move {
                    call_count.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(input.method, Method::POST);
                    assert!(input.url.ends_with("/authorization-requests"));
                    let body = serde_json::from_slice::<Value>(&input.body).unwrap();
                    assert_eq!(body["code_challenge_method"], "S256");
                    assert!(
                        body["redirect_uri"]
                            .as_str()
                            .unwrap()
                            .contains("/orivelle-orbit/callback")
                    );
                    *request.lock().unwrap() = Some(body);
                    Ok(crate::network::RemoteResponse {
                        status: 200,
                        request_id: None,
                        bytes: serde_json::to_vec(&reply).unwrap(),
                    })
                })
            }));
            let (start, _url) = service.start_login(&install.origin).await.unwrap();
            let epoch = service.login_epoch.load(Ordering::SeqCst);
            let view = service.bootstrap().unwrap();
            assert_eq!(
                view.pending_authorization.unwrap().request_id,
                start.request_id
            );
            assert_eq!(view.session.status, "authenticated");
            assert_eq!(view.session.account.unwrap().id, record.view.account_id);
            if mode == "cancelled" {
                let view = service.cancel_login(&start.request_id).unwrap();
                assert!(view.pending_authorization.is_none());
                assert_eq!(view.session.account.unwrap().id, record.view.account_id);
                let (_, session) = serde_json::from_value::<TokenResponse>(
                    golden()["token_reply"]["data"].clone(),
                )
                .unwrap()
                .into_session(&install)
                .unwrap();
                assert_eq!(
                    service
                        .commit_authorization(
                            &install,
                            "synthetic-late-account".into(),
                            session,
                            epoch
                        )
                        .err()
                        .unwrap()
                        .code,
                    "authorization_cancelled"
                );
                assert!(
                    vault
                        .get(&format!(
                            "account/{}/{}/session",
                            install.id,
                            sha256(b"synthetic-late-account")
                        ))
                        .is_err()
                );
                assert_eq!(
                    service.record(&record.view.id).unwrap().view.revision,
                    record.view.revision
                );
            } else if mode == "connected" {
                let (_, session) = serde_json::from_value::<TokenResponse>(
                    golden()["token_reply"]["data"].clone(),
                )
                .unwrap()
                .into_session(&install)
                .unwrap();
                let connected = service
                    .commit_authorization(&install, record.view.account_id.clone(), session, epoch)
                    .unwrap();
                let view = service.cancel_login(&start.request_id).unwrap();
                assert!(view.pending_authorization.is_none());
                assert_eq!(view.session.status, "authenticated");
                assert_eq!(
                    service.record(&connected.view.id).unwrap().session_status,
                    "authenticated"
                );
                assert!(vault.get(&connected.session_reference).is_ok());
            } else if mode == "denied" {
                let body = captured.lock().unwrap().clone().unwrap();
                let mut callback = url::Url::parse(body["redirect_uri"].as_str().unwrap()).unwrap();
                callback
                    .query_pairs_mut()
                    .append_pair("state", body["state"].as_str().unwrap())
                    .append_pair("iss", &install.origin)
                    .append_pair("error", "access_denied");
                let mut socket =
                    tokio::net::TcpStream::connect(("127.0.0.1", callback.port().unwrap()))
                        .await
                        .unwrap();
                let message = format!(
                    "GET {}?{} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                    callback.path(),
                    callback.query().unwrap(),
                    callback.authority()
                );
                let mid = message.len() / 2;
                socket.write_all(&message.as_bytes()[..mid]).await.unwrap();
                tokio::task::yield_now().await;
                socket.write_all(&message.as_bytes()[mid..]).await.unwrap();
            } else if mode == "failed" {
                service
                    .fail_login(&start.request_id, fail("browser_unavailable"))
                    .unwrap();
            }
            tokio::time::timeout(Duration::from_secs(4), finished.notified())
                .await
                .unwrap();
            let events = events.lock().unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].request_id, start.request_id);
            assert_eq!(events[0].state, mode);
            assert!(service.bootstrap().unwrap().pending_authorization.is_none());
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            service.cancel_login(&start.request_id).unwrap();
        }
    }
    #[tokio::test]
    async fn update_maintenance_blocks_new_sends_and_snapshot_mutations_during_installer_await() {
        let (_dir, service, _vault, mut record, install) = setup();
        service
            .accept_credential(
                &mut record,
                &install,
                serde_json::from_value(golden()["credential_reply"]["data"].clone()).unwrap(),
            )
            .unwrap();
        let before = service.bootstrap().unwrap().state_revision;
        let calls = Arc::new(AtomicU64::new(0));
        let counted = calls.clone();
        service.http.scripted(Arc::new(move |_| {
            let counted = counted.clone();
            Box::pin(async move {
                counted.fetch_add(1, Ordering::SeqCst);
                Err(fail("unexpected_fixture_http"))
            })
        }));
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (a, b) = (started.clone(), release.clone());
        let installer = service.clone();
        let task = tokio::spawn(async move {
            let _lease = installer.begin_maintenance()?;
            a.notify_one();
            b.notified().await;
            Err::<(), PublicError>(fail("update_unavailable"))
        });
        started.notified().await;
        let view = service.bootstrap().unwrap();
        assert!(view.maintenance_active);
        assert!(view.state_revision > before);
        assert_eq!(
            service
                .create_image_task(
                    &record.view.id,
                    record.view.revision,
                    "synthetic",
                    "prompt".into(),
                    ImageParameters::default(),
                    vec![]
                )
                .unwrap_err()
                .code,
            "update_busy"
        );
        let mut snapshot = task_snapshot(&record);
        snapshot.task.state = "queued".into();
        snapshot.task.request_id = None;
        assert_eq!(
            service
                .send_image_once(&mut snapshot)
                .await
                .unwrap_err()
                .code,
            "update_busy"
        );
        assert_eq!(
            service
                .update_settings(json!({"locale":"en"}))
                .unwrap_err()
                .code,
            "update_busy"
        );
        assert_eq!(
            service.select_connection(&record.view.id).unwrap_err().code,
            "update_busy"
        );
        assert_eq!(
            service.preview_restore(&uuid()).unwrap_err().code,
            "update_busy"
        );
        assert_eq!(
            service
                .apply_client_config(&uuid(), true, true)
                .unwrap_err()
                .code,
            "update_busy"
        );
        assert_eq!(
            service.logout(&record.view.id).await.unwrap_err().code,
            "update_busy"
        );
        assert!(
            service
                .store
                .list::<TaskView>("task", None)
                .unwrap()
                .is_empty()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        release.notify_one();
        assert_eq!(task.await.unwrap().unwrap_err().code, "update_unavailable");
        let after = service.bootstrap().unwrap();
        assert!(!after.maintenance_active);
        assert!(after.state_revision > view.state_revision);
        service.update_settings(json!({"locale":"en"})).unwrap();
        let lease = service.begin_maintenance().unwrap();
        drop(lease);
        assert!(service.begin_mutation().is_ok());
    }
    #[tokio::test]
    async fn cancelled_inflight_workers_and_unsaved_material_still_block_installation() {
        let (_dir, service, _vault, mut record, install) = setup();
        service
            .accept_credential(
                &mut record,
                &install,
                serde_json::from_value(golden()["credential_reply"]["data"].clone()).unwrap(),
            )
            .unwrap();
        let mut snapshot = task_snapshot(&record);
        snapshot.task.state = "queued".into();
        snapshot.task.request_id = None;
        service.persist_task(&snapshot).unwrap();
        let permit = service.image_workers.clone().acquire_owned().await.unwrap();
        service.claim_image_send(&mut snapshot).unwrap();
        assert_eq!(
            service.cancel_image_task(&snapshot.task.id).unwrap().state,
            "unconfirmed"
        );
        assert_eq!(
            service.begin_maintenance().err().unwrap().code,
            "update_busy"
        );
        drop(permit);
        let lease = service.begin_maintenance().unwrap();
        drop(lease);
        let mut current = service
            .store
            .get::<TaskSnapshot>("task-snapshot", &snapshot.task.id)
            .unwrap()
            .unwrap();
        current.task.local_save_available = true;
        service.persist_task(&current).unwrap();
        assert_eq!(
            service.begin_maintenance().err().unwrap().code,
            "update_busy"
        );
        current.task.local_save_available = false;
        service.persist_task(&current).unwrap();
        let operation = service.begin_mutation().unwrap();
        assert_eq!(
            service.begin_maintenance().err().unwrap().code,
            "update_busy"
        );
        drop(operation);
        assert!(service.begin_maintenance().is_ok());
    }
    #[tokio::test]
    async fn pending_account_get_holds_snapshot_lease_until_response_is_committed() {
        let (_dir, service, _vault, record, _install) = setup();
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (a, b) = (started.clone(), release.clone());
        let reply = golden()["bootstrap_reply"].clone();
        service.http.scripted(Arc::new(move |_| {
            let (a, b, reply) = (a.clone(), b.clone(), reply.clone());
            Box::pin(async move {
                a.notify_one();
                b.notified().await;
                Ok(crate::network::RemoteResponse {
                    status: 200,
                    request_id: None,
                    bytes: serde_json::to_vec(&reply).unwrap(),
                })
            })
        }));
        let worker = service.clone();
        let id = record.view.id.clone();
        let revision = record.view.revision;
        let task = tokio::spawn(async move { worker.refresh_account(&id, revision).await });
        started.notified().await;
        assert_eq!(
            service.begin_maintenance().err().unwrap().code,
            "update_busy"
        );
        release.notify_one();
        task.await.unwrap().unwrap();
        let lease = service.begin_maintenance().unwrap();
        drop(lease);
        assert!(!service.bootstrap().unwrap().maintenance_active);
    }
    #[test]
    fn window_keep_running_is_only_an_available_macos_setting() {
        let (_dir, service, _vault, _record, _install) = setup();
        let result = service.update_settings(json!({"close_behavior":"keep_running"}));
        if cfg!(target_os = "macos") {
            assert_eq!(result.unwrap().close_behavior, "keep_running");
        } else {
            assert_eq!(result.unwrap_err().code, "invalid_settings");
        }
    }
    #[tokio::test]
    async fn expired_refresh_gate_does_not_send_again() {
        let (_d, service, _v, mut record, _i) = setup();
        record.session_status = "expired".into();
        record.refresh_attempt = Some(uuid());
        service
            .store
            .put(
                "connection",
                &record.view.id,
                &record.view.account_id,
                &record,
            )
            .unwrap();
        for _ in 0..2 {
            assert!(
                matches!(service.fresh_session(&record.view.id).await,Err(e)if e.code=="session_expired")
            );
        }
    }
    #[test]
    fn logout_between_refresh_response_and_commit_cannot_revive_session() {
        let (_d, service, vault, mut record, install) = setup();
        record.session_status = "refreshing".into();
        record.refresh_attempt = Some(uuid());
        service
            .store
            .put(
                "connection",
                &record.view.id,
                &record.view.account_id,
                &record,
            )
            .unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let worker_barrier = barrier.clone();
        let worker = service.clone();
        let id = record.view.id.clone();
        let wire = golden();
        let (_, new_session) =
            serde_json::from_value::<TokenResponse>(wire["token_reply"]["data"].clone())
                .unwrap()
                .into_session(&install)
                .unwrap();
        let thread = std::thread::spawn(move || {
            worker_barrier.wait();
            worker.commit_refresh(&mut record, new_session)
        });
        service.commit_logout(&id).unwrap();
        barrier.wait();
        assert!(matches!(thread.join().unwrap(),Err(e)if e.code=="connection_conflict"));
        let logged_out = service.record(&id).unwrap();
        assert_eq!(logged_out.session_status, "logged_out");
        assert!(vault.get(&logged_out.session_reference).is_err());
    }
    #[test]
    fn bootstrap_snapshot_pairs_actor_with_its_own_revision_and_wallet() {
        let (_d, service, _v, record, _i) = setup();
        let mut other = record.clone();
        other.view.id = uuid();
        other.view.account_id = "another".into();
        other.view.label = "another".into();
        other.session_status = "authenticated".into();
        let mut cache = service.account_cache(&record).unwrap();
        cache.account.id = "another".into();
        cache.account.label = "another".into();
        cache.account.wallet = Some(Money {
            amount: Some("99.00000000".into()),
            currency: Some("USD".into()),
            status: "known".into(),
        });
        service
            .store
            .put("connection", &other.view.id, "another", &other)
            .unwrap();
        service
            .store
            .put("account-cache", &other.view.id, "another", &cache)
            .unwrap();
        let writer = service.clone();
        let first = record.view.id.clone();
        let second = other.view.id.clone();
        let thread = std::thread::spawn(move || {
            for i in 0..40 {
                writer
                    .select_connection(if i % 2 == 0 { &first } else { &second })
                    .unwrap();
            }
        });
        let mut last = 0;
        for _ in 0..40 {
            let view = service.bootstrap().unwrap();
            assert!(view.state_revision >= last);
            last = view.state_revision;
            let active = view
                .connections
                .iter()
                .find(|c| Some(&c.id) == view.active_connection_id.as_ref())
                .unwrap();
            let account = view.session.account.unwrap();
            assert_eq!(active.account_id, account.id);
            assert_eq!(
                account.wallet.unwrap().amount.as_deref(),
                Some(if account.id == "another" {
                    "99.00000000"
                } else {
                    "12.34567890"
                })
            );
        }
        thread.join().unwrap();
    }
    #[tokio::test]
    async fn forget_last_connection_installation_still_retained_backup_only() {
        for phase in ["committed", "publishing", "conflict"] {
            let (_d, service, vault, record, install) = setup();
            let path = service.paths.codex_config.clone();
            std::fs::write(&path, b"model='old'\n").unwrap();
            let mut journal = service
                .config_engine()
                .prepare(
                    ClientKind::Codex,
                    &record.view.account_id,
                    &install,
                    vec![FileChange {
                        path: path.display().to_string(),
                        before: Some(b"model='old'\n".to_vec()),
                        after: Some(b"model='new'\n".to_vec()),
                    }],
                    None,
                    vec![],
                )
                .unwrap();
            if phase == "committed" {
                service.config_engine().apply(&journal.id).unwrap();
            } else {
                journal.phase = phase.into();
                service
                    .store
                    .put("journal", &journal.id, &journal.account_id, &journal)
                    .unwrap();
            }
            service.store.delete("connection", &record.view.id).unwrap();
            service.store.set_meta("active_connection", "").unwrap();
            let bootstrap = service.bootstrap().unwrap();
            assert_eq!(bootstrap.installations.len(), 1);
            assert_eq!(bootstrap.backups.len(), 1);
            let preview = service.preview_forget_installation(&install.id).unwrap();
            assert_eq!(preview.backup_group_count, 1);
            let after = service
                .forget_installation(&preview.id, true, true)
                .await
                .unwrap();
            assert!(after.installations.is_empty());
            assert!(after.backups.is_empty());
            assert!(vault.get(&install.backup_reference).is_err());
        }
    }
}
