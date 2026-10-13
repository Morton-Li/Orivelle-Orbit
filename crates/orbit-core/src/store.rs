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
}
pub struct Snapshot {
    records: Vec<(String, String, String, String)>,
    pub meta: HashMap<String, String>,
    pub revision: u64,
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
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA busy_timeout=5000;").map_err(|_| fail("database_unavailable"))?;
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|_| fail("database_unavailable"))?;
        if version > 1 {
            return Err(fail("database_schema_newer"));
        }
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
        for mut task in store.list::<TaskView>("task", None)? {
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
        let mut stmt=db.prepare("SELECT payload FROM records WHERE kind=?1 AND (?2 IS NULL OR scope=?2) ORDER BY rowid DESC LIMIT 1000").map_err(|_| fail("database_unavailable"))?;
        stmt.query_map(params![kind, scope], |r| r.get::<_, String>(0))
            .map_err(|_| fail("database_unavailable"))?
            .map(|v| {
                serde_json::from_str(&v.map_err(|_| fail("database_unavailable"))?)
                    .map_err(|_| fail("local_metadata_invalid"))
            })
            .collect()
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
            let mut stmt=tx.prepare("SELECT kind,id,scope,CASE WHEN kind='backup' THEN json_remove(payload,'$.ciphertext') ELSE payload END FROM records WHERE kind IN ('connection','installation','account-cache','task','task-snapshot','image','backup','settings','client-state') ORDER BY rowid DESC LIMIT 10000").map_err(|_|fail("database_unavailable"))?;
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
    pub fn consistent_backup(&self, target: &Path) -> Result<()> {
        let db = self.db.lock().map_err(|_| fail("database_unavailable"))?;
        db.backup("main", target, None)
            .map_err(|_| fail("database_backup_failed"))?;
        private_file(target)
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
}
