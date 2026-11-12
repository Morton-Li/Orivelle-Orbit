//! One opt-in native component test. Panel responses are scripted; the actual
//! default user config and four fresh NativeVault entries are real. The open
//! Desktop may observe the temporary shared defaults. No helper/client is run.

use super::*;
use crate::vault::NativeVault;
use fs2::FileExt;
use futures_util::FutureExt;
use std::{
    collections::VecDeque,
    ffi::{CString, c_void},
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    panic::AssertUnwindSafe,
    ptr,
};

type ProbeResult<T> = std::result::Result<T, &'static str>;
const SERVICE: &str = "io.orivelle.orbit";
const OPT_IN: &str = "CONFIRM_SHARED_CODEX_CONFIG_WRITE_AND_RESTORE";

// Locked apple-native-keyring-store 1.0.2 inherits keyring-core 1.0.1's
// get_attributes -> get_secret default; get_credential also reads a password.
// Do not call either on a potential collision. This SDK query is attributes-only,
// exact service/account and User-default keychain, matching NativeVault's store.
type Ref = *const c_void;
struct Owned(Ref);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: Ref);
    fn CFStringCreateWithCString(
        allocator: Ref,
        value: *const std::ffi::c_char,
        encoding: u32,
    ) -> Ref;
    fn CFArrayCreate(allocator: Ref, values: *const Ref, count: isize, callbacks: Ref) -> Ref;
    fn CFDictionaryCreate(
        allocator: Ref,
        keys: *const Ref,
        values: *const Ref,
        count: isize,
        key_callbacks: Ref,
        value_callbacks: Ref,
    ) -> Ref;
    static kCFBooleanTrue: Ref;
    static kCFBooleanFalse: Ref;
}
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecKeychainCopyDomainDefault(domain: i32, keychain: *mut Ref) -> i32;
    fn SecItemCopyMatching(query: Ref, result: *mut Ref) -> i32;
    static kSecClass: Ref;
    static kSecClassGenericPassword: Ref;
    static kSecAttrService: Ref;
    static kSecAttrAccount: Ref;
    static kSecMatchSearchList: Ref;
    static kSecMatchLimit: Ref;
    static kSecMatchLimitOne: Ref;
    static kSecReturnAttributes: Ref;
    static kSecReturnData: Ref;
    static kSecUseAuthenticationUI: Ref;
    static kSecUseAuthenticationUIFail: Ref;
}
fn exact_absence(reference: &str) -> ProbeResult<()> {
    let service = CString::new(SERVICE).map_err(|_| "absence_input_invalid")?;
    let account = CString::new(reference).map_err(|_| "absence_input_invalid")?;
    let mut keychain = ptr::null();
    let status = unsafe { SecKeychainCopyDomainDefault(0, &mut keychain) };
    let keychain = Owned(keychain);
    if status != 0 || keychain.0.is_null() {
        return Err("absence_keychain_unavailable");
    }
    let service =
        Owned(unsafe { CFStringCreateWithCString(ptr::null(), service.as_ptr(), 0x08000100) });
    let account =
        Owned(unsafe { CFStringCreateWithCString(ptr::null(), account.as_ptr(), 0x08000100) });
    let search = Owned(unsafe { CFArrayCreate(ptr::null(), &keychain.0, 1, ptr::null()) });
    if service.0.is_null() || account.0.is_null() || search.0.is_null() {
        return Err("absence_query_unavailable");
    }
    let keys = unsafe {
        [
            kSecClass,
            kSecAttrService,
            kSecAttrAccount,
            kSecMatchSearchList,
            kSecMatchLimit,
            kSecReturnAttributes,
            kSecReturnData,
            kSecUseAuthenticationUI,
        ]
    };
    let values = unsafe {
        [
            kSecClassGenericPassword,
            service.0,
            account.0,
            search.0,
            kSecMatchLimitOne,
            kCFBooleanTrue,
            kCFBooleanFalse,
            kSecUseAuthenticationUIFail,
        ]
    };
    let query = Owned(unsafe {
        CFDictionaryCreate(
            ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            keys.len() as isize,
            ptr::null(),
            ptr::null(),
        )
    });
    if query.0.is_null() {
        return Err("absence_query_unavailable");
    }
    let mut result = ptr::null();
    let status = unsafe { SecItemCopyMatching(query.0, &mut result) };
    let _result = Owned(result); // Release without inspecting any returned attributes.
    match status {
        -25300 => Ok(()), // errSecItemNotFound, not an error collapsed by Vault.
        0 => Err("fresh_reference_collision"),
        _ => Err("fresh_reference_absence_unverified"),
    }
}

