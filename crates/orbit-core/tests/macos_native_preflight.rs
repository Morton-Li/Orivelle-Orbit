#![cfg(target_os = "macos")]

use fs2::FileExt;
use keyring_core::api::CredentialStoreApi;
use orbit_core::vault::{NativeVault, Vault};
use rand::{RngCore, rngs::OsRng};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

type ProbeResult<T> = Result<T, &'static str>;
const SERVICE: &str = "io.orivelle.orbit";
const PROBE_PREFIX: &str = "macos-native-preflight-20261021-";
const STEPS: [&str; 10] = [
    "reference_absent",
    "create",
    "read_created",
    "update",
    "read_updated",
    "delete",
    "read_deleted_missing",
    "repeat_delete",
    "repeat_delete_absent",
    "cleanup",
];

#[derive(Serialize)]
struct Step {
    name: &'static str,
    state: &'static str,
}

#[derive(Serialize)]
struct RecoveryJournal {
    reference: String,
    steps: Vec<Step>,
}

/// Owns only the fresh reference named in the public recovery journal.
struct Probe {
    vault: NativeVault,
    journal: RecoveryJournal,
    path: PathBuf,
    owned: bool,
}

impl Probe {
    fn new(directory: &Path) -> ProbeResult<Self> {
        let id = uuid::Uuid::new_v4();
        let probe = Self {
            vault: NativeVault::default(),
            journal: RecoveryJournal {
                reference: format!("{PROBE_PREFIX}{id}"),
                steps: STEPS
                    .into_iter()
                    .map(|name| Step {
                        name,
                        state: "pending",
                    })
                    .collect(),
            },
            path: directory.join(format!("keychain-probe-{id}.json")),
            owned: false,
        };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&probe.path)
            .map_err(|_| "preflight_journal_create_failed")?;
        probe.write(&mut file)?;
        probe.sync_directory()?;
        Ok(probe)
    }

    fn write(&self, file: &mut File) -> ProbeResult<()> {
        let public = serde_json::to_vec_pretty(&self.journal)
            .map_err(|_| "preflight_journal_encode_failed")?;
        file.write_all(&public)
            .and_then(|_| file.write_all(b"\n"))
            .and_then(|_| file.sync_all())
            .map_err(|_| "preflight_journal_write_failed")
    }

    fn sync_directory(&self) -> ProbeResult<()> {
        File::open(self.path.parent().ok_or("preflight_journal_path_invalid")?)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| "preflight_journal_sync_failed")
    }

    fn record(&mut self, name: &str, state: &'static str) -> ProbeResult<()> {
        self.journal
            .steps
            .iter_mut()
            .find(|step| step.name == name)
            .ok_or("preflight_step_invalid")?
            .state = state;
        let parent = self.path.parent().ok_or("preflight_journal_path_invalid")?;
        let mut staged = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| "preflight_journal_stage_failed")?;
        self.write(staged.as_file_mut())?;
        staged
            .persist(&self.path)
            .map_err(|_| "preflight_journal_publish_failed")?;
        self.sync_directory()
    }

    fn step(
        &mut self,
        name: &str,
        action: impl FnOnce(&NativeVault, &str) -> ProbeResult<()>,
    ) -> ProbeResult<()> {
        self.record(name, "running")?;
        let result = action(&self.vault, &self.journal.reference);
        let recorded = self.record(name, if result.is_ok() { "passed" } else { "failed" });
        result.and(recorded)
    }

    fn roundtrip(&mut self) -> ProbeResult<()> {
        self.step("reference_absent", |vault, reference| {
            exact_reference_absent(reference)?;
            vault_reports_missing(vault, reference)
        })?;
        // Persist ownership before the first possible Keychain write. A random
        // collision or a denied absence check never reaches this point.
        self.record("cleanup", "armed")?;
        self.owned = true;
        let mut initial = Zeroizing::new([0u8; 32]);
        let mut updated = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(initial.as_mut());
        OsRng.fill_bytes(updated.as_mut());
        if initial.as_slice() == updated.as_slice() {
            updated[0] ^= 1;
        }
        self.step("create", |vault, reference| {
            vault
                .put(reference, initial.as_slice())
                .map_err(|_| "native_keychain_create_failed")
        })?;
        self.step("read_created", |vault, reference| {
            matches_secret(vault, reference, initial.as_slice())
        })?;
        self.step("update", |vault, reference| {
            vault
                .put(reference, updated.as_slice())
                .map_err(|_| "native_keychain_update_failed")
        })?;
        self.step("read_updated", |vault, reference| {
            matches_secret(vault, reference, updated.as_slice())
        })?;
        self.step("delete", |vault, reference| {
            vault
                .delete(reference)
                .map_err(|_| "native_keychain_delete_failed")
        })?;
        self.step("read_deleted_missing", |vault, reference| {
            vault_reports_missing(vault, reference)?;
            exact_reference_absent(reference)
        })?;
        self.step("repeat_delete", |vault, reference| {
            vault
                .delete(reference)
                .map_err(|_| "native_keychain_repeat_delete_failed")
        })?;
        self.step("repeat_delete_absent", |_, reference| {
            exact_reference_absent(reference)
        })?;
        self.owned = false;
        self.record("cleanup", "passed")
    }

    fn cleanup(&mut self) {
        if !self.owned {
            return;
        }
        let _ = self.record("cleanup", "running");
        let deleted = self.vault.delete(&self.journal.reference).is_ok();
        let absent = exact_reference_absent(&self.journal.reference).is_ok();
        if deleted && absent {
            self.owned = false;
            let _ = self.record("cleanup", "passed");
        } else {
            let _ = self.record("cleanup", "failed");
        }
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn vault_reports_missing(vault: &NativeVault, reference: &str) -> ProbeResult<()> {
    match vault.get(reference) {
        Err(error) if error.code == "credential_missing" => Ok(()),
        _ => Err("native_keychain_missing_check_failed"),
    }
}

fn matches_secret(vault: &NativeVault, reference: &str, expected: &[u8]) -> ProbeResult<()> {
    let actual = vault
        .get(reference)
        .map_err(|_| "native_keychain_read_failed")?;
    if actual.as_slice() != expected {
        // Never use an assertion that formats either private value.
        return Err("native_keychain_value_mismatch");
    }
    Ok(())
}

fn exact_reference_absent(reference: &str) -> ProbeResult<()> {
    // NativeVault deliberately maps all get errors to credential_missing.
    // Distinguish NoEntry from denied/unavailable for this single fresh ref;
    // no store enumeration, attributes or other references are queried.
    let store = apple_native_keyring_store::keychain::Store::new()
        .map_err(|_| "native_keychain_absence_store_failed")?;
    let entry = store
        .build(SERVICE, reference, Some(&HashMap::new()))
        .map_err(|_| "native_keychain_absence_entry_failed")?;
    match entry.get_secret() {
        Err(keyring_core::Error::NoEntry) => Ok(()),
        Ok(value) => {
            drop(Zeroizing::new(value));
            Err("native_keychain_reference_not_absent")
        }
        Err(_) => Err("native_keychain_absence_unverified"),
    }
}

fn run_probe() -> ProbeResult<()> {
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|_| "preflight_project_unavailable")?;
    let directory = project.join(".cache/evidence/macos-native-preflight-20261021");
    let metadata =
        fs::symlink_metadata(&directory).map_err(|_| "preflight_evidence_directory_unavailable")?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || directory
            .canonicalize()
            .map_err(|_| "preflight_evidence_directory_unavailable")?
            != directory
    {
        return Err("preflight_evidence_directory_invalid");
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(directory.join("keychain-probe.lock"))
        .map_err(|_| "preflight_lock_unavailable")?;
    if !lock
        .metadata()
        .map_err(|_| "preflight_lock_unavailable")?
        .is_file()
    {
        return Err("preflight_lock_invalid");
    }
    FileExt::try_lock_exclusive(&lock).map_err(|_| "native_keychain_probe_already_running")?;
    let mut probe = Probe::new(&directory)?;
    probe.roundtrip()
}

/// Explicitly run this singleton only during the authorized native Mac phase.
/// It never enumerates Keychain, launches a client/helper or changes config.
#[test]
#[ignore = "requires the authorized real macOS NativeVault preflight"]
fn native_keychain_roundtrip() {
    if let Err(code) = run_probe() {
        panic!("{code}");
    }
}
