use crate::{
    dto::*,
    error::{Result, fail},
    identity::{HelperBinding, Installation},
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub struct Store {
    pub root: PathBuf,
    db: Mutex<Connection>,
    image_files: Mutex<()>,
}
pub struct Snapshot {
    records: Vec<(String, String, String, String)>,
    pub meta: HashMap<String, String>,
    pub revision: u64,
}
pub struct PreviewScope {
    pub token: String,
    pub origin: String,
    pub account_id: String,
    pub installation_id: String,
    pub connection_id: String,
    pub connection_revision: u64,
}
pub fn preview_scope_token(
    nonce: &str,
    origin: &str,
    account_id: &str,
    installation_id: &str,
    connection_id: &str,
    revision: u64,
) -> String {
    crate::identity::sha256(format!("orbit-preview-v1\n{nonce}\n{origin}\n{account_id}\n{installation_id}\n{connection_id}\n{revision}\n").as_bytes())
}
impl Snapshot {
    pub fn get<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<Option<T>> {
        self.records
            .iter()
            .find(|r| r.0 == kind && r.1 == id)
            .map(|r| serde_json::from_str(&r.3).map_err(|_| fail("local_metadata_invalid")))
            .transpose()
    }
    pub fn list<T: DeserializeOwned>(&self, kind: &str, scope: Option<&str>) -> Result<Vec<T>> {
        self.records
            .iter()
            .filter(|r| r.0 == kind && scope.is_none_or(|scope| r.2 == scope))
            .map(|r| serde_json::from_str(&r.3).map_err(|_| fail("local_metadata_invalid")))
            .collect()
    }
}
pub fn private_directory(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink() || !m.is_dir()) {
        return Err(fail("local_storage_unavailable"));
    }
    fs::create_dir_all(path).map_err(|_| fail("local_storage_unavailable"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| fail("local_storage_unavailable"))?;
    }
    #[cfg(windows)]
    windows_private_acl(path, true)?;
    Ok(())
}
pub fn private_file(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) {
        return Err(fail("local_storage_unavailable"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| fail("local_storage_unavailable"))?;
    }
    #[cfg(windows)]
    windows_private_acl(path, false)?;
    let _ = path;
    Ok(())
}
/// Windows PowerShell is part of the supported Windows 11 baseline. Only this
/// fixed ACL operation is invoked; paths are data in a child environment value,
/// never executable PowerShell text. Failure or unverifiable ACLs fail closed.
#[cfg(windows)]
fn windows_private_acl(path: &Path, directory: bool) -> Result<()> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    const SCRIPT: &str = r#"$ErrorActionPreference='Stop'; try {
$p=$env:ORBIT_PRIVATE_PATH; $item=Get-Item -LiteralPath $p -Force;
if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { exit 1 };
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User;
$system=[Security.Principal.SecurityIdentifier]::new('S-1-5-18');
$inherit=[Security.AccessControl.InheritanceFlags]::None;
if ($env:ORBIT_PRIVATE_DIRECTORY -eq '1') {
 $acl=[Security.AccessControl.DirectorySecurity]::new();
 $inherit=[Security.AccessControl.InheritanceFlags]'ContainerInherit,ObjectInherit';
} else { $acl=[Security.AccessControl.FileSecurity]::new() };
$acl.SetOwner($sid); $acl.SetAccessRuleProtection($true,$false);
foreach ($principal in @($sid,$system)) {
 $rule=[Security.AccessControl.FileSystemAccessRule]::new($principal,[Security.AccessControl.FileSystemRights]::FullControl,$inherit,[Security.AccessControl.PropagationFlags]::None,[Security.AccessControl.AccessControlType]::Allow);
 $acl.AddAccessRule($rule);
};
Set-Acl -LiteralPath $p -AclObject $acl;
$actual=Get-Acl -LiteralPath $p;
if (!$actual.AreAccessRulesProtected -or $actual.GetOwner([Security.Principal.SecurityIdentifier]).Value -ne $sid.Value) { exit 1 };
$rules=$actual.GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier]);
if ($rules.Count -ne 2) { exit 1 };
foreach ($r in $rules) {
 if ($r.AccessControlType -ne [Security.AccessControl.AccessControlType]::Allow -or $r.FileSystemRights -ne [Security.AccessControl.FileSystemRights]::FullControl -or ($r.IdentityReference.Value -ne $sid.Value -and $r.IdentityReference.Value -ne $system.Value)) { exit 1 };
};
[Console]::Write('ORBIT_ACL_OK'); exit 0;
} catch { exit 1 }"#;
    if !path.is_absolute() {
        return Err(fail("local_storage_unavailable"));
    }
    let system_root =
        std::env::var_os("SystemRoot").ok_or_else(|| fail("local_storage_unavailable"))?;
    let executable =
        PathBuf::from(system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let mut child = Command::new(executable)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            SCRIPT,
        ])
        .env("ORBIT_PRIVATE_PATH", path)
        .env("ORBIT_PRIVATE_DIRECTORY", if directory { "1" } else { "0" })
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| fail("local_storage_unavailable"))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|_| fail("local_storage_unavailable"))?
        {
            let mut output = Vec::new();
            child
                .stdout
                .take()
                .ok_or_else(|| fail("local_storage_unavailable"))?
                .take(64)
                .read_to_end(&mut output)
                .map_err(|_| fail("local_storage_unavailable"))?;
            return if status.success() && output == b"ORBIT_ACL_OK" {
                Ok(())
            } else {
                Err(fail("local_storage_unavailable"))
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(fail("local_storage_unavailable"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        crate::update_recovery::ensure_ready(root)?;
        Self::open_inner(root, true)
    }
    pub(crate) fn open_for_update_validation(root: &Path) -> Result<Self> {
        crate::update_recovery::ensure_ready(root)?;
        Self::open_inner(root, false)
    }
    fn open_inner(root: &Path, recover_tasks: bool) -> Result<Self> {
        private_directory(root)?;
        for name in [
            "images",
            "previews",
            "inputs",
            "pending-save",
            "backups",
            "journals",
            "locks",
            "updates",
        ] {
            private_directory(&root.join(name))?;
        }
        let path = root.join("orbit.sqlite3");
        let mut db = Connection::open(&path).map_err(|_| fail("local_storage_unavailable"))?;
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|_| fail("database_unavailable"))?;
        if version > 1 {
            return Err(fail("database_schema_newer"));
        }
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA busy_timeout=5000;").map_err(|_| fail("database_unavailable"))?;
        if version == 0 {
            let tx = db.transaction().map_err(|_| fail("database_unavailable"))?;
            tx.execute_batch("CREATE TABLE records(kind TEXT NOT NULL,id TEXT NOT NULL,scope TEXT NOT NULL,payload TEXT NOT NULL CHECK(json_valid(payload)),PRIMARY KEY(kind,id)) STRICT; CREATE INDEX records_scope ON records(kind,scope); CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) STRICT; PRAGMA user_version=1;").map_err(|_| fail("database_migration_failed"))?;
            tx.commit().map_err(|_| fail("database_migration_failed"))?;
        }
        private_file(&path)?;
        for suffix in ["-wal", "-shm"] {
            let p = root.join(format!("orbit.sqlite3{suffix}"));
            if p.exists() {
                private_file(&p)?;
            }
        }
        let store = Self {
            root: root.to_path_buf(),
            db: Mutex::new(db),
            image_files: Mutex::new(()),
        };
        store
            .db
            .lock()
            .map_err(|_| fail("database_unavailable"))?
            .execute(
                "INSERT OR IGNORE INTO meta(key,value) VALUES('state_revision','1')",
                [],
            )
            .map_err(|_| fail("database_unavailable"))?;
        // Never replay a network task after a crash or process exit.
        for mut task in if recover_tasks {
            store.list::<TaskView>("task", None)?
        } else {
            Vec::new()
        } {
            if ["queued", "sending", "waiting", "saving"].contains(&task.state.as_str()) {
                task.state = "unconfirmed".into();
                task.error = Some(fail("result_unconfirmed"));
                if let Some(mut snapshot) =
                    store.get::<crate::images::TaskSnapshot>("task-snapshot", &task.id)?
                {
                    snapshot.task = task.clone();
                    task.local_save_available =
                        crate::images::local_material_available(&store, &snapshot)?;
                    snapshot.task.local_save_available = task.local_save_available;
                    store.put_task(&snapshot)?;
                } else {
                    task.local_save_available = false;
                    store.put("task", &task.id, &task.account_id, &task)?;
                }
            }
        }
        Ok(store)
    }
    pub fn open_readonly(root: &Path) -> Result<Self> {
        crate::update_recovery::ensure_ready(root)?;
        let path = root.join("orbit.sqlite3");
        for file in [root, path.as_path()] {
            let metadata = fs::symlink_metadata(file).map_err(|_| fail("binding_disabled"))?;
            if metadata.file_type().is_symlink()
                || (file == root && !metadata.is_dir())
                || (file == path.as_path() && !metadata.is_file())
            {
                return Err(fail("binding_disabled"));
            }
        }
        let db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|_| fail("binding_disabled"))?;
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|_| fail("binding_disabled"))?;
        if version != 1 {
            return Err(fail("binding_disabled"));
        }
        Ok(Self {
            root: root.to_path_buf(),
            db: Mutex::new(db),
            image_files: Mutex::new(()),
        })
    }
    pub fn put<T: Serialize>(&self, kind: &str, id: &str, scope: &str, value: &T) -> Result<()> {
        let payload = serde_json::to_string(value).map_err(|_| fail("serialization_failed"))?;
        if payload.len()
            > if ["journal", "backup"].contains(&kind) {
                24 * 1024 * 1024
            } else {
                4 * 1024 * 1024
            }
        {
            return Err(fail("metadata_too_large"));
        }
        let mut db = self.db.lock().map_err(|_| fail("database_unavailable"))?;
        let tx = db.transaction().map_err(|_| fail("database_unavailable"))?;
        tx.execute("INSERT INTO records(kind,id,scope,payload) VALUES(?1,?2,?3,?4) ON CONFLICT(kind,id) DO UPDATE SET scope=excluded.scope,payload=excluded.payload",params![kind,id,scope,payload]).map_err(|_|fail("database_unavailable"))?;
        Self::advance(&tx)?;
        tx.commit().map_err(|_| fail("database_unavailable"))?;
        Ok(())
    }
    /// The public task and immutable scope snapshot always move together, including
    /// crash recovery and local-only save retries.
    pub fn put_task(&self, snapshot: &crate::images::TaskSnapshot) -> Result<()> {
        let task =
            serde_json::to_string(&snapshot.task).map_err(|_| fail("serialization_failed"))?;
        let payload = serde_json::to_string(snapshot).map_err(|_| fail("serialization_failed"))?;
        if task.len() > 4 * 1024 * 1024 || payload.len() > 4 * 1024 * 1024 {
            return Err(fail("metadata_too_large"));
        }
        let mut db = self.db.lock().map_err(|_| fail("database_unavailable"))?;
        let tx = db.transaction().map_err(|_| fail("database_unavailable"))?;
        for (kind, value) in [("task", &task), ("task-snapshot", &payload)] {
            tx.execute("INSERT INTO records(kind,id,scope,payload) VALUES(?1,?2,?3,?4) ON CONFLICT(kind,id) DO UPDATE SET scope=excluded.scope,payload=excluded.payload",params![kind,snapshot.task.id,snapshot.task.account_id,value]).map_err(|_|fail("database_unavailable"))?;
        }
        Self::advance(&tx)?;
        tx.commit().map_err(|_| fail("database_unavailable"))
    }
    /// Merge authoritative facts into the current local snapshot in one transaction.
    /// A late GET cannot restore deleted images or rewind a completed local save.
    pub fn merge_task_facts(
        &self,
        expected: &crate::images::TaskSnapshot,
        facts: &crate::dto::TaskView,
    ) -> Result<crate::images::TaskSnapshot> {
        let mut db = self.db.lock().map_err(|_| fail("database_unavailable"))?;
        let tx = db.transaction().map_err(|_| fail("database_unavailable"))?;
        let raw: Option<String> = tx
            .query_row(
                "SELECT payload FROM records WHERE kind='task-snapshot' AND id=?1",
                params![expected.task.id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| fail("database_unavailable"))?;
        let mut current: crate::images::TaskSnapshot =
            serde_json::from_str(&raw.ok_or_else(|| fail("task_unavailable"))?)
                .map_err(|_| fail("local_metadata_invalid"))?;
        let mut a = serde_json::to_value(expected).map_err(|_| fail("serialization_failed"))?;
        let mut b = serde_json::to_value(&current).map_err(|_| fail("serialization_failed"))?;
        for value in [&mut a, &mut b] {
            let task = value["task"]
                .as_object_mut()
                .ok_or_else(|| fail("local_metadata_invalid"))?;
            for key in [
                "state",
                "result_state",
                "outcome",
                "charge_state",
                "charges",
                "image_ids",
                "local_save_available",
                "error",
            ] {
                task.remove(key);
            }
        }
        if a != b {
            return Err(fail("account_scope_mismatch"));
        }
        current.task.result_state = facts.result_state.clone();
        current.task.outcome = facts.outcome.clone();
        current.task.charge_state = facts.charge_state.clone();
        current.task.charges = facts.charges.clone();
        for (kind, payload) in [
            ("task", serde_json::to_string(&current.task)),
            ("task-snapshot", serde_json::to_string(&current)),
        ] {
            let payload = payload.map_err(|_| fail("serialization_failed"))?;
            tx.execute(
                "UPDATE records SET payload=?1 WHERE kind=?2 AND id=?3",
                params![payload, kind, current.task.id],
            )
            .map_err(|_| fail("database_unavailable"))?;
        }
        Self::advance(&tx)?;
        tx.commit().map_err(|_| fail("database_unavailable"))?;
        Ok(current)
    }
    pub fn commit_configuration(
        &self,
        journal: &crate::config::Journal,
        source: Option<&str>,
    ) -> Result<()> {
        let payload = serde_json::to_string(journal).map_err(|_| fail("serialization_failed"))?;
        if journal.phase != "committed" || payload.len() > 24 * 1024 * 1024 {
            return Err(fail("backup_invalid"));
        }
        let mut db = self.db.lock().map_err(|_| fail("database_unavailable"))?;
        let tx = db.transaction().map_err(|_| fail("database_unavailable"))?;
        for kind in ["journal", "backup"] {
            tx.execute("INSERT INTO records(kind,id,scope,payload) VALUES(?1,?2,?3,?4) ON CONFLICT(kind,id) DO UPDATE SET scope=excluded.scope,payload=excluded.payload",params![kind,journal.id,journal.account_id,payload]).map_err(|_|fail("database_unavailable"))?;
        }
        if let Some(source) = source {
            let changed=tx.execute("UPDATE records SET payload=json_set(payload,'$.phase','restored') WHERE kind='journal' AND id=?1 AND scope=?2 AND json_extract(payload,'$.installation.id')=?3 AND json_extract(payload,'$.installation.origin')=?4",params![source,journal.account_id,journal.installation.id,journal.installation.origin]).map_err(|_|fail("database_unavailable"))?;
            if changed != 1 {
                return Err(fail("backup_invalid"));
            }
        }
        Self::advance(&tx)?;
        tx.commit().map_err(|_| fail("database_unavailable"))
    }
    pub fn get<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<Option<T>> {
        let raw: Option<String> = self
            .db
            .lock()
            .map_err(|_| fail("database_unavailable"))?
            .query_row(
                "SELECT payload FROM records WHERE kind=?1 AND id=?2",
                params![kind, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| fail("database_unavailable"))?;
        raw.map(|v| serde_json::from_str(&v).map_err(|_| fail("local_metadata_invalid")))
            .transpose()
    }
    pub fn list<T: DeserializeOwned>(&self, kind: &str, scope: Option<&str>) -> Result<Vec<T>> {
        let db = self.db.lock().map_err(|_| fail("database_unavailable"))?;
        let mut stmt=db.prepare("SELECT payload FROM records WHERE kind=?1 AND (?2 IS NULL OR scope=?2) ORDER BY rowid DESC").map_err(|_| fail("database_unavailable"))?;
        let mut size = 0usize;
        let mut values = vec![];
        for row in stmt
            .query_map(params![kind, scope], |r| r.get::<_, String>(0))
            .map_err(|_| fail("database_unavailable"))?
        {
            let row = row.map_err(|_| fail("database_unavailable"))?;
            size = size
                .checked_add(row.len())
                .ok_or_else(|| fail("metadata_too_large"))?;
            if size > 64 * 1024 * 1024 {
                return Err(fail("metadata_too_large"));
            }
            values.push(serde_json::from_str(&row).map_err(|_| fail("local_metadata_invalid"))?);
        }
        Ok(values)
    }
    fn advance(tx: &rusqlite::Transaction<'_>) -> Result<()> {
        tx.execute("UPDATE meta SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT) WHERE key='state_revision'",[]).map_err(|_|fail("database_unavailable"))?;
        Ok(())
    }
    pub fn delete(&self, kind: &str, id: &str) -> Result<()> {
        let mut db = self.db.lock().map_err(|_| fail("database_unavailable"))?;
        let tx = db.transaction().map_err(|_| fail("database_unavailable"))?;
        tx.execute(
            "DELETE FROM records WHERE kind=?1 AND id=?2",
            params![kind, id],
        )
        .map_err(|_| fail("database_unavailable"))?;
        Self::advance(&tx)?;
        tx.commit().map_err(|_| fail("database_unavailable"))?;
        Ok(())
    }
    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        if key == "state_revision" {
            return Err(fail("invalid_metadata_key"));
        }
        let mut db = self.db.lock().map_err(|_| fail("database_unavailable"))?;
        let tx = db.transaction().map_err(|_| fail("database_unavailable"))?;
        tx.execute("INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value]).map_err(|_|fail("database_unavailable"))?;
        Self::advance(&tx)?;
        tx.commit().map_err(|_| fail("database_unavailable"))?;
        Ok(())
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        let mut db = self.db.lock().map_err(|_| fail("database_unavailable"))?;
        let tx = db.transaction().map_err(|_| fail("database_unavailable"))?;
        let mut records = vec![];
        let mut size = 0usize;
        {
            let mut stmt=tx.prepare("SELECT kind,id,scope,CASE WHEN kind IN ('backup','journal') THEN json_remove(payload,'$.ciphertext') ELSE payload END FROM records WHERE kind IN ('connection','installation','account-cache','task','task-snapshot','image','backup','journal','settings','client-state','authorization-pending') ORDER BY rowid DESC").map_err(|_|fail("database_unavailable"))?;
            for row in stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })
                .map_err(|_| fail("database_unavailable"))?
            {
                let row = row.map_err(|_| fail("database_unavailable"))?;
                size += row.3.len();
                if size > 64 * 1024 * 1024 {
                    return Err(fail("metadata_too_large"));
                }
                records.push(row);
            }
        }
        let mut meta = HashMap::new();
        {
            let mut stmt = tx
                .prepare("SELECT key,value FROM meta")
                .map_err(|_| fail("database_unavailable"))?;
            for row in stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(|_| fail("database_unavailable"))?
            {
                let (k, v) = row.map_err(|_| fail("database_unavailable"))?;
                meta.insert(k, v);
            }
        }
        let revision = meta
            .get("state_revision")
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| fail("local_metadata_invalid"))?;
        tx.commit().map_err(|_| fail("database_unavailable"))?;
        Ok(Snapshot {
            records,
            meta,
            revision,
        })
    }
    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        self.db
            .lock()
            .map_err(|_| fail("database_unavailable"))?
            .query_row("SELECT value FROM meta WHERE key=?1", params![key], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|_| fail("database_unavailable"))
    }
    pub fn installation(&self, origin: &str) -> Result<Option<Installation>> {
        self.get("installation", &crate::identity::sha256(origin.as_bytes()))
    }
    pub fn disable_bindings(&self, account: &str, origin: &str) -> Result<()> {
        for mut b in self.list::<HelperBinding>("binding", Some(account))? {
            if b.origin == origin {
                b.enabled = false;
                self.put("binding", &b.id, &b.account_id, &b)?;
            }
        }
        Ok(())
    }
    pub fn settings(&self) -> Result<SettingsView> {
        Ok(self.get("settings", "local")?.unwrap_or_default())
    }
    pub fn preview_scope(&self) -> Result<Option<PreviewScope>> {
        let mut db = self.db.lock().map_err(|_| fail("database_unavailable"))?;
        let tx = db.transaction().map_err(|_| fail("database_unavailable"))?;
        let raw:Option<String>=tx.query_row("SELECT r.payload FROM records r JOIN meta m ON m.key='active_connection' AND m.value=r.id WHERE r.kind='connection'",[],|r|r.get(0)).optional().map_err(|_|fail("database_unavailable"))?;
        let nonce: Option<String> = tx
            .query_row(
                "SELECT value FROM meta WHERE key='preview_runtime_nonce'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| fail("database_unavailable"))?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let Some(nonce) = nonce else {
            return Ok(None);
        };
        let record: serde_json::Value =
            serde_json::from_str(&raw).map_err(|_| fail("local_metadata_invalid"))?;
        let view = &record["view"];
        let field = |key: &str| {
            view[key]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| fail("local_metadata_invalid"))
        };
        let origin = field("panel_origin")?;
        let account_id = field("account_id")?;
        let connection_id = field("id")?;
        let installation_id = record["installation_id"]
            .as_str()
            .ok_or_else(|| fail("local_metadata_invalid"))?
            .to_owned();
        let connection_revision = view["revision"]
            .as_u64()
            .ok_or_else(|| fail("local_metadata_invalid"))?;
        let forgetting: Option<String> = tx
            .query_row(
                "SELECT value FROM meta WHERE key=?1",
                params![format!("installation_forgetting/{installation_id}")],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| fail("database_unavailable"))?;
        if !matches!(
            record["session_status"].as_str(),
            Some("authenticated" | "refreshing")
        ) || forgetting.as_deref() == Some("1")
            || record["session_expires_at"]
                .as_i64()
                .is_none_or(|at| at <= chrono::Utc::now().timestamp())
        {
            return Ok(None);
        }
        tx.commit().map_err(|_| fail("database_unavailable"))?;
        let token = preview_scope_token(
            &nonce,
            &origin,
            &account_id,
            &installation_id,
            &connection_id,
            connection_revision,
        );
        Ok(Some(PreviewScope {
            token,
            origin,
            account_id,
            installation_id,
            connection_id,
            connection_revision,
        }))
    }
    pub fn consistent_backup(&self, target: &Path) -> Result<()> {
        let db = self.db.lock().map_err(|_| fail("database_unavailable"))?;
        db.backup("main", target, None)
            .map_err(|_| fail("database_backup_failed"))?;
        private_file(target)
    }
    pub fn image_files_lock(&self) -> Result<std::sync::MutexGuard<'_, ()>> {
        self.image_files
            .lock()
            .map_err(|_| fail("local_storage_unavailable"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_and_scope_isolation_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let s = Store::open(dir.path()).unwrap();
            s.put(
                "meta-test",
                "a",
                "account-a",
                &serde_json::json!({"safe":1}),
            )
            .unwrap();
            s.put(
                "meta-test",
                "b",
                "account-b",
                &serde_json::json!({"safe":2}),
            )
            .unwrap();
        }
        let s = Store::open(dir.path()).unwrap();
        assert_eq!(
            s.list::<serde_json::Value>("meta-test", Some("account-a"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            s.list::<serde_json::Value>("meta-test", None)
                .unwrap()
                .len(),
            2
        );
        let backup = dir.path().join("backup.sqlite3");
        s.consistent_backup(&backup).unwrap();
        assert!(backup.exists());
    }
    #[test]
    fn safety_enumeration_and_snapshot_never_silently_evict_old_identity_rows() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store
            .put(
                "connection",
                "old",
                "actor",
                &serde_json::json!({"old_identity":true}),
            )
            .unwrap();
        {
            let mut db = store.db.lock().unwrap();
            let tx = db.transaction().unwrap();
            for n in 0..10001 {
                tx.execute(
                    "INSERT INTO records(kind,id,scope,payload) VALUES('task',?1,'actor','{}')",
                    params![n.to_string()],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }
        assert_eq!(
            store.list::<serde_json::Value>("task", None).unwrap().len(),
            10001
        );
        let snapshot = store.snapshot().unwrap();
        assert_eq!(
            snapshot
                .list::<serde_json::Value>("task", None)
                .unwrap()
                .len(),
            10001
        );
        assert!(
            snapshot
                .get::<serde_json::Value>("connection", "old")
                .unwrap()
                .is_some()
        );
    }
}