#[derive(Serialize)]
struct ReferenceState {
    kind: &'static str,
    reference: String,
    absent: bool,
    write_attempted: bool,
    deleted: bool,
}
#[derive(Serialize)]
struct RecoveryRecord {
    run_id: String,
    step: &'static str,
    state: &'static str,
    store: &'static str,
    references: Vec<ReferenceState>,
    snapshot_id: Option<String>,
    original_present: [bool; 2],
    original_modes: [Option<u32>; 2],
    exact_restore_proven: bool,
    root_retained: bool,
    last_error: Option<&'static str>,
    #[serde(skip)]
    path: PathBuf,
}
impl RecoveryRecord {
    fn persist(&self) -> ProbeResult<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|_| "recovery_record_encode_failed")?;
        crate::config::atomic_write(&self.path, &bytes).map_err(|_| "recovery_record_write_failed")
    }
}
type SharedRecord = Arc<Mutex<RecoveryRecord>>;
fn record_step(record: &SharedRecord, step: &'static str, state: &'static str) -> ProbeResult<()> {
    let mut record = record.lock().map_err(|_| "recovery_record_unavailable")?;
    record.step = step;
    record.state = state;
    record.persist()
}

/// No namespace access is permitted. Every API access must name one of four
/// predeclared refs; all four raw absence checks finish before any native put.
struct OwnedNativeVault {
    native: NativeVault,
    record: SharedRecord,
}
impl OwnedNativeVault {
    fn arm(&self) -> ProbeResult<()> {
        let mut record = self
            .record
            .lock()
            .map_err(|_| "recovery_record_unavailable")?;
        for item in &mut record.references {
            exact_absence(&item.reference)?;
            item.absent = true;
        }
        record.step = "four_exact_references_absent";
        record.state = "armed";
        record.persist()
    }
    fn delete_owned(&self) -> ProbeResult<()> {
        let refs = self
            .record
            .lock()
            .map_err(|_| "recovery_record_unavailable")?
            .references
            .iter()
            .filter(|item| item.write_attempted)
            .map(|item| item.reference.clone())
            .collect::<Vec<_>>();
        for reference in refs {
            self.delete(&reference)
                .map_err(|_| "owned_keychain_cleanup_failed")?;
            exact_absence(&reference)?;
        }
        Ok(())
    }
}
impl Vault for OwnedNativeVault {
    fn get(&self, reference: &str) -> Result<Zeroizing<Vec<u8>>> {
        let record = self
            .record
            .lock()
            .map_err(|_| fail("native_probe_scope_unavailable"))?;
        if !record
            .references
            .iter()
            .any(|item| item.reference == reference && item.absent && item.write_attempted)
        {
            return Err(fail("native_probe_reference_outside_scope"));
        }
        self.native.get(reference)
    }
    fn put(&self, reference: &str, value: &[u8]) -> Result<()> {
        let mut record = self
            .record
            .lock()
            .map_err(|_| fail("native_probe_scope_unavailable"))?;
        if !record.references.iter().all(|item| item.absent) {
            return Err(fail("native_probe_absence_unverified"));
        }
        let item = record
            .references
            .iter_mut()
            .find(|item| item.reference == reference)
            .ok_or_else(|| fail("native_probe_reference_outside_scope"))?;
        item.write_attempted = true; // Durable ownership precedes a possibly ambiguous put.
        item.deleted = false;
        record
            .persist()
            .map_err(|_| fail("native_probe_recovery_record_failed"))?;
        self.native.put(reference, value)
    }
    fn delete(&self, reference: &str) -> Result<()> {
        let mut record = self
            .record
            .lock()
            .map_err(|_| fail("native_probe_scope_unavailable"))?;
        if !record
            .references
            .iter()
            .any(|item| item.reference == reference && item.absent && item.write_attempted)
        {
            return Err(fail("native_probe_reference_outside_scope"));
        }
        self.native.delete(reference)?;
        record
            .references
            .iter_mut()
            .find(|item| item.reference == reference)
            .ok_or_else(|| fail("native_probe_reference_outside_scope"))?
            .deleted = true;
        record
            .persist()
            .map_err(|_| fail("native_probe_recovery_record_failed"))
    }
}

