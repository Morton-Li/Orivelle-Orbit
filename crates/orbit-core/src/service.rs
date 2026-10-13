use crate::{
    config::{ConfigEngine, FileChange, Journal, codex_patch, read_file},
    dto::*,
    error::{Result, fail},
    identity::{
        HelperBinding, InferenceSecret, Installation, authorization_epoch_reference,
        bound_helper_token, canonical_origin, sha256, uuid, validate_uuid,
    },
    images::{self, InputRecord, TaskSnapshot},
    network::{CredentialDelivery, Http, ProofSession, TokenResponse},
    store::Store,
    system::{SystemPaths, detect_clients},
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
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Semaphore,
};
use zeroize::Zeroizing;

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
}
struct ForgetEntry {
    view: ForgetInstallationPreview,
    installation_id: String,
    account_ids: Vec<String>,
    backup_ids: Vec<String>,
    expires_at: i64,
}
type Observer = Arc<dyn Fn(BootstrapView) + Send + Sync>;
pub struct Service {
    pub store: Arc<Store>,
    pub vault: Arc<dyn Vault>,
    pub http: Http,
    pub paths: SystemPaths,
    state_revision: AtomicU64,
    login_epoch: AtomicU64,
    account_gate: Mutex<()>,
    clients: Mutex<Vec<ClientView>>,
    previews: Mutex<HashMap<String, PreviewEntry>>,
    forgets: Mutex<HashMap<String, ForgetEntry>>,
    diagnostics: Mutex<HashMap<String, DiagnosticsPreview>>,
    observer: Mutex<Option<Observer>>,
    cancelled: Mutex<HashSet<String>>,
    image_workers: Arc<Semaphore>,
    image_memory_workers: Arc<Semaphore>,
    session_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    received_memory: Mutex<HashMap<String, Vec<images::ReceivedImage>>>,
}
impl Service {
    pub fn new(store: Arc<Store>, vault: Arc<dyn Vault>, paths: SystemPaths) -> Result<Arc<Self>> {
        let clients = detect_clients(&paths);
        store.set_meta("preview_runtime_nonce", &uuid())?;
        for client in &clients {
            store.put("client-state", client.client.as_str(), "local", client)?;
        }
        Ok(Arc::new(Self {
            store,
            vault,
            http: Http::new()?,
            paths,
            state_revision: AtomicU64::new(1),
            login_epoch: AtomicU64::new(0),
            account_gate: Mutex::new(()),
            clients: Mutex::new(clients),
            previews: Mutex::new(HashMap::new()),
            forgets: Mutex::new(HashMap::new()),
            diagnostics: Mutex::new(HashMap::new()),
            observer: Mutex::new(None),
            cancelled: Mutex::new(HashSet::new()),
            image_workers: Arc::new(Semaphore::new(2)),
            image_memory_workers: Arc::new(Semaphore::new(2)),
            session_locks: Mutex::new(HashMap::new()),
            received_memory: Mutex::new(HashMap::new()),
        }))
    }
    pub fn observe(&self, observer: Observer) -> Result<()> {
        *self
            .observer
            .lock()
            .map_err(|_| fail("state_unavailable"))? = Some(observer);
        Ok(())
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
            if session.status == "authenticated"
                && record
                    .session_expires_at
                    .is_some_and(|at| at <= chrono::Utc::now().timestamp())
            {
                session.status = "expired".into();
            }
            session.panel_origin = Some(record.view.panel_origin.clone());
            if let Some(cache) = snapshot.get::<AccountCache>("account-cache", &record.view.id)? {
                if session.status == "authenticated" {
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
            if session.status == "authenticated" {
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
        for j in snapshot.list::<Journal>("backup", None)? {
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
                    state: if j.phase == "conflict" {
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
        Ok(BootstrapView {
            schema_version: 1,
            state_revision: snapshot.revision,
            source: "local".into(),
            session,
            connections: records.into_iter().map(|r| r.view).collect(),
            installations,
            nodes,
            active_connection_id: active.map(|r| r.view.id),
            clients: snapshot.list("client-state", Some("local"))?,
            settings: snapshot.get("settings", "local")?.unwrap_or_default(),
            tasks,
            image_history: history,
            backups,
        })
    }
    pub async fn start_login(self: &Arc<Self>, panel_origin: &str) -> Result<(LoginStart, String)> {
        let epoch = self.login_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let origin = canonical_origin(panel_origin)?;
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
        let seconds = (expire - chrono::Utc::now().timestamp()).clamp(1, 600) as u64;
        let view = LoginStart {
            request_id: start.authorization_request_id.clone(),
            expires_at: start.expires_at,
        };
        let service = self.clone();
        tokio::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(seconds),
                service.finish_login(
                    listener,
                    installation,
                    start.authorization_request_id,
                    start.proof_nonce,
                    state,
                    verifier,
                    redirect_uri,
                    epoch,
                ),
            )
            .await;
            if !matches!(result, Ok(Ok(()))) {
                service.changed();
            }
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
        let count = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut request))
            .await
            .map_err(|_| fail("invalid_callback"))?
            .map_err(|_| fail("invalid_callback"))?;
        if count > 8192 {
            return Err(fail("invalid_callback"));
        }
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
        let ticket = session.credential_delivery_ticket.clone();
        {
            let _gate = self
                .account_gate
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
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
            record.authorization_epoch = self.advance_authorization_epoch(&record)?;
            self.save_session(&record, session)?;
            self.store
                .disable_bindings(&account_id, &installation.origin)?;
            self.store.put("connection", &id, &account_id, &record)?;
        }
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
        self.refresh_account(&record.view.id, false).await?;
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
    fn accept_credential(
        &self,
        record: &mut ConnectionRecord,
        installation: &Installation,
        delivery: CredentialDelivery,
    ) -> Result<()> {
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
        if let Some(old) = record.credential_reference.take() {
            if old != reference {
                let _ = self.vault.delete(&old);
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
        Ok(())
    }
    async fn fresh_session(
        &self,
        id: &str,
    ) -> Result<(ConnectionRecord, Installation, ProofSession)> {
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
                record.session_status = "expired".into();
                record.refresh_attempt = Some(uuid());
                self.store
                    .put("connection", id, &record.view.account_id, &record)?;
            }
            self.changed();
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
            self.commit_refresh(&mut record, new_session)?;
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
        let current = self.record(&record.view.id)?;
        self.ensure_installation_available(&record.installation_id)?;
        if current.view.revision != record.view.revision
            || current.session_status != "expired"
            || current.refresh_attempt != record.refresh_attempt
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
        let _ = self.cleanup_pending(record)?;
        self.invalidate_evidence()
    }
    pub async fn refresh_account(&self, id: &str, activate: bool) -> Result<()> {
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
        let cache = map_account_cache(&value, &record)?;
        {
            let _gate = self
                .account_gate
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            let mut current = self.record(id)?;
            if current.session_status != "authenticated"
                || current.view.revision != record.view.revision
                || current.installation_id != record.installation_id
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
    pub fn select_connection(&self, id: &str) -> Result<BootstrapView> {
        self.login_epoch.fetch_add(1, Ordering::SeqCst);
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
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
        Ok(self
            .store
            .list::<TaskView>("task", None)?
            .iter()
            .any(|t| ["queued", "sending", "waiting", "saving"].contains(&t.state.as_str())))
    }
    pub fn select_node(
        &self,
        id: &str,
        node_id: &str,
        expected_revision: u64,
    ) -> Result<BootstrapView> {
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
        let secret_failure = self.cleanup_pending(&mut record)?;
        self.invalidate_evidence()?;
        if secret_failure {
            return Err(fail("vault_unavailable"));
        }
        Ok(())
    }
    pub async fn logout(&self, id: &str) -> Result<BootstrapView> {
        self.login_epoch.fetch_add(1, Ordering::SeqCst);
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
        }
    }
    pub fn preview_client_config(
        &self,
        id: &str,
        revision: u64,
        client: ClientKind,
        model_id: &str,
    ) -> Result<ConfigPreview> {
        let (record, cache) = self.current_connection(id, revision)?;
        let model = cache
            .models
            .iter()
            .find(|m| {
                m.id == model_id && m.category == "text" && m.provider_id == record.view.provider_id
            })
            .ok_or_else(|| fail("model_unavailable"))?;
        let context = self.context(&record, &cache, model)?;
        let target = self
            .clients
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .iter()
            .find(|c| c.client == client)
            .cloned()
            .ok_or_else(|| fail("platform_unsupported"))?;
        let now = chrono::Utc::now();
        let mut preview = ConfigPreview {
            id: uuid(),
            client,
            operation: "apply".into(),
            target_version: target.version.clone(),
            paths: target.config_paths.clone(),
            model_id: Some(model_id.into()),
            context: Some(context),
            restore_owner: None,
            impact: match client {
                ClientKind::Codex => vec![
                    "修改用户默认模型和 Orivelle provider；Codex、CLI 与 IDE 可能共用此配置。"
                        .into(),
                    "应用后需重启；保存配置不会自动发送推理请求。".into(),
                ],
                ClientKind::Claude => vec![
                    "第三方模式影响 Claude 整个桌面应用，包括 Chat、Code 与 Cowork。".into(),
                    "保留官方模式选择、组织策略和原登录资料。".into(),
                ],
            },
            redacted_diff: "No file mutation is permitted for this preview.".into(),
            expires_at: (now + chrono::Duration::minutes(5)).to_rfc3339(),
            can_apply: false,
            can_export_import: false,
            blocking_reason: None,
        };
        let mut journal_id = None;
        let mut import_candidate = None;
        if !self.paths.writes_authorized {
            preview.blocking_reason = Some("integration_authorization_required".into());
        } else if !target.installed || !target.platform_supported || target.version.is_none() {
            preview.blocking_reason = Some("client_unsupported".into());
        } else if target.managed != "no" {
            preview.blocking_reason = Some("managed_policy_unverified".into());
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
            if client == ClientKind::Codex {
                self.paths.ensure_configuration_parent(client)?;
            }
            let install = self.installation(&record)?;
            let binding_id = uuid();
            let credential_ref = record
                .credential_reference
                .clone()
                .ok_or_else(|| fail("credential_missing"))?;
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
                secret_reference: credential_ref,
                enabled: false,
                expires_at: record.credential_expires_at,
                authorization_epoch: record.authorization_epoch,
            };
            let mut checked = binding.clone();
            checked.enabled = true;
            let _ = bound_helper_token(
                &checked,
                client,
                &binding_id,
                &self.store,
                self.vault.as_ref(),
                now.timestamp(),
            )?;
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
                let before = read_file(&self.paths.codex_config)?;
                let after = codex_patch(
                    before.as_deref(),
                    record
                        .view
                        .edge_base_url
                        .as_ref()
                        .ok_or_else(|| fail("node_unavailable"))?,
                    model_id,
                    &self.paths.helper,
                    &binding_id,
                )?;
                preview.redacted_diff =
                    crate::config::codex_redacted_diff(before.as_deref(), &after)?;
                let previous = self
                    .store
                    .list::<HelperBinding>("binding", None)?
                    .into_iter()
                    .filter(|b| b.client == client && b.enabled)
                    .map(|b| b.id)
                    .collect();
                let mut journal = self.config_engine().prepare(
                    client,
                    &record.view.account_id,
                    &install,
                    vec![FileChange {
                        path: self.paths.codex_config.display().to_string(),
                        before,
                        after: Some(after),
                    }],
                    Some(binding_id),
                    previous,
                )?;
                journal.account_label = cache.account.label.clone();
                self.store
                    .put("journal", &journal.id, &record.view.account_id, &journal)?;
                journal_id = Some(journal.id);
                preview.can_apply = true;
            }
        }
        if client == ClientKind::Claude {
            preview.impact.push("仅导出官方 V2 正文诊断候选；不写 configLibrary envelope/_meta，需在 Claude 官方第三方模式手工导入并重启，尚无真实加载/连通证据。".into());
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
    fn validate_preview_context(&self, preview: &ConfigPreview) -> Result<()> {
        let target = self
            .clients
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .iter()
            .find(|c| c.client == preview.client)
            .cloned()
            .ok_or_else(|| fail("client_unsupported"))?;
        if target.version != preview.target_version || !target.installed || target.managed != "no" {
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
    pub fn apply_client_config(
        &self,
        id: &str,
        confirmed: bool,
        restore: bool,
    ) -> Result<ConfigResult> {
        if !confirmed {
            return Err(fail("confirmation_required"));
        }
        if !self.paths.writes_authorized {
            return Err(fail("integration_authorization_required"));
        }
        let entry = self.take_preview(id, if restore { "restore" } else { "apply" })?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let _ = self.validate_preview_context(&entry.view)?;
        let journal = self.config_engine().apply(
            entry
                .journal_id
                .as_deref()
                .ok_or_else(|| fail("preview_expired"))?,
        )?;
        let revision = read_file(&self.paths.codex_config)?.map(|b| sha256(&b));
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
        if !path.is_absolute()
            || path == self.paths.codex_config
            || path.starts_with(&self.paths.claude_library)
            || (self.paths.isolated && !path.starts_with(&self.store.root))
        {
            return Err(fail("invalid_path"));
        }
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        self.validate_preview_context(&entry.view)?;
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
        let _ = bound_helper_token(
            &binding,
            ClientKind::Claude,
            &binding_id,
            &self.store,
            self.vault.as_ref(),
            chrono::Utc::now().timestamp(),
        )?;
        crate::config::atomic_write(path, &bytes)?;
        self.store
            .put("binding", &binding_id, &binding.account_id, &binding)?;
        // Exporting a diagnostic candidate is not applying, loading, or observing
        // a Claude configuration. Public client evidence therefore remains absent.
        self.store.put("manual-import",&binding_id,&binding.account_id,&json!({"client":"claude","origin":binding.origin,"installation_id":binding.installation_id,"configuration_hash":sha256(&bytes),"state":"diagnostic_exported","observed_at":chrono::Utc::now().to_rfc3339()}))?;
        self.changed();
        Ok(())
    }
    pub fn preview_restore(&self, backup_id: &str) -> Result<ConfigPreview> {
        validate_uuid(backup_id)?;
        let backup = self
            .store
            .get::<Journal>("backup", backup_id)?
            .ok_or_else(|| fail("backup_invalid"))?;
        let install = self
            .store
            .installation(&backup.installation.origin)?
            .ok_or_else(|| fail("backup_invalid"))?;
        if install.id != backup.installation.id {
            return Err(fail("account_scope_mismatch"));
        }
        let target = self
            .clients
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .iter()
            .find(|c| c.client == backup.client)
            .cloned()
            .ok_or_else(|| fail("client_unsupported"))?;
        let authorized = self.paths.writes_authorized
            && backup.client == ClientKind::Codex
            && target.installed
            && target.platform_supported
            && target.managed == "no";
        let journal_id = if authorized {
            Some(self.config_engine().prepare_restore(backup_id)?.id)
        } else {
            None
        };
        let preview = ConfigPreview {
            id: uuid(),
            client: backup.client,
            operation: "restore".into(),
            target_version: target.version,
            paths: backup.paths,
            model_id: None,
            context: None,
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
                "恢复不依赖当前推理 key、节点或会话；第三方修改时整组停止覆盖。".into(),
            ],
            redacted_diff:
                "Restore encrypted original configuration; original secrets stay redacted.".into(),
            expires_at: (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339(),
            can_apply: authorized,
            can_export_import: false,
            blocking_reason: if authorized {
                None
            } else {
                Some(
                    if backup.client == ClientKind::Claude {
                        "schema_unverified"
                    } else if !self.paths.writes_authorized {
                        "integration_authorization_required"
                    } else {
                        "client_unsupported"
                    }
                    .into(),
                )
            },
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
                },
            );
        Ok(preview)
    }
    pub fn import_inputs(&self, id: &str, paths: &[PathBuf]) -> Result<Vec<ImageInput>> {
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
        apply_server_fact(&mut snapshot.task, &fact)?;
        self.persist_task(snapshot)
    }
    pub fn cancel_image_task(&self, id: &str) -> Result<TaskView> {
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
    pub fn image_for_native_export(&self, id: &str) -> Result<(HistoryImage, PathBuf)> {
        let (image, _) = self.scoped_image(id)?;
        let name = self
            .store
            .get::<String>("image-file", id)?
            .ok_or_else(|| fail("local_image_missing"))?;
        Ok((image, images::owned_image_path(&self.store, &name)?))
    }
    pub fn export_image(&self, id: &str, target: &Path) -> Result<()> {
        let _permit = self.image_memory_permit()?;
        let (_, source) = self.image_for_native_export(id)?;
        if !target.is_absolute() || source == target {
            return Err(fail("invalid_path"));
        }
        let bytes = images::bounded_read(&source, images::MAX_IMAGE_BYTES)?;
        let _ = images::validate_image(&bytes)?;
        crate::config::atomic_write(target, &bytes)
    }
    pub fn delete_history(&self, ids: &[String], confirmed: bool) -> Result<u32> {
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
        validate_uuid(id)?;
        let diagnostics = self
            .diagnostics
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .remove(id)
            .ok_or_else(|| fail("preview_expired"))?;
        crate::config::atomic_write(path, diagnostics.text.as_bytes())
    }
    pub fn preview_forget_installation(&self, id: &str) -> Result<ForgetInstallationPreview> {
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
        let mut accounts = self
            .store
            .list::<ConnectionRecord>("connection", None)?
            .into_iter()
            .filter(|r| r.installation_id == id && r.view.panel_origin == installation.origin)
            .map(|r| r.view.label)
            .chain(
                self.store
                    .list::<Journal>("backup", None)?
                    .into_iter()
                    .filter(|b| b.installation.id == id)
                    .map(|b| b.account_label),
            )
            .collect::<Vec<_>>();
        accounts.sort();
        accounts.dedup();
        let backup_records = self
            .store
            .list::<Journal>("backup", None)?
            .into_iter()
            .filter(|b| b.installation.id == id)
            .collect::<Vec<_>>();
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
            let current_backups = self
                .store
                .list::<Journal>("backup", None)?
                .into_iter()
                .filter(|b| b.installation.id == installation.id)
                .collect::<Vec<_>>();
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
            self.login_epoch.fetch_add(1, Ordering::SeqCst);
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
    let category = text(value, "model_category")?;
    let image = category == "image";
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
        id: text(value, "id")?,
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
        assert!(service.image_for_native_export(&image.id).is_err());
        assert!(service.read_preview_asset("/../../orbit.sqlite3").is_err());
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
            .preview_client_config(
                &record.view.id,
                record.view.revision,
                ClientKind::Claude,
                &model,
            )
            .unwrap();
        assert!(!blocked.can_apply && !blocked.can_export_import);
        let mut paths = original.paths.clone();
        paths.writes_authorized = true;
        std::fs::write(&paths.helper, b"synthetic helper fixture - never executed").unwrap();
        paths.helper_expected_sha256 = Some(sha256(b"synthetic helper fixture - never executed"));
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
            client.version = Some("2.19675.0-synthetic".into());
            client.managed = "no".into();
            service
                .store
                .put("client-state", "claude", "local", client)
                .unwrap();
        }
        let preview = service
            .preview_client_config(
                &record.view.id,
                record.view.revision,
                ClientKind::Claude,
                &model,
            )
            .unwrap();
        assert!(!preview.can_apply && preview.can_export_import);
        assert_eq!(
            preview.blocking_reason.as_deref(),
            Some("schema_unverified")
        );
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
            .preview_client_config(
                &record.view.id,
                record.view.revision,
                ClientKind::Claude,
                &model,
            )
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
        let (_d, service, vault, record, install) = setup();
        let path = service.paths.codex_config.clone();
        std::fs::write(&path, b"model='old'\n").unwrap();
        let journal = service
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
        service.config_engine().apply(&journal.id).unwrap();
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