struct OriginalFile {
    bytes: Option<Zeroizing<Vec<u8>>>,
    mode: Option<u32>,
}
fn file_identity(metadata: &fs::Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
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
fn capture(path: &Path) -> ProbeResult<OriginalFile> {
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(OriginalFile {
                bytes: None,
                mode: None,
            });
        }
        Err(_) => return Err("original_file_unavailable"),
    };
    let before = file
        .metadata()
        .map_err(|_| "original_metadata_unavailable")?;
    if !before.is_file()
        || before.uid() != unsafe { libc::geteuid() }
        || before.len() > crate::config::MAX_CONFIG_BYTES
    {
        return Err("original_file_scope_unverified");
    }
    let mut bytes = Zeroizing::new(Vec::new());
    (&mut file)
        .take(crate::config::MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "original_read_failed")?;
    let after = file
        .metadata()
        .map_err(|_| "original_metadata_unavailable")?;
    let current = fs::symlink_metadata(path).map_err(|_| "original_metadata_unavailable")?;
    if bytes.len() as u64 > crate::config::MAX_CONFIG_BYTES
        || !current.is_file()
        || file_identity(&before) != file_identity(&after)
        || file_identity(&before) != file_identity(&current)
    {
        return Err("original_changed_during_capture");
    }
    Ok(OriginalFile {
        bytes: Some(bytes),
        mode: Some(before.permissions().mode() & 0o7777),
    })
}
fn same_bytes(original: &OriginalFile, current: &OriginalFile) -> bool {
    original.bytes.as_ref().map(|bytes| bytes.as_slice())
        == current.bytes.as_ref().map(|bytes| bytes.as_slice())
}
fn restore_mode(path: &Path, original: &OriginalFile) -> ProbeResult<()> {
    let current = capture(path)?;
    if !same_bytes(original, &current) {
        return Err("restore_has_external_file_state");
    }
    let Some(mode) = original.mode else {
        return Ok(());
    };
    if current.mode == Some(mode) {
        return Ok(());
    }
    // The product's standard restore publishes private 0600 files. Only this
    // acceptance cleanup restores an original mode, after exact byte proof.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "mode_restore_open_failed")?;
    let before = file
        .metadata()
        .map_err(|_| "mode_restore_metadata_failed")?;
    let path_before = fs::symlink_metadata(path).map_err(|_| "mode_restore_metadata_failed")?;
    if !before.is_file()
        || before.uid() != unsafe { libc::geteuid() }
        || file_identity(&before) != file_identity(&path_before)
    {
        return Err("mode_restore_scope_changed");
    }
    let mut checked = Zeroizing::new(Vec::new());
    (&file)
        .take(crate::config::MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut checked)
        .map_err(|_| "mode_restore_read_failed")?;
    if original
        .bytes
        .as_ref()
        .is_none_or(|expected| expected.as_slice() != checked.as_slice())
        || file_identity(&before)
            != file_identity(
                &file
                    .metadata()
                    .map_err(|_| "mode_restore_metadata_failed")?,
            )
        || file_identity(&before)
            != file_identity(
                &fs::symlink_metadata(path).map_err(|_| "mode_restore_metadata_failed")?,
            )
    {
        return Err("mode_restore_file_changed");
    }
    file.set_permissions(fs::Permissions::from_mode(mode))
        .map_err(|_| "mode_restore_failed")?;
    file.sync_all().map_err(|_| "mode_restore_sync_failed")?;
    let after = file
        .metadata()
        .map_err(|_| "mode_restore_metadata_failed")?;
    let current = fs::symlink_metadata(path).map_err(|_| "mode_restore_metadata_failed")?;
    if after.dev() != current.dev() || after.ino() != current.ino() {
        return Err("mode_restore_file_changed");
    }
    Ok(())
}

struct PlannedResponse {
    method: Method,
    url: String,
    body: &'static [u8],
    response: Vec<u8>,
}
type PendingResponses = Arc<Mutex<VecDeque<PlannedResponse>>>;

fn scripted_panel(
    service: &Service,
    bootstrap: &Value,
    node: &str,
) -> ProbeResult<(PendingResponses, Arc<AtomicBool>)> {
    let bootstrap_bytes =
        serde_json::to_vec(bootstrap).map_err(|_| "synthetic_response_encode_failed")?;
    let models_bytes = serde_json::to_vec(&json!({
        "data": bootstrap["data"]["models"],
        "meta": { "revision": bootstrap["data"]["meta"]["catalog_revision"] },
    }))
    .map_err(|_| "synthetic_response_encode_failed")?;
    let origin = "https://panel.example.test";
    let pending = Arc::new(Mutex::new(VecDeque::from([
        PlannedResponse {
            method: Method::GET,
            url: format!("{origin}/api/v1/orbit/bootstrap"),
            body: b"",
            response: bootstrap_bytes.clone(),
        },
        PlannedResponse {
            method: Method::GET,
            url: format!("{origin}/api/v1/orbit/models?node_id={node}"),
            body: b"",
            response: models_bytes.clone(),
        },
        PlannedResponse {
            method: Method::GET,
            url: format!("{origin}/api/v1/orbit/bootstrap"),
            body: b"",
            response: bootstrap_bytes,
        },
        PlannedResponse {
            method: Method::GET,
            url: format!("{origin}/api/v1/orbit/models?node_id={node}"),
            body: b"",
            response: models_bytes,
        },
        PlannedResponse {
            method: Method::POST,
            url: format!("{origin}/api/v1/orbit/sessions/current/revoke"),
            body: b"{}",
            response: br#"{"data":{}}"#.to_vec(),
        },
    ])));
    let unexpected = Arc::new(AtomicBool::new(false));
    let expected = pending.clone();
    let denied = unexpected.clone();
    // Http::request_observed returns this result before reqwest.send(). There is
    // no fallback, socket, real login or paid inference endpoint in this entry.
    service.http.scripted(Arc::new(move |request| {
        let planned = expected.lock().ok().and_then(|mut queue| queue.pop_front());
        let response = match planned {
            Some(planned)
                if planned.method == request.method
                    && planned.url == request.url
                    && planned.body == request.body.as_slice() =>
            {
                Ok(crate::network::RemoteResponse {
                    status: 200,
                    request_id: None,
                    bytes: planned.response,
                })
            }
            _ => {
                denied.store(true, Ordering::SeqCst);
                Err(fail("native_probe_unplanned_http"))
            }
        };
        Box::pin(async move { response })
    }));
    Ok((pending, unexpected))
}

fn random_secret(prefix: &str) -> String {
    let mut bytes = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(&mut *bytes);
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(&*bytes))
}
fn require_private_directory(path: &Path) -> ProbeResult<()> {
    let meta = fs::symlink_metadata(path).map_err(|_| "evidence_directory_unavailable")?;
    if !meta.is_dir() || meta.file_type().is_symlink() || meta.uid() != unsafe { libc::geteuid() } {
        return Err("evidence_directory_scope_unverified");
    }
    Ok(())
}
fn hook_free(paths: &SystemPaths) -> bool {
    !paths.isolated
        && paths.fixture_clients.is_none()
        && paths.fixture_policy.is_none()
        && paths.fixture_codex_runtime.is_none()
        && paths.fixture_configuration_target.is_none()
}

struct NativeRun {
    service: Arc<Service>,
    vault: Arc<OwnedNativeVault>,
    recovery: SharedRecord,
    originals: [OriginalFile; 2], // catalog then config, matching the joint journal.
    files: [PathBuf; 2],
    installation: Installation,
    connection: ConnectionRecord,
    bootstrap: Value,
    pending: PendingResponses,
    unexpected_http: Arc<AtomicBool>,
    backup_id: Option<String>,
    mutation_possible: bool,
    cleanup_attempted: bool,
    _singleton: File,
}
impl NativeRun {
    fn prepare() -> ProbeResult<Self> {
        if std::env::var("ORBIT_RUN_MACOS_CONFIG_ROUNDTRIP")
            .ok()
            .as_deref()
            != Some(OPT_IN)
        {
            return Err("native_roundtrip_opt_in_required");
        }
        let project = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .map_err(|_| "project_root_unavailable")?;
        let evidence = project.join(".cache/evidence/desktop-config-flow-20261022");
        require_private_directory(&evidence)?;
        if evidence
            .canonicalize()
            .map_err(|_| "evidence_directory_unavailable")?
            != evidence
            || std::env::var_os("ORBIT_NATIVE_CONFIG_EVIDENCE_ROOT")
                .map(PathBuf::from)
                .as_ref()
                != Some(&evidence)
        {
            return Err("native_evidence_root_opt_in_required");
        }
        let singleton = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(evidence.join("native-config-roundtrip.lock"))
            .map_err(|_| "singleton_lock_unavailable")?;
        let lock_meta = singleton
            .metadata()
            .map_err(|_| "singleton_lock_unavailable")?;
        if !lock_meta.is_file()
            || lock_meta.uid() != unsafe { libc::geteuid() }
            || lock_meta.permissions().mode() & 0o077 != 0
        {
            return Err("singleton_lock_scope_unverified");
        }
        singleton
            .try_lock_exclusive()
            .map_err(|_| "native_roundtrip_already_running")?;
        let run_id = uuid();
        let run_root = evidence.join(format!("native-component-{run_id}"));
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&run_root)
            .map_err(|_| "owned_run_directory_unavailable")?;
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("native_home_unavailable")?;
        if !home.is_absolute()
            || home.canonicalize().map_err(|_| "native_home_unavailable")? != home
        {
            return Err("native_home_scope_unverified");
        }
        let store = Arc::new(
            Store::open(&run_root.join("store")).map_err(|_| "native_project_store_unavailable")?,
        );
        let mut paths =
            SystemPaths::native(&store.root, &home).map_err(|_| "native_paths_unavailable")?;
        if !hook_free(&paths)
            || paths.codex_config != home.join(".codex/config.toml")
            || paths.codex_catalog() != home.join(".codex/orivelle-orbit.models.json")
        {
            return Err("native_paths_scope_unverified");
        }
        let helper_sha = std::env::var("ORBIT_NATIVE_HELPER_SHA256")
            .map_err(|_| "native_helper_hash_required")?;
        if helper_sha.len() != 64
            || !helper_sha
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("native_helper_hash_invalid");
        }
        paths.helper = project.join("runtime/target/debug/orbit-credential-helper");
        paths.helper_expected_sha256 = Some(helper_sha);
        if !paths.helper_available() {
            return Err("native_helper_not_prebuilt_or_changed");
        }

        let wire = golden();
        let installation_id = uuid();
        let revision = format!("{}{}", uuid().replace('-', ""), uuid().replace('-', ""));
        let account = wire["token_reply"]["data"]["account_id"]
            .as_str()
            .ok_or("synthetic_account_invalid")?
            .to_owned();
        let installation = Installation {
            id: installation_id.clone(),
            origin: "https://panel.example.test".into(),
            public_key: String::new(),
            signing_reference: format!("installation/{installation_id}/ed25519"),
            backup_reference: format!("installation/{installation_id}/backup"),
        };
        let connection = ConnectionRecord {
            view: ConnectionView {
                id: uuid(),
                label: "Native component synthetic account".into(),
                panel_origin: installation.origin.clone(),
                account_id: account.clone(),
                node_id: Some("01ARZ3NDEKTSV4RRFFQ69G5FAY".into()),
                node_label: Some("Synthetic node".into()),
                edge_base_url: Some("https://edge.example.test/v1".into()),
                provider_id: "openai".into(),
                revision: 1,
                credential_revision: None,
                permission_state: "allowed".into(),
            },
            installation_id: installation_id.clone(),
            session_reference: format!("native-config-flow/{run_id}/session/{}", uuid()),
            credential_reference: None,
            session_status: "authenticated".into(),
            expires_at: chrono::Utc::now().timestamp() + 900,
            session_expires_at: Some((chrono::Utc::now() + chrono::Duration::hours(1)).timestamp()),
            refresh_attempt: None,
            credential_expires_at: None,
            credential_state: "unavailable".into(),
            api_key_id: None,
            authorization_epoch: 1,
            cleanup_references: vec![],
        };
        let inference = format!(
            "inference/{installation_id}/{}/{revision}",
            sha256(account.as_bytes())
        );
        let recovery = Arc::new(Mutex::new(RecoveryRecord {
            run_id,
            step: "project_prepared",
            state: "preflight_pending",
            store: "store",
            references: [
                (
                    "installation_signing",
                    installation.signing_reference.clone(),
                ),
                ("installation_backup", installation.backup_reference.clone()),
                ("session", connection.session_reference.clone()),
                ("inference", inference),
            ]
            .into_iter()
            .map(|(kind, reference)| ReferenceState {
                kind,
                reference,
                absent: false,
                write_attempted: false,
                deleted: false,
            })
            .collect(),
            snapshot_id: None,
            original_present: [false; 2],
            original_modes: [None; 2],
            exact_restore_proven: false,
            root_retained: false,
            last_error: None,
            path: run_root.join("recovery.json"),
        }));
        recovery
            .lock()
            .map_err(|_| "recovery_record_unavailable")?
            .persist()?;
        let vault = Arc::new(OwnedNativeVault {
            native: NativeVault::default(),
            record: recovery.clone(),
        });
        let service = Service::new(store, vault.clone(), paths)
            .map_err(|_| "native_service_preflight_failed")?;
        // Empty project Store means startup recovery cannot read a native ref.
        // Full installed-target and local policy preflight precedes arm/put and
        // every write to the two shared host files. Runtime stays unconfirmed.
        service
            .configuration_preflight(ClientKind::Codex)
            .map_err(|_| "native_configuration_preflight_failed")?;
        let observation = service
            .observe_configuration(ClientKind::Codex)
            .map_err(|_| "native_configuration_observation_failed")?;
        if observation.policy.state != crate::policy::State::Unknown
            || observation
                .runtime_view
                .as_ref()
                .is_none_or(|runtime| runtime.state != "unverified")
            || configuration_target_view(&observation).is_none()
            || !hook_free(&service.paths)
        {
            return Err("native_preflight_evidence_boundary_changed");
        }
        let files = [
            service.paths.codex_catalog(),
            service.paths.codex_config.clone(),
        ];
        let originals = [capture(&files[0])?, capture(&files[1])?];
        {
            let mut record = recovery.lock().map_err(|_| "recovery_record_unavailable")?;
            record.original_present = originals.each_ref().map(|file| file.bytes.is_some());
            record.original_modes = originals.each_ref().map(|file| file.mode);
            record.root_retained = fs::symlink_metadata(home.join(".codex")).is_err();
            record.step = "native_preflight_passed";
            record.state = "no_native_mutations";
            record.persist()?;
        }
        let mut bootstrap = wire["bootstrap_reply"].clone();
        bootstrap["data"]["inference_keys"][0]["key_revision"] = json!(revision);
        bootstrap["data"]["models"][0]["codex_catalog_binding"] = json!({
            "state": "declared", "verified": false, "profile_id": crate::codex_binding::PROFILE_ID,
            "cli_version": crate::codex_catalog::SUPPORTED_CLI_VERSION, "source_sha256": crate::codex_catalog::SOURCE_SHA256, "template_slug": "gpt-5.5",
        });
        bootstrap["data"]["models"][0]["reasoning"] = json!({ "state": "restricted", "capability_status": "supported",
            "supported_efforts": ["low", "medium", "high"], "allowed_efforts": ["medium", "high"], "default_effort": "medium", "revision": 1 });
        let (pending, unexpected_http) = scripted_panel(
            &service,
            &bootstrap,
            connection
                .view
                .node_id
                .as_deref()
                .ok_or("synthetic_node_invalid")?,
        )?;
        Ok(Self {
            service,
            vault,
            recovery,
            originals,
            files,
            installation,
            connection,
            bootstrap,
            pending,
            unexpected_http,
            backup_id: None,
            mutation_possible: false,
            cleanup_attempted: false,
            _singleton: singleton,
        })
    }

    fn seed_native_state(&mut self) -> ProbeResult<()> {
        self.vault.arm()?;
        record_step(
            &self.recovery,
            "native_identity_seed",
            "writing_owned_references",
        )?;
        let signing = ed25519_dalek::SigningKey::generate(&mut OsRng);
        self.installation.public_key = URL_SAFE_NO_PAD.encode(signing.verifying_key().to_bytes());
        #[derive(Serialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
        struct Seed {
            origin: String,
            installation_id: String,
            seed: String,
        }
        let seed = Zeroizing::new(signing.to_bytes());
        let secret = Seed {
            origin: self.installation.origin.clone(),
            installation_id: self.installation.id.clone(),
            seed: URL_SAFE_NO_PAD.encode(&*seed),
        };
        let bytes = Zeroizing::new(
            serde_json::to_vec(&secret).map_err(|_| "synthetic_signing_encode_failed")?,
        );
        self.vault
            .put(&self.installation.signing_reference, &bytes)
            .map_err(|_| "native_signing_write_failed")?;
        let mut backup = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(&mut *backup);
        self.vault
            .put(&self.installation.backup_reference, &*backup)
            .map_err(|_| "native_backup_key_write_failed")?;
        let store = &self.service.store;
        store
            .put(
                "installation",
                &sha256(self.installation.origin.as_bytes()),
                &self.installation.origin,
                &self.installation,
            )
            .map_err(|_| "synthetic_installation_store_failed")?;
        store
            .put(
                "connection",
                &self.connection.view.id,
                &self.connection.view.account_id,
                &self.connection,
            )
            .map_err(|_| "synthetic_connection_store_failed")?;
        store
            .set_meta("active_connection", &self.connection.view.id)
            .map_err(|_| "synthetic_connection_store_failed")?;
        store
            .set_meta(
                &authorization_epoch_reference(
                    &self.installation.origin,
                    &self.installation.id,
                    &self.connection.view.account_id,
                ),
                "1",
            )
            .map_err(|_| "synthetic_epoch_store_failed")?;
        let session = ProofSession {
            binding_id: "01ARZ3NDEKTSV4RRFFQ69G5FAW".into(),
            nonce: random_secret(""),
            access_token: random_secret("orv_orbit_at_"),
            refresh_token: random_secret("orv_orbit_rt_"),
            expires_at: self.connection.expires_at,
            session_expires_at: (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            scopes: vec![
                "account:read".into(),
                "connections:read".into(),
                "usage:read".into(),
                "session:revoke".into(),
                "inference-key:obtain".into(),
            ],
            credential_delivery_ticket: None,
        };
        self.service
            .save_session(&self.connection, session)
            .map_err(|_| "native_session_write_failed")?;
        let cache = map_account_cache(&self.bootstrap["data"], &self.connection)
            .map_err(|_| "synthetic_cache_invalid")?;
        store
            .put(
                "account-cache",
                &self.connection.view.id,
                &self.connection.view.account_id,
                &cache,
            )
            .map_err(|_| "synthetic_cache_store_failed")?;
        let key_revision = self.bootstrap["data"]["inference_keys"][0]["key_revision"]
            .as_str()
            .ok_or("synthetic_key_revision_invalid")?
            .to_owned();
        self.service
            .accept_credential(
                &mut self.connection,
                &self.installation,
                CredentialDelivery {
                    provider_id: "openai".into(),
                    api_key_id: "01ARZ3NDEKTSV4RRFFQ69G5FAX".into(),
                    key_revision,
                    plain_text_token: random_secret("orv_sk_native_probe."),
                    expires_at: None,
                },
            )
            .map_err(|_| "native_inference_write_failed")?;
        record_step(
            &self.recovery,
            "native_identity_seeded",
            "owned_references_only",
        )
    }

    fn unchanged(&self, include_modes: bool) -> ProbeResult<bool> {
        for (path, original) in self.files.iter().zip(&self.originals) {
            let current = capture(path)?;
            if !same_bytes(original, &current) || include_modes && current.mode != original.mode {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn require_unconfirmed(&self) -> ProbeResult<()> {
        let bootstrap = self
            .service
            .bootstrap()
            .map_err(|_| "native_bootstrap_failed")?;
        let client = bootstrap
            .clients
            .iter()
            .find(|client| client.client == ClientKind::Codex)
            .ok_or("native_client_missing")?;
        let observation = self
            .service
            .observe_configuration(ClientKind::Codex)
            .map_err(|_| "native_configuration_observation_failed")?;
        if client.loaded_evidence.is_some()
            || client.connection_evidence.is_some()
            || client.managed != "unknown"
            || observation
                .runtime_view
                .as_ref()
                .is_none_or(|runtime| runtime.state != "unverified")
        {
            return Err("native_evidence_was_overclaimed");
        }
        Ok(())
    }

    async fn exercise(&mut self) -> ProbeResult<()> {
        self.seed_native_state()?;
        record_step(
            &self.recovery,
            "joint_preview",
            "preparing_encrypted_originals",
        )?;
        let preview = self
            .service
            .preview_switch_config(
                &self.connection.view.id,
                self.connection.view.revision,
                ClientKind::Codex,
                None,
            )
            .await
            .map_err(|_| "native_joint_preview_failed")?;
        if !preview.can_apply
            || preview.codex_configuration_target.is_none()
            || preview.codex_catalog.is_none()
            || preview
                .codex_runtime
                .as_ref()
                .is_none_or(|runtime| runtime.state != "unverified")
        {
            return Err("native_joint_preview_not_qualified");
        }
        let journal_id = self
            .service
            .previews
            .lock()
            .map_err(|_| "native_preview_unavailable")?
            .get(&preview.id)
            .and_then(|entry| entry.journal_id.clone())
            .ok_or("native_joint_journal_missing")?;
        self.backup_id = Some(journal_id.clone());
        {
            let mut record = self
                .recovery
                .lock()
                .map_err(|_| "recovery_record_unavailable")?;
            record.snapshot_id = Some(journal_id);
            record.step = "joint_preview_encrypted";
            record.state = "confirmation_pending";
            record.persist()?;
        }
        match self
            .service
            .apply_client_config_refreshed(&preview.id, false)
            .await
        {
            Err(error) if error.code == "confirmation_required" => {}
            _ => return Err("unconfirmed_apply_was_not_refused"),
        }
        if !self.unchanged(true)?
            || self
                .pending
                .lock()
                .map_err(|_| "synthetic_script_unavailable")?
                .len()
                != 3
        {
            return Err("unconfirmed_apply_had_side_effects");
        }
        record_step(
            &self.recovery,
            "confirmed_joint_apply",
            "host_write_may_have_started",
        )?;
        self.mutation_possible = true;
        let saved = self
            .service
            .apply_client_config_refreshed(&preview.id, true)
            .await
            .map_err(|_| "native_joint_apply_failed")?;
        if saved.state != "saved"
            || saved.loaded_state != "restart_required"
            || saved.snapshot_id != self.backup_id
        {
            return Err("native_saved_state_invalid");
        }
        self.require_unconfirmed()?;
        let backup = self
            .service
            .store
            .get::<Journal>(
                "journal",
                self.backup_id.as_deref().ok_or("native_backup_missing")?,
            )
            .map_err(|_| "native_backup_read_failed")?
            .ok_or("native_backup_missing")?;
        if backup.ciphertext.is_empty()
            || backup.paths
                != self
                    .files
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
        {
            return Err("native_encrypted_backup_invalid");
        }
        let changes = Zeroizing::new(
            self.service
                .config_engine()
                .synthetic_files(&backup)
                .map_err(|_| "native_backup_decryption_failed")?,
        );
        if changes.len() != 2 {
            return Err("native_joint_backup_cardinality_invalid");
        }
        for ((change, path), original) in changes.iter().zip(&self.files).zip(&self.originals) {
            let current = capture(path)?;
            if change.path != path.display().to_string()
                || change.before.as_deref() != original.bytes.as_ref().map(|bytes| bytes.as_slice())
                || change.after.is_none()
                || change.after.as_deref() != current.bytes.as_ref().map(|bytes| bytes.as_slice())
            {
                return Err("native_joint_backup_readback_mismatch");
            }
        }
        record_step(
            &self.recovery,
            "encrypted_backup_readback_verified",
            "logout_pending",
        )?;
        self.service
            .logout(&self.connection.view.id)
            .await
            .map_err(|_| "native_logout_failed")?;
        if self
            .service
            .store
            .list::<HelperBinding>("binding", None)
            .map_err(|_| "native_grants_unavailable")?
            .iter()
            .any(|binding| binding.enabled)
        {
            return Err("native_logout_left_grant_enabled");
        }
        record_step(&self.recovery, "logged_out", "standard_restore_pending")?;
        let restore = self
            .service
            .preview_restore(self.backup_id.as_deref().ok_or("native_backup_missing")?)
            .map_err(|_| "native_restore_preview_failed")?;
        if !restore.can_apply {
            return Err("native_restore_not_qualified");
        }
        let restored = self
            .service
            .apply_client_config(&restore.id, true, true)
            .map_err(|_| "native_standard_restore_failed")?;
        if restored.state != "restored"
            || restored.loaded_state != "restart_required"
            || !self.unchanged(false)?
        {
            return Err("native_original_bytes_not_restored");
        }
        self.require_unconfirmed()?;
        if self.unexpected_http.load(Ordering::SeqCst)
            || !self
                .pending
                .lock()
                .map_err(|_| "synthetic_script_unavailable")?
                .is_empty()
        {
            return Err("synthetic_http_plan_not_exact");
        }
        record_step(
            &self.recovery,
            "standard_restore_verified",
            "mode_and_owned_ref_cleanup_pending",
        )
    }

    fn cleanup_inner(&mut self) -> ProbeResult<()> {
        if !self.unchanged(false)? {
            if !self.mutation_possible {
                return Err("pre_apply_external_file_change_retained");
            }
            record_step(
                &self.recovery,
                "failure_cleanup_restore",
                "conflict_aware_restore_only",
            )?;
            let preview = self
                .service
                .preview_restore(
                    self.backup_id
                        .as_deref()
                        .ok_or("recovery_backup_id_missing")?,
                )
                .map_err(|_| "recovery_preview_failed")?;
            if !preview.can_apply {
                return Err("recovery_qualification_blocked");
            }
            self.service
                .apply_client_config(&preview.id, true, true)
                .map_err(|_| "recovery_conflict_or_restore_failed")?;
        }
        if !self.unchanged(false)? {
            return Err("recovery_original_bytes_unproven");
        }
        if let Some(record) = self
            .service
            .store
            .get::<ConnectionRecord>("connection", &self.connection.view.id)
            .map_err(|_| "recovery_connection_unavailable")?
            && record.session_status != "logged_out"
        {
            // Local production logout fences this test account before deletion;
            // the failure path sends no additional synthetic/real HTTP request.
            self.service
                .commit_logout(&record.view.id)
                .map_err(|_| "recovery_local_logout_failed")?;
        }
        if self
            .service
            .store
            .list::<HelperBinding>("binding", None)
            .map_err(|_| "recovery_grants_unavailable")?
            .iter()
            .any(|binding| binding.enabled)
        {
            return Err("recovery_grant_remains_enabled");
        }
        self.service
            .configuration_preflight(ClientKind::Codex)
            .map_err(|_| "recovery_policy_or_target_changed")?;
        for (path, original) in self.files.iter().zip(&self.originals) {
            restore_mode(path, original)?;
        }
        if !self.unchanged(true)? {
            return Err("recovery_exact_original_unproven");
        }
        {
            let mut record = self
                .recovery
                .lock()
                .map_err(|_| "recovery_record_unavailable")?;
            record.exact_restore_proven = true;
            record.step = "exact_original_bytes_and_modes_restored";
            record.state = "owned_ref_cleanup_pending";
            record.persist()?;
        }
        self.vault.delete_owned()?;
        record_step(&self.recovery, "owned_references_deleted", "complete")
        // If this run created ~/.codex, retain it. Never remove a nonempty or
        // surprise directory in acceptance cleanup. The recovery record says so.
    }
    fn cleanup(&mut self) -> ProbeResult<()> {
        self.cleanup_attempted = true;
        let result = self.cleanup_inner();
        if let Err(code) = result {
            if let Ok(mut record) = self.recovery.lock() {
                record.step = "cleanup_incomplete";
                record.state = "retain_project_store_and_owned_backup_refs";
                record.last_error = Some(code);
                let _ = record.persist();
            }
        }
        result
    }
}
impl Drop for NativeRun {
    fn drop(&mut self) {
        if !self.cleanup_attempted {
            // Normal errors and caught panics use cleanup explicitly; this is a
            // final unwind guard. SIGKILL cannot run Drop: keep recovery.json.
            let _ = std::panic::catch_unwind(AssertUnwindSafe(|| self.cleanup()));
        }
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "Writes the actual shared default Codex config and four fresh Keychain refs; Root-reviewed explicit opt-in required"]
async fn native_macos_service_config_roundtrip() {
    // Run only this selected ignored entry with --test-threads=1. A caught panic
    // still invokes the process hook, so suppress it throughout preparation,
    // execution, cleanup and Drop; restore it only after all that work is caught.
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let worked = AssertUnwindSafe(async {
        let mut run = NativeRun::prepare()?;
        let exercised = AssertUnwindSafe(run.exercise()).catch_unwind().await;
        let failure = match exercised {
            Ok(Ok(())) => None,
            Ok(Err(code)) => Some(code),
            Err(_) => Some("native_roundtrip_panicked"),
        };
        run.cleanup()?;
        failure.map_or(Ok(()), Err)
    })
    .catch_unwind()
    .await;
    // Discard payloads while the silent hook is still installed. No arbitrary
    // product error, diff, secret or panic payload reaches terminal output.
    let failure = match worked {
        Ok(result) => result.err(),
        Err(_) => Some("native_roundtrip_or_cleanup_panicked"),
    };
    // No active unwind reaches set_hook (which would otherwise double-panic).
    std::panic::set_hook(previous_hook);
    if let Some(code) = failure {
        panic!("native roundtrip failed; consult retained recovery record: {code}");
    }
    println!(
        r#"{{"state":"passed","scope":"native_service_component","panel":"scripted_no_network","original_files":"bytes_and_modes_restored","owned_keychain_refs":"deleted","runtime":"unverified","loaded_connected":"unconfirmed"}}"#
    );
}
