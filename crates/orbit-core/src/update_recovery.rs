//! A filesystem update transaction, independent of SQLite schema compatibility.
//! No vault, network or client configuration adapter is used in this module.
use crate::{
    error::{Result, fail},
    identity::{authorization_epoch_reference, canonical_origin, sha256, uuid, validate_uuid},
    store::{Store, private_directory, private_file},
    updates::{self, VerifiedArtifact},
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_JOURNAL: u64 = 16 * 1024 * 1024;
const MAX_REGISTRATION: u64 = 32 * 1024;
const MAX_DATABASE: u64 = 512 * 1024 * 1024;
const MAX_METADATA: usize = 64 * 1024 * 1024;
const FAMILY: [&str; 3] = ["orbit.sqlite3", "orbit.sqlite3-wal", "orbit.sqlite3-shm"];
const PLATFORM: &str = "darwin-aarch64";
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Prepared,
    Validating,
    RollingBack,
    Committed,
    RolledBack,
    Aborted,
}
impl Phase {
    fn terminal(self) -> bool {
        matches!(self, Self::Committed | Self::RolledBack | Self::Aborted)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    installation_id: String,
    root_hash: String,
    app_hash: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Publication {
    purpose: String,
    before: [Option<String>; 3],
    output_hash: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TreeSeal {
    mode: u32,
    records: BTreeMap<String, Value>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Retirement {
    staged: TreeSeal,
    operation: TreeSeal,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    format: u32,
    operation_id: String,
    registration: Registration,
    platform: String,
    from_version: String,
    to_version: String,
    old_tree: String,
    new_tree: String,
    backup_hash: String,
    baseline_content: String,
    phase: Phase,
    publication: Option<Publication>,
    #[serde(default)]
    previous_publication: Option<Publication>,
    #[serde(default)]
    retiring: bool,
    #[serde(default)]
    retirement: Option<Retirement>,
}
struct Layout {
    root: PathBuf,
    current: PathBuf,
    operation: PathBuf,
    staged: PathBuf,
}
impl Layout {
    fn new(root: &Path, current: &Path, id: &str) -> Result<Self> {
        validate_uuid(id)?;
        let root = canonical_directory(root)?;
        let current = canonical_directory(current)?;
        if current.extension().and_then(|s| s.to_str()) != Some("app") {
            return Err(fail("update_bundle_invalid"));
        }
        let parent = current.parent().ok_or_else(|| fail("invalid_path"))?;
        Ok(Self {
            operation: root.join("updates").join(id),
            staged: parent.join(format!(".orbit-staged-{id}.app")),
            root,
            current,
        })
    }
    fn pending(&self) -> PathBuf {
        self.root.join("updates/pending.json")
    }
    fn file(&self, name: &str) -> PathBuf {
        self.operation.join(name)
    }
    fn save(&self, journal: &Journal) -> Result<()> {
        let bytes = serde_json::to_vec(journal).map_err(|_| fail("update_recovery_invalid"))?;
        if bytes.len() as u64 > MAX_JOURNAL {
            return Err(fail("update_resource_limit"));
        }
        crate::config::atomic_write(&self.pending(), &bytes)
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum BootDisposition {
    Ready,
    RestartRequired,
}
pub struct PreparedUpdate {
    layout: Layout,
    journal: Journal,
}
trait Installer {
    fn swap(&self, current: &Path, staged: &Path) -> Result<()>;
}
struct NativeInstaller;
impl Installer for NativeInstaller {
    fn swap(&self, current: &Path, staged: &Path) -> Result<()> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            updates::swap_macos_bundle(current, staged)
        }
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            let _ = (current, staged);
            Err(fail("platform_unsupported"))
        }
    }
}
trait Faults {
    fn checkpoint(&self, _name: &str) -> Result<()> {
        Ok(())
    }
}
struct NoFaults;
impl Faults for NoFaults {}

fn canonical_directory(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute()
        || path.components().any(|p| {
            !matches!(
                p,
                Component::RootDir | Component::Prefix(_) | Component::Normal(_)
            )
        })
    {
        return Err(fail("update_recovery_invalid"));
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| fail("update_recovery_invalid"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(fail("update_recovery_invalid"));
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| fail("update_recovery_invalid"))?;
    if canonical != path || canonical.to_str().is_none() {
        return Err(fail("update_recovery_invalid"));
    }
    Ok(canonical)
}
fn digest_file(path: &Path, maximum: u64) -> Result<String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| fail("update_recovery_conflict"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > maximum {
        return Err(fail("update_resource_limit"));
    }
    let mut file = File::open(path).map_err(|_| fail("update_recovery_conflict"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut length = 0u64;
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|_| fail("update_recovery_conflict"))?;
        if n == 0 {
            break;
        }
        length = length
            .checked_add(n as u64)
            .ok_or_else(|| fail("update_resource_limit"))?;
        if length > maximum {
            return Err(fail("update_resource_limit"));
        }
        hash.update(&buffer[..n]);
    }
    if length != metadata.len() {
        return Err(fail("update_recovery_conflict"));
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn optional_digest(path: &Path) -> Result<Option<String>> {
    match fs::symlink_metadata(path) {
        Ok(_) => digest_file(path, MAX_DATABASE).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(fail("update_recovery_conflict")),
    }
}
fn file_mode(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        u32::from(metadata.permissions().readonly())
    }
}
fn tree_records(root: &Path) -> Result<BTreeMap<String, Value>> {
    tree_records_inner(root, false)
}
fn tree_records_inner(root: &Path, retiring: bool) -> Result<BTreeMap<String, Value>> {
    let root = canonical_directory(root)?;
    let mut pending = vec![root.clone()];
    let mut records = BTreeMap::new();
    let mut bytes = 0u64;
    let mut metadata_bytes = 0usize;
    while let Some(directory) = pending.pop() {
        for item in fs::read_dir(directory).map_err(|_| fail("update_recovery_conflict"))? {
            let path = item.map_err(|_| fail("update_recovery_conflict"))?.path();
            let relative = path
                .strip_prefix(&root)
                .map_err(|_| fail("update_recovery_invalid"))?
                .to_str()
                .ok_or_else(|| fail("update_recovery_invalid"))?
                .to_owned();
            if records.len() >= updates::MAX_ARCHIVE_ENTRIES {
                return Err(fail("update_resource_limit"));
            }
            metadata_bytes = metadata_bytes
                .checked_add(relative.len().saturating_mul(6))
                .and_then(|n| n.checked_add(256))
                .ok_or_else(|| fail("update_resource_limit"))?;
            if metadata_bytes > MAX_METADATA {
                return Err(fail("update_resource_limit"));
            }
            let metadata =
                fs::symlink_metadata(&path).map_err(|_| fail("update_recovery_conflict"))?;
            let mode = file_mode(&metadata);
            let content = if metadata.file_type().is_symlink() {
                let link = fs::read_link(&path).map_err(|_| fail("update_recovery_conflict"))?;
                if link.is_absolute()
                    || !retiring && !path.canonicalize().is_ok_and(|p| p.starts_with(&root))
                {
                    return Err(fail("update_recovery_invalid"));
                }
                json!([
                    "link",
                    mode,
                    link.to_str()
                        .ok_or_else(|| fail("update_recovery_invalid"))?
                ])
            } else if metadata.is_dir() {
                pending.push(path.clone());
                json!(["directory", mode])
            } else if metadata.is_file() {
                bytes = bytes
                    .checked_add(metadata.len())
                    .ok_or_else(|| fail("update_resource_limit"))?;
                if bytes > updates::MAX_EXPANDED_BYTES {
                    return Err(fail("update_resource_limit"));
                }
                json!([
                    "file",
                    mode,
                    metadata.len(),
                    digest_file(&path, updates::MAX_UPDATE_BYTES)?
                ])
            } else {
                return Err(fail("update_recovery_invalid"));
            };
            records.insert(relative, content);
        }
    }
    Ok(records)
}
fn tree_digest(root: &Path) -> Result<String> {
    Ok(sha256(
        &serde_json::to_vec(&tree_records(root)?).map_err(|_| fail("update_recovery_invalid"))?,
    ))
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn version(value: &str) -> Result<Vec<u32>> {
    if value.len() > 32 {
        return Err(fail("update_recovery_invalid"));
    }
    let parts = value
        .split('.')
        .map(|v| {
            v.parse::<u32>()
                .map_err(|_| fail("update_recovery_invalid"))
        })
        .collect::<Result<Vec<_>>>()?;
    if parts.len() != 3 {
        return Err(fail("update_recovery_invalid"));
    }
    Ok(parts)
}
fn read_json<T: for<'a> Deserialize<'a>>(path: &Path) -> Result<T> {
    serde_json::from_slice(
        &crate::images::bounded_read(path, MAX_REGISTRATION)
            .map_err(|_| fail("update_recovery_invalid"))?,
    )
    .map_err(|_| fail("update_recovery_invalid"))
}
fn load(root: &Path) -> Result<Option<Journal>> {
    match fs::symlink_metadata(root.join("updates")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
        _ => return Err(fail("update_recovery_invalid")),
    }
    let pending = root.join("updates/pending.json");
    match fs::symlink_metadata(&pending) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(m) if m.is_file() && !m.file_type().is_symlink() && m.len() <= MAX_JOURNAL => {}
        _ => return Err(fail("update_recovery_invalid")),
    }
    let root = canonical_directory(root)?;
    canonical_directory(&root.join("updates"))?;
    let journal: Journal = serde_json::from_slice(
        &crate::images::bounded_read(&pending, MAX_JOURNAL)
            .map_err(|_| fail("update_recovery_invalid"))?,
    )
    .map_err(|_| fail("update_recovery_invalid"))?;
    let registration: Registration = read_json(&root.join("updates/registration.json"))?;
    validate_uuid(&journal.operation_id)?;
    validate_uuid(&registration.installation_id)?;
    if journal.format != 1
        || journal.platform != PLATFORM
        || journal.registration.installation_id != registration.installation_id
        || journal.registration.root_hash != registration.root_hash
        || journal.registration.app_hash != registration.app_hash
        || registration.root_hash != sha256(root.to_string_lossy().as_bytes())
        || version(&journal.to_version)? <= version(&journal.from_version)?
        || [
            &journal.old_tree,
            &journal.new_tree,
            &journal.backup_hash,
            &journal.baseline_content,
            &registration.app_hash,
        ]
        .iter()
        .any(|h| !valid_hash(h))
    {
        return Err(fail("update_recovery_invalid"));
    }
    for publication in [&journal.publication, &journal.previous_publication]
        .into_iter()
        .flatten()
    {
        if !matches!(publication.purpose.as_str(), "upgrade" | "rollback")
            || !valid_hash(&publication.output_hash)
            || publication.before.iter().flatten().any(|h| !valid_hash(h))
        {
            return Err(fail("update_recovery_invalid"));
        }
    }
    if journal.retiring && !journal.phase.terminal()
        || journal.retiring != journal.retirement.is_some()
    {
        return Err(fail("update_recovery_invalid"));
    }
    if let Some(retirement) = &journal.retirement {
        for seal in [&retirement.staged, &retirement.operation] {
            if seal.mode > 0o7777 || seal.records.len() > updates::MAX_ARCHIVE_ENTRIES {
                return Err(fail("update_recovery_invalid"));
            }
            for (name, record) in &seal.records {
                if name.is_empty()
                    || name.len() > 4096
                    || Path::new(name)
                        .components()
                        .any(|c| !matches!(c, Component::Normal(_)))
                    || !record.is_array()
                {
                    return Err(fail("update_recovery_invalid"));
                }
            }
        }
    }
    Ok(Some(journal))
}
/// Helpers and ordinary Store::open reject even a malformed pending marker.
pub fn ensure_ready(root: &Path) -> Result<()> {
    if load(root)?.is_some_and(|j| !j.phase.terminal() || j.retiring) {
        return Err(fail("update_recovery_pending"));
    }
    Ok(())
}
fn feed_hash(hash: &mut Sha256, total: &mut usize, tag: u8, bytes: &[u8]) -> Result<()> {
    *total = total
        .checked_add(bytes.len())
        .and_then(|n| n.checked_add(9))
        .ok_or_else(|| fail("update_resource_limit"))?;
    if *total > MAX_METADATA {
        return Err(fail("update_resource_limit"));
    }
    hash.update([tag]);
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
    Ok(())
}
fn content_hash(path: &Path) -> Result<String> {
    use rusqlite::types::ValueRef;
    // SQLite READ_ONLY can still create/change WAL/SHM. Inspect a bounded private
    // family clone instead, never opening the live DB or immutable backup.
    let parent = path.parent().ok_or_else(|| fail("invalid_path"))?;
    canonical_directory(parent)?;
    let copy = tempfile::tempdir_in(parent).map_err(|_| fail("update_recovery_conflict"))?;
    private_directory(copy.path())?;
    let mut snapshots = Vec::new();
    let mut total_bytes = 0u64;
    for (i, suffix) in ["", "-wal", "-shm"].iter().enumerate() {
        let source = PathBuf::from(format!("{}{suffix}", path.to_string_lossy()));
        let hash = optional_digest(&source)?;
        if let Some(hash) = &hash {
            total_bytes = total_bytes
                .checked_add(
                    fs::symlink_metadata(&source)
                        .map_err(|_| fail("update_recovery_conflict"))?
                        .len(),
                )
                .ok_or_else(|| fail("update_resource_limit"))?;
            if total_bytes > MAX_DATABASE {
                return Err(fail("update_resource_limit"));
            }
            atomic_copy(&source, &copy.path().join(FAMILY[i]), hash)?;
        } else if i == 0 {
            return Err(fail("update_recovery_conflict"));
        }
        snapshots.push((source, hash));
    }
    for (source, hash) in snapshots {
        if optional_digest(&source)? != hash {
            return Err(fail("update_recovery_conflict"));
        }
    }
    let db = Connection::open_with_flags(
        copy.path().join(FAMILY[0]),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_| fail("update_recovery_conflict"))?;
    let check: String = db
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(|_| fail("update_recovery_conflict"))?;
    if check != "ok" {
        return Err(fail("update_recovery_conflict"));
    }
    let schema: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|_| fail("update_recovery_conflict"))?;
    let mut hash = Sha256::new();
    let mut total = 0usize;
    let mut row_count = 0usize;
    feed_hash(&mut hash, &mut total, 1, &schema.to_le_bytes())?;
    let mut tables = Vec::new();
    {
        let mut statement = db
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .map_err(|_| fail("update_recovery_conflict"))?;
        let mut rows = statement
            .query([])
            .map_err(|_| fail("update_recovery_conflict"))?;
        while let Some(row) = rows.next().map_err(|_| fail("update_recovery_conflict"))? {
            let ValueRef::Text(name) = row
                .get_ref(0)
                .map_err(|_| fail("update_recovery_conflict"))?
            else {
                return Err(fail("update_recovery_conflict"));
            };
            if name.len() > 4096 || tables.len() >= 256 {
                return Err(fail("update_resource_limit"));
            }
            tables.push(
                std::str::from_utf8(name)
                    .map_err(|_| fail("update_recovery_invalid"))?
                    .to_owned(),
            );
        }
    }
    let mut queries = vec![
        "SELECT type,name,tbl_name,coalesce(sql,'') FROM sqlite_master ORDER BY type,name"
            .to_owned(),
    ];
    for table in tables {
        let quoted = table.replace('"', "\"\"");
        let columns = db
            .prepare(&format!("SELECT * FROM \"{quoted}\""))
            .map_err(|_| fail("update_recovery_conflict"))?
            .column_count();
        if columns == 0 || columns > 256 {
            return Err(fail("update_resource_limit"));
        }
        let order = (1..=columns)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        queries.push(format!("SELECT * FROM \"{quoted}\" ORDER BY {order}"));
    }
    for sql in queries {
        feed_hash(&mut hash, &mut total, 5, sql.as_bytes())?;
        let mut statement = db
            .prepare(&sql)
            .map_err(|_| fail("update_recovery_conflict"))?;
        let columns = statement.column_count();
        let mut rows = statement
            .query([])
            .map_err(|_| fail("update_recovery_conflict"))?;
        while let Some(row) = rows.next().map_err(|_| fail("update_recovery_conflict"))? {
            row_count += 1;
            if row_count > 1_000_000 {
                return Err(fail("update_resource_limit"));
            }
            feed_hash(&mut hash, &mut total, 6, &(columns as u64).to_le_bytes())?;
            for i in 0..columns {
                match row
                    .get_ref(i)
                    .map_err(|_| fail("update_recovery_conflict"))?
                {
                    ValueRef::Null => feed_hash(&mut hash, &mut total, 0, &[])?,
                    ValueRef::Integer(n) => feed_hash(&mut hash, &mut total, 1, &n.to_le_bytes())?,
                    ValueRef::Real(n) => {
                        feed_hash(&mut hash, &mut total, 2, &n.to_bits().to_le_bytes())?
                    }
                    ValueRef::Text(bytes) => feed_hash(&mut hash, &mut total, 3, bytes)?,
                    ValueRef::Blob(bytes) => feed_hash(&mut hash, &mut total, 4, bytes)?,
                }
            }
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn atomic_copy(source: &Path, target: &Path, expected: &str) -> Result<()> {
    if digest_file(source, MAX_DATABASE)? != expected {
        return Err(fail("update_recovery_conflict"));
    }
    let parent = target.parent().ok_or_else(|| fail("invalid_path"))?;
    canonical_directory(parent)?;
    if target
        .try_exists()
        .map_err(|_| fail("update_recovery_conflict"))?
    {
        return Err(fail("update_recovery_conflict"));
    }
    let mut staged =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| fail("update_restore_failed"))?;
    private_file(staged.path())?;
    let mut file = File::open(source).map_err(|_| fail("update_restore_failed"))?;
    let mut length = 0u64;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|_| fail("update_restore_failed"))?;
        if n == 0 {
            break;
        }
        length += n as u64;
        if length > MAX_DATABASE {
            return Err(fail("update_resource_limit"));
        }
        hash.update(&buffer[..n]);
        staged
            .write_all(&buffer[..n])
            .map_err(|_| fail("update_restore_failed"))?;
    }
    if format!("{:x}", hash.finalize()) != expected {
        return Err(fail("update_recovery_conflict"));
    }
    staged
        .as_file()
        .sync_all()
        .map_err(|_| fail("update_restore_failed"))?;
    staged
        .persist_noclobber(target)
        .map_err(|_| fail("update_restore_failed"))?;
    File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|_| fail("update_restore_failed"))
}
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|_| fail("update_restore_failed"))
}
fn family(root: &Path) -> Result<[Option<String>; 3]> {
    Ok([
        optional_digest(&root.join(FAMILY[0]))?,
        optional_digest(&root.join(FAMILY[1]))?,
        optional_digest(&root.join(FAMILY[2]))?,
    ])
}
fn verify_publication(layout: &Layout, publication: &Publication) -> Result<bool> {
    let prior = layout.file(&format!("{}-prior", publication.purpose));
    let mut moved = true;
    for (i, name) in FAMILY.iter().enumerate() {
        let current = optional_digest(&layout.root.join(name))?;
        let quarantined = optional_digest(&prior.join(name))?;
        match &publication.before[i] {
            Some(expected) if quarantined.as_ref() == Some(expected) && current.is_none() => {}
            Some(expected) if current.as_ref() == Some(expected) && quarantined.is_none() => {
                moved = false;
            }
            Some(expected)
                if i == 0
                    && quarantined.as_ref() == Some(expected)
                    && current.as_ref() == Some(&publication.output_hash) => {}
            None if current.is_none() && quarantined.is_none() => {}
            None if i == 0
                && current.as_ref() == Some(&publication.output_hash)
                && quarantined.is_none() => {}
            _ => return Err(fail("update_recovery_conflict")),
        }
    }
    let current_main = optional_digest(&layout.root.join(FAMILY[0]))?;
    let prior_main = optional_digest(&prior.join(FAMILY[0]))?;
    let published = current_main.as_ref() == Some(&publication.output_hash)
        && (publication.before[0].as_ref() != Some(&publication.output_hash)
            || prior_main.is_some());
    if published && !moved {
        return Err(fail("update_recovery_conflict"));
    }
    Ok(published)
}
fn publish(
    layout: &Layout,
    journal: &mut Journal,
    output: &Path,
    purpose: &str,
    fault: &dyn Faults,
) -> Result<()> {
    let output_hash = digest_file(output, MAX_DATABASE)?;
    if journal
        .publication
        .as_ref()
        .is_none_or(|p| p.purpose != purpose)
    {
        if journal
            .publication
            .as_ref()
            .is_some_and(|p| p.purpose == "upgrade")
        {
            journal.previous_publication = journal.publication.clone();
        }
        journal.publication = Some(Publication {
            purpose: purpose.into(),
            before: family(&layout.root)?,
            output_hash: output_hash.clone(),
        });
        layout.save(journal)?;
        fault.checkpoint("publication_claim")?;
    }
    let publication = journal
        .publication
        .as_ref()
        .ok_or_else(|| fail("update_recovery_invalid"))?;
    if publication.output_hash != output_hash {
        return Err(fail("update_recovery_conflict"));
    }
    if verify_publication(layout, publication)? {
        return Ok(());
    }
    let prior = layout.file(&format!("{purpose}-prior"));
    private_directory(&prior)?;
    for (i, name) in FAMILY.iter().enumerate() {
        let source = layout.root.join(name);
        let target = prior.join(name);
        if publication.before[i].is_some() && optional_digest(&target)?.is_none() {
            if optional_digest(&source)? != publication.before[i] {
                return Err(fail("update_recovery_conflict"));
            }
            fs::rename(&source, &target).map_err(|_| fail("update_restore_failed"))?;
            sync_directory(&layout.root)?;
            sync_directory(&prior)?;
            fault.checkpoint(name)?;
        }
    }
    if !verify_publication(layout, publication)? {
        atomic_copy(output, &layout.root.join(FAMILY[0]), &output_hash)?;
        fault.checkpoint("database_published")?;
    }
    if !verify_publication(layout, publication)? {
        return Err(fail("update_restore_failed"));
    }
    Ok(())
}

/// Caller holds the maintenance lease and one helper-update exclusive lock.
pub(crate) fn ensure_configuration_idle(store: &Store) -> Result<()> {
    for journal in store.list::<crate::config::Journal>("journal", None)? {
        if !["committed", "restored"].contains(&journal.phase.as_str()) {
            return Err(fail("update_configuration_pending"));
        }
    }
    Ok(())
}
/// Caller holds the maintenance lease and one helper-update exclusive lock.
pub fn prepare_macos(
    store: &Store,
    current: &Path,
    extracted: &Path,
    artifact: &VerifiedArtifact,
) -> Result<PreparedUpdate> {
    if artifact.candidate().target != PLATFORM {
        return Err(fail("update_target_mismatch"));
    }
    updates::validate_macos_bundle(current, updates::CURRENT_VERSION)?;
    updates::validate_macos_bundle(extracted, &artifact.candidate().version)?;
    prepare(
        store,
        current,
        extracted,
        updates::CURRENT_VERSION,
        &artifact.candidate().version,
        &NoFaults,
    )
}
fn prepare(
    store: &Store,
    current: &Path,
    extracted: &Path,
    from: &str,
    to: &str,
    fault: &dyn Faults,
) -> Result<PreparedUpdate> {
    if version(to)? <= version(from)? {
        return Err(fail("update_target_mismatch"));
    }
    ensure_configuration_idle(store)?;
    retire_completed(&store.root, current, fault)?;
    let id = uuid();
    let layout = Layout::new(&store.root, current, &id)?;
    private_directory(&layout.operation)?;
    let registration_path = layout.root.join("updates/registration.json");
    let registration = if registration_path
        .try_exists()
        .map_err(|_| fail("update_recovery_invalid"))?
    {
        read_json::<Registration>(&registration_path)?
    } else {
        let registration = Registration {
            installation_id: uuid(),
            root_hash: sha256(layout.root.to_string_lossy().as_bytes()),
            app_hash: sha256(layout.current.to_string_lossy().as_bytes()),
        };
        crate::config::atomic_write(
            &registration_path,
            &serde_json::to_vec(&registration).map_err(|_| fail("update_recovery_invalid"))?,
        )?;
        registration
    };
    if registration.root_hash != sha256(layout.root.to_string_lossy().as_bytes())
        || registration.app_hash != sha256(layout.current.to_string_lossy().as_bytes())
    {
        return Err(fail("update_recovery_conflict"));
    }
    validate_uuid(&registration.installation_id)?;
    let backup = layout.file("before.sqlite3");
    store.consistent_backup(&backup)?;
    File::open(&backup)
        .and_then(|f| f.sync_all())
        .map_err(|_| fail("database_backup_failed"))?;
    sync_directory(&layout.operation)?;
    let baseline_content = content_hash(&backup)?;
    if content_hash(&layout.root.join(FAMILY[0]))? != baseline_content {
        return Err(fail("update_recovery_conflict"));
    }
    let old_tree = tree_digest(current)?;
    let new_tree = tree_digest(extracted)?;
    if old_tree == new_tree
        || layout
            .staged
            .try_exists()
            .map_err(|_| fail("update_recovery_conflict"))?
    {
        return Err(fail("update_recovery_conflict"));
    }
    fs::rename(extracted, &layout.staged).map_err(|_| fail("update_staging_failed"))?;
    sync_directory(layout.staged.parent().ok_or_else(|| fail("invalid_path"))?)?;
    let journal = Journal {
        format: 1,
        operation_id: id,
        registration,
        platform: PLATFORM.into(),
        from_version: from.into(),
        to_version: to.into(),
        old_tree,
        new_tree,
        backup_hash: digest_file(&backup, MAX_DATABASE)?,
        baseline_content,
        phase: Phase::Prepared,
        publication: None,
        previous_publication: None,
        retiring: false,
        retirement: None,
    };
    layout.save(&journal)?;
    fault.checkpoint("prepared")?;
    Ok(PreparedUpdate { layout, journal })
}
impl PreparedUpdate {
    pub fn swap(self) -> Result<()> {
        self.swap_with(&NativeInstaller, &NoFaults)
    }
    fn swap_with(self, installer: &dyn Installer, fault: &dyn Faults) -> Result<()> {
        if tree_digest(&self.layout.current)? != self.journal.old_tree
            || tree_digest(&self.layout.staged)? != self.journal.new_tree
            || digest_file(&self.layout.file("before.sqlite3"), MAX_DATABASE)?
                != self.journal.backup_hash
            || content_hash(&self.layout.root.join(FAMILY[0]))? != self.journal.baseline_content
        {
            return Err(fail("update_recovery_conflict"));
        }
        fault.checkpoint("before_swap")?;
        installer.swap(&self.layout.current, &self.layout.staged)?;
        fault.checkpoint("after_swap")
        // The durable phase deliberately remains prepared. Boot observes both trees.
    }
}

fn validate_working(layout: &Layout) -> Result<PathBuf> {
    let work = layout.file("validation");
    private_directory(&work)?;
    let source = layout.file("before.sqlite3");
    atomic_copy(
        &source,
        &work.join(FAMILY[0]),
        &digest_file(&source, MAX_DATABASE)?,
    )?;
    let store = Store::open_for_update_validation(&work)?;
    // Full metadata enumeration is bounded and deserialized without executing any
    // account, network, vault, helper deployment or configuration recovery code.
    crate::service::Service::validate_upgrade_store(&store)?;
    let output = layout.file("validated.sqlite3");
    store.consistent_backup(&output)?;
    drop(store);
    let _ = content_hash(&output)?;
    File::open(&output)
        .and_then(|f| f.sync_all())
        .map_err(|_| fail("update_validation_failed"))?;
    sync_directory(&layout.operation)?;
    Ok(output)
}
fn sanitize_backup(layout: &Layout, journal: &Journal) -> Result<PathBuf> {
    let output = layout.file("rollback.sqlite3");
    if output
        .try_exists()
        .map_err(|_| fail("update_recovery_conflict"))?
    {
        // Only a fully persisted sanitized output is reused. Its hash is claimed
        // before any live database file is moved.
        if journal.publication.as_ref().is_some_and(|p| {
            p.purpose == "rollback"
                && digest_file(&output, MAX_DATABASE).ok().as_ref() == Some(&p.output_hash)
        }) {
            return Ok(output);
        }
        // No publication claim means this is disposable, uncommitted workspace.
        // Rebuild from the still verified original backup after a crash here.
        if journal
            .publication
            .as_ref()
            .is_some_and(|p| p.purpose == "rollback")
        {
            return Err(fail("update_recovery_conflict"));
        }
        let _ = digest_file(&output, MAX_DATABASE)?;
        fs::remove_file(&output).map_err(|_| fail("update_restore_failed"))?;
    }
    let work = layout.file("rollback-work.sqlite3");
    if work
        .try_exists()
        .map_err(|_| fail("update_recovery_conflict"))?
    {
        // This path is a disposable operation-owned workspace, never the sole DB.
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let path = layout.file(&format!("rollback-work.sqlite3{suffix}"));
            if path
                .try_exists()
                .map_err(|_| fail("update_recovery_conflict"))?
            {
                let _ = digest_file(&path, MAX_DATABASE)?;
                fs::remove_file(path).map_err(|_| fail("update_restore_failed"))?;
            }
        }
    }
    atomic_copy(&layout.file("before.sqlite3"), &work, &journal.backup_hash)?;
    let mut db = Connection::open(&work).map_err(|_| fail("update_restore_failed"))?;
    db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")
        .map_err(|_| fail("update_restore_failed"))?;
    let schema: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|_| fail("update_restore_failed"))?;
    if schema != crate::SCHEMA_VERSION as i64 {
        return Err(fail("database_schema_newer"));
    }
    let records = {
        let mut statement = db.prepare("SELECT kind,id,scope,payload FROM records WHERE kind IN ('connection','binding','client-state') ORDER BY kind,id").map_err(|_| fail("update_restore_failed"))?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|_| fail("update_restore_failed"))?;
        let mut records = Vec::new();
        let mut size = 0usize;
        for row in rows {
            let row = row.map_err(|_| fail("update_restore_failed"))?;
            size = size
                .checked_add(row.3.len())
                .ok_or_else(|| fail("update_resource_limit"))?;
            if size > MAX_METADATA {
                return Err(fail("update_resource_limit"));
            }
            records.push(row);
        }
        records
    };
    let tx = db
        .transaction()
        .map_err(|_| fail("update_restore_failed"))?;
    let mut epochs: BTreeMap<String, u64> = BTreeMap::new();
    let mut values = Vec::new();
    for (kind, id, scope, payload) in records {
        let mut value: Value =
            serde_json::from_str(&payload).map_err(|_| fail("local_metadata_invalid"))?;
        if kind == "connection" || kind == "binding" {
            let origin = value[if kind == "connection" {
                "view"
            } else {
                "origin"
            }]
            .clone();
            let origin = if kind == "connection" {
                origin["panel_origin"].as_str()
            } else {
                origin.as_str()
            }
            .ok_or_else(|| fail("local_metadata_invalid"))?;
            if canonical_origin(origin)? != origin {
                return Err(fail("local_metadata_invalid"));
            }
            let installation = value["installation_id"]
                .as_str()
                .ok_or_else(|| fail("local_metadata_invalid"))?;
            validate_uuid(installation)?;
            let account = if kind == "connection" {
                value["view"]["account_id"].as_str()
            } else {
                value["account_id"].as_str()
            }
            .ok_or_else(|| fail("local_metadata_invalid"))?;
            let key = authorization_epoch_reference(origin, installation, account);
            let previous = value["authorization_epoch"].as_u64().unwrap_or(0);
            let old: Option<String> = tx
                .query_row("SELECT value FROM meta WHERE key=?1", [&key], |r| r.get(0))
                .optional()
                .map_err(|_| fail("update_restore_failed"))?;
            let old = old
                .map(|s| s.parse::<u64>().map_err(|_| fail("local_metadata_invalid")))
                .transpose()?
                .unwrap_or(0)
                .max(previous);
            epochs
                .entry(key)
                .and_modify(|v| *v = (*v).max(old))
                .or_insert(old);
        }
        if kind == "connection" {
            let mut cleanup = value["cleanup_references"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for field in ["session_reference", "credential_reference"] {
                if let Some(reference) = value[field].as_str() {
                    if !cleanup.contains(&json!(reference)) {
                        cleanup.push(json!(reference));
                    }
                }
            }
            value["cleanup_references"] = json!(cleanup);
            value["credential_reference"] = Value::Null;
            value["view"]["credential_revision"] = Value::Null;
            value["view"]["permission_state"] = json!("unavailable");
            value["view"]["revision"] = json!(
                value["view"]["revision"]
                    .as_u64()
                    .ok_or_else(|| fail("local_metadata_invalid"))?
                    .checked_add(1)
                    .ok_or_else(|| fail("local_metadata_invalid"))?
            );
            value["session_status"] = json!("logged_out");
            value["expires_at"] = json!(0);
            for field in [
                "session_expires_at",
                "credential_expires_at",
                "refresh_attempt",
                "api_key_id",
            ] {
                value[field] = Value::Null;
            }
            value["credential_state"] = json!("unavailable");
        } else if kind == "binding" {
            value["enabled"] = json!(false);
        } else {
            value["loaded_evidence"] = Value::Null;
            value["connection_evidence"] = Value::Null;
            value["loaded_state"] = json!("unconfirmed");
            value["connection_state"] = json!("not_tested");
        }
        values.push((kind, id, scope, value));
    }
    for (key, old) in &epochs {
        let next = old
            .checked_add(1)
            .ok_or_else(|| fail("local_metadata_invalid"))?;
        tx.execute("INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,next.to_string()]).map_err(|_| fail("update_restore_failed"))?;
    }
    for (kind, id, scope, mut value) in values {
        if kind == "connection" {
            let key = authorization_epoch_reference(
                value["view"]["panel_origin"].as_str().unwrap(),
                value["installation_id"].as_str().unwrap(),
                value["view"]["account_id"].as_str().unwrap(),
            );
            value["authorization_epoch"] = json!(
                epochs[&key]
                    .checked_add(1)
                    .ok_or_else(|| fail("local_metadata_invalid"))?
            );
        }
        tx.execute(
            "UPDATE records SET payload=?1 WHERE kind=?2 AND id=?3 AND scope=?4",
            params![
                serde_json::to_string(&value).map_err(|_| fail("local_metadata_invalid"))?,
                kind,
                id,
                scope
            ],
        )
        .map_err(|_| fail("update_restore_failed"))?;
    }
    tx.execute("DELETE FROM records WHERE kind IN ('account-cache','authorization-pending','authorization-terminal','update-candidate')", []).map_err(|_| fail("update_restore_failed"))?;
    tx.execute("DELETE FROM meta WHERE key IN ('active_connection','preview_runtime_nonce','maintenance_active')", []).map_err(|_| fail("update_restore_failed"))?;
    tx.commit().map_err(|_| fail("update_restore_failed"))?;
    db.backup("main", &output, None)
        .map_err(|_| fail("update_restore_failed"))?;
    drop(db);
    private_file(&output)?;
    let _ = content_hash(&output)?;
    File::open(&output)
        .and_then(|f| f.sync_all())
        .map_err(|_| fail("update_restore_failed"))?;
    sync_directory(&layout.operation)?;
    Ok(output)
}

fn trees(layout: &Layout, journal: &Journal) -> Result<bool> {
    let current = tree_digest(&layout.current)?;
    let staged = tree_digest(&layout.staged)?;
    if current == journal.old_tree && staged == journal.new_tree {
        Ok(false)
    } else if current == journal.new_tree && staged == journal.old_tree {
        Ok(true)
    } else {
        Err(fail("update_recovery_conflict"))
    }
}
fn verify_backup(layout: &Layout, journal: &Journal) -> Result<()> {
    canonical_directory(&layout.operation)?;
    if digest_file(&layout.file("before.sqlite3"), MAX_DATABASE)? != journal.backup_hash
        || content_hash(&layout.file("before.sqlite3"))? != journal.baseline_content
    {
        return Err(fail("update_recovery_conflict"));
    }
    Ok(())
}
fn rollback(
    layout: &Layout,
    journal: &mut Journal,
    installer: &dyn Installer,
    fault: &dyn Faults,
) -> Result<()> {
    verify_backup(layout, journal)?;
    if let Some(previous) = &journal.previous_publication {
        verify_quarantine(layout, previous, false)?;
    }
    let swapped = trees(layout, journal)?;
    if let Some(publication) = journal.publication.as_ref() {
        verify_publication(layout, publication)?;
    } else if content_hash(&layout.root.join(FAMILY[0]))? != journal.baseline_content {
        return Err(fail("update_recovery_conflict"));
    }
    let sanitized = sanitize_backup(layout, journal)?;
    publish(layout, journal, &sanitized, "rollback", fault)?;
    if swapped {
        fault.checkpoint("before_rollback_swap")?;
        installer.swap(&layout.current, &layout.staged)?;
        fault.checkpoint("after_rollback_swap")?;
    }
    if trees(layout, journal)? {
        return Err(fail("update_recovery_conflict"));
    }
    journal.phase = Phase::RolledBack;
    layout.save(journal)?;
    fault.checkpoint("rolled_back")
}
/// Called after GUI RuntimeLock acquisition and before ordinary Store::open.
/// The sole helper exclusive lock is released before normal helper deployment.
pub fn boot(root: &Path, current: Option<&Path>, running_version: &str) -> Result<BootDisposition> {
    if load(root)?.is_none() {
        return Ok(BootDisposition::Ready);
    }
    let _helper = updates::exclusive_helper_lock(root)?;
    boot_with(
        root,
        current,
        running_version,
        &NativeInstaller,
        &NoFaults,
        &validate_working,
    )
}
fn boot_with(
    root: &Path,
    current: Option<&Path>,
    running_version: &str,
    installer: &dyn Installer,
    fault: &dyn Faults,
    validate: &dyn Fn(&Layout) -> Result<PathBuf>,
) -> Result<BootDisposition> {
    let Some(mut journal) = load(root)? else {
        return Ok(BootDisposition::Ready);
    };
    let layout = Layout::new(
        root,
        current.ok_or_else(|| fail("update_recovery_conflict"))?,
        &journal.operation_id,
    )?;
    if sha256(layout.current.to_string_lossy().as_bytes()) != journal.registration.app_hash {
        return Err(fail("update_recovery_conflict"));
    }
    if journal.phase.terminal() {
        let expected = if journal.phase == Phase::Committed {
            &journal.new_tree
        } else {
            &journal.old_tree
        };
        if tree_digest(&layout.current)? != *expected {
            return Err(fail("update_recovery_conflict"));
        }
        let expected_version = if journal.phase == Phase::Committed {
            &journal.to_version
        } else {
            &journal.from_version
        };
        if running_version != expected_version {
            return Ok(BootDisposition::RestartRequired);
        }
        if journal.retiring {
            retire_completed(root, &layout.current, fault)?;
        }
        return Ok(BootDisposition::Ready);
    }
    verify_backup(&layout, &journal)?;
    let swapped = trees(&layout, &journal)?;
    if journal.phase == Phase::Prepared {
        if content_hash(&layout.root.join(FAMILY[0]))? != journal.baseline_content {
            return Err(fail("update_recovery_conflict"));
        }
        if !swapped {
            if running_version != journal.from_version {
                return Err(fail("update_recovery_conflict"));
            }
            journal.phase = Phase::Aborted;
            layout.save(&journal)?;
            return Ok(BootDisposition::Ready);
        }
        if running_version != journal.to_version {
            return Err(fail("update_recovery_conflict"));
        }
        journal.phase = Phase::Validating;
        layout.save(&journal)?;
        fault.checkpoint("validating")?;
        match validate(&layout) {
            Ok(output) => {
                if content_hash(&layout.root.join(FAMILY[0]))? != journal.baseline_content {
                    return Err(fail("update_recovery_conflict"));
                }
                publish(&layout, &mut journal, &output, "upgrade", fault)?;
                journal.phase = Phase::Committed;
                layout.save(&journal)?;
                fault.checkpoint("committed")?;
                return Ok(BootDisposition::Ready);
            }
            Err(_) => {}
        }
    }
    // An earlier validating process did not commit. Do not infer success from
    // schema version, tree position or a validation output file.
    journal.phase = Phase::RollingBack;
    layout.save(&journal)?;
    fault.checkpoint("rolling_back")?;
    rollback(&layout, &mut journal, installer, fault)?;
    Ok(BootDisposition::RestartRequired)
}

fn verify_quarantine(layout: &Layout, publication: &Publication, complete: bool) -> Result<()> {
    let directory = layout.file(&format!("{}-prior", publication.purpose));
    for (i, name) in FAMILY.iter().enumerate() {
        let actual = optional_digest(&directory.join(name))?;
        if actual.is_some() && actual != publication.before[i]
            || complete && actual != publication.before[i]
        {
            return Err(fail("update_recovery_conflict"));
        }
    }
    Ok(())
}
fn seal_tree(root: &Path) -> Result<TreeSeal> {
    canonical_directory(root)?;
    Ok(TreeSeal {
        mode: file_mode(&fs::symlink_metadata(root).map_err(|_| fail("update_recovery_conflict"))?),
        records: tree_records(root)?,
    })
}
fn remaining_tree(root: &Path, seal: &TreeSeal) -> Result<BTreeMap<String, Value>> {
    match fs::symlink_metadata(root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() && file_mode(&m) == seal.mode => {}
        _ => return Err(fail("update_recovery_conflict")),
    }
    // A sealed symlink may become dangling as its owned target is retired. Never
    // follow it; the exact recorded link text and mode must still match.
    let remaining = tree_records_inner(root, true)?;
    if remaining
        .iter()
        .any(|(name, record)| seal.records.get(name) != Some(record))
    {
        return Err(fail("update_recovery_conflict"));
    }
    Ok(remaining)
}
fn retirement_record(path: &Path) -> Result<Value> {
    let metadata = fs::symlink_metadata(path).map_err(|_| fail("update_recovery_conflict"))?;
    let mode = file_mode(&metadata);
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(path).map_err(|_| fail("update_recovery_conflict"))?;
        Ok(json!([
            "link",
            mode,
            target
                .to_str()
                .ok_or_else(|| fail("update_recovery_invalid"))?
        ]))
    } else if metadata.is_dir() {
        Ok(json!(["directory", mode]))
    } else if metadata.is_file() {
        Ok(json!([
            "file",
            mode,
            metadata.len(),
            digest_file(path, updates::MAX_UPDATE_BYTES)?
        ]))
    } else {
        Err(fail("update_recovery_conflict"))
    }
}
fn retire_tree(root: &Path, seal: &TreeSeal, fault: &dyn Faults) -> Result<()> {
    let remaining = remaining_tree(root, seal)?;
    let mut names = remaining.keys().collect::<Vec<_>>();
    names.sort_by_key(|name| std::cmp::Reverse((name.split('/').count(), name.as_str())));
    for name in names {
        let path = root.join(name);
        let parent = path.parent().ok_or_else(|| fail("invalid_path"))?;
        if parent.canonicalize().ok().as_deref() != Some(parent) {
            return Err(fail("update_recovery_conflict"));
        }
        if Some(&retirement_record(&path)?) != seal.records.get(name) {
            return Err(fail("update_recovery_conflict"));
        }
        if remaining[name][0] == "directory" {
            fs::remove_dir(&path)
        } else {
            fs::remove_file(&path)
        }
        .map_err(|_| fail("update_restore_failed"))?;
        sync_directory(parent)?;
        fault.checkpoint("retirement_entry")?;
    }
    match fs::symlink_metadata(root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() && file_mode(&m) == seal.mode => {}
        _ => return Err(fail("update_recovery_conflict")),
    }
    fs::remove_dir(root).map_err(|_| fail("update_restore_failed"))?;
    sync_directory(root.parent().ok_or_else(|| fail("invalid_path"))?)?;
    fault.checkpoint("retirement_root")
}
fn known_operation_material(records: &BTreeMap<String, Value>) -> bool {
    records.iter().all(|(name, record)| {
        if record[0] == "link" {
            return false;
        }
        let mut parts = name.split('/');
        let head = parts.next().unwrap();
        let tail = parts.next();
        if parts.next().is_some() {
            return false;
        }
        match tail {
            None if ["validation", "upgrade-prior", "rollback-prior"].contains(&head) => {
                record[0] == "directory"
            }
            None => {
                record[0] == "file"
                    && [
                        "before.sqlite3",
                        "before.sqlite3-wal",
                        "before.sqlite3-shm",
                        "validated.sqlite3",
                        "rollback-work.sqlite3",
                        "rollback-work.sqlite3-wal",
                        "rollback-work.sqlite3-shm",
                        "rollback-work.sqlite3-journal",
                        "rollback.sqlite3",
                    ]
                    .contains(&head)
            }
            Some(tail) if ["upgrade-prior", "rollback-prior"].contains(&head) => {
                record[0] == "file" && FAMILY.contains(&tail)
            }
            Some(tail) if head == "validation" => {
                record[0] == "file" && FAMILY.contains(&tail)
                    || record[0] == "directory"
                        && [
                            "images",
                            "previews",
                            "inputs",
                            "pending-save",
                            "backups",
                            "journals",
                            "locks",
                            "updates",
                        ]
                        .contains(&tail)
            }
            _ => false,
        }
    })
}
fn retire_completed(root: &Path, current: &Path, fault: &dyn Faults) -> Result<()> {
    let Some(mut journal) = load(root)? else {
        return Ok(());
    };
    if !journal.phase.terminal() {
        return Err(fail("update_recovery_pending"));
    }
    let layout = Layout::new(root, current, &journal.operation_id)?;
    let expected = if journal.phase == Phase::Committed {
        &journal.new_tree
    } else {
        &journal.old_tree
    };
    if tree_digest(current)? != *expected
        || sha256(layout.current.to_string_lossy().as_bytes()) != journal.registration.app_hash
    {
        return Err(fail("update_recovery_conflict"));
    }
    if !journal.retiring {
        verify_backup(&layout, &journal)?;
        let staged_expected = if journal.phase == Phase::Committed {
            &journal.old_tree
        } else {
            &journal.new_tree
        };
        if tree_digest(&layout.staged)? != *staged_expected {
            return Err(fail("update_recovery_conflict"));
        }
        if let Some(publication) = &journal.publication {
            verify_quarantine(&layout, publication, true)?;
        }
        if let Some(previous) = &journal.previous_publication {
            verify_quarantine(&layout, previous, false)?;
        }
        let operation = seal_tree(&layout.operation)?;
        if !known_operation_material(&operation.records) {
            return Err(fail("update_recovery_conflict"));
        }
        journal.retirement = Some(Retirement {
            staged: seal_tree(&layout.staged)?,
            operation,
        });
        journal.retiring = true;
        layout.save(&journal)?;
        fault.checkpoint("retiring")?;
    }
    let retirement = journal
        .retirement
        .as_ref()
        .ok_or_else(|| fail("update_recovery_invalid"))?;
    // Check both remaining trees before deleting any item. Unknown or changed
    // material is preserved, including after a previous partial retirement.
    remaining_tree(&layout.staged, &retirement.staged)?;
    remaining_tree(&layout.operation, &retirement.operation)?;
    retire_tree(&layout.staged, &retirement.staged, fault)?;
    retire_tree(&layout.operation, &retirement.operation, fault)?;
    fault.checkpoint("retirement_operation")?;
    fs::remove_file(layout.pending()).map_err(|_| fail("update_restore_failed"))?;
    sync_directory(&root.join("updates"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{
        dto::ClientKind,
        identity::{HelperBinding, bound_helper_token},
        vault::Vault,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    struct FakeInstaller {
        calls: AtomicU64,
    }
    #[derive(Default)]
    struct NoCallsVault(AtomicU64);
    impl Vault for NoCallsVault {
        fn get(&self, _: &str) -> Result<zeroize::Zeroizing<Vec<u8>>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(fail("unexpected_vault_call"))
        }
        fn put(&self, _: &str, _: &[u8]) -> Result<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(fail("unexpected_vault_call"))
        }
        fn delete(&self, _: &str) -> Result<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(fail("unexpected_vault_call"))
        }
    }
    impl FakeInstaller {
        fn new() -> Self {
            Self {
                calls: AtomicU64::new(0),
            }
        }
    }
    impl Installer for FakeInstaller {
        fn swap(&self, current: &Path, staged: &Path) -> Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let temporary = current.parent().unwrap().join("fake-swap-only.app");
            fs::rename(current, &temporary).unwrap();
            fs::rename(staged, current).unwrap();
            fs::rename(temporary, staged).unwrap();
            sync_directory(current.parent().unwrap())
        }
    }
    struct CrashAt(&'static str);
    impl Faults for CrashAt {
        fn checkpoint(&self, name: &str) -> Result<()> {
            if name == self.0 {
                Err(fail("synthetic_crash"))
            } else {
                Ok(())
            }
        }
    }
    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        app: PathBuf,
        extracted: PathBuf,
        id: String,
        binding: HelperBinding,
    }
    fn fixture() -> (Fixture, Store) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data");
        let app = dir.path().join("Orbit.app");
        let extracted = dir.path().join("New.app");
        for (path, contents) in [
            (&app, b"old synthetic bundle".as_slice()),
            (&extracted, b"new synthetic bundle".as_slice()),
        ] {
            fs::create_dir_all(path.join("Contents/MacOS")).unwrap();
            fs::write(path.join("Contents/MacOS/fixture"), contents).unwrap();
        }
        let store = Store::open(&root).unwrap();
        let id = uuid();
        let installation = uuid();
        let binding = HelperBinding {
            id: uuid(),
            client: ClientKind::Codex,
            origin: "https://panel.example.test".into(),
            account_id: "synthetic-account".into(),
            installation_id: installation.clone(),
            provider_id: "openai".into(),
            credential_revision: "f".repeat(64),
            secret_reference: "synthetic-inference-ref".into(),
            enabled: true,
            expires_at: None,
            authorization_epoch: 7,
        };
        let record = json!({"view":{"id":id,"label":"synthetic","panel_origin":binding.origin,"account_id":binding.account_id,"node_id":null,"node_label":null,"edge_base_url":null,"provider_id":"openai","revision":4,"credential_revision":binding.credential_revision,"permission_state":"allowed"},"installation_id":installation,"session_reference":"synthetic-session-ref","credential_reference":"synthetic-inference-ref","session_status":"authenticated","expires_at":9999999999i64,"session_expires_at":9999999999i64,"refresh_attempt":null,"credential_expires_at":null,"credential_state":"active","api_key_id":"synthetic-key","authorization_epoch":7,"cleanup_references":["older-cleanup-ref"]});
        store
            .put("connection", &id, &binding.account_id, &record)
            .unwrap();
        store
            .put("binding", &binding.id, &binding.account_id, &binding)
            .unwrap();
        store.set_meta("active_connection", &id).unwrap();
        store
            .set_meta(
                &authorization_epoch_reference(&binding.origin, &installation, &binding.account_id),
                "7",
            )
            .unwrap();
        let evidence = json!({"kind":"synthetic","observed_at":"2026-10-14T00:00:00Z","configuration_revision":"synthetic-config","client_version":"synthetic","account_id":binding.account_id,"connection_id":id,"connection_revision":4,"node_id":"synthetic-node","model_id":"synthetic-model"});
        store.put("client-state", "codex", "local", &json!({"client":"codex","installed":false,"display_name":"Synthetic Codex","version":null,"bundle_id":null,"app_path":null,"config_paths":[],"platform_supported":true,"managed":"no","configuration_state":"saved","loaded_state":"confirmed","connection_state":"passed","configuration_revision":"synthetic-config","loaded_evidence":evidence,"connection_evidence":evidence,"manual_steps":[]})).unwrap();
        (
            Fixture {
                _dir: dir,
                root,
                app,
                extracted,
                id,
                binding,
            },
            store,
        )
    }
    fn prepared(f: &Fixture, store: &Store) -> PreparedUpdate {
        prepare(store, &f.app, &f.extracted, "0.1.0", "0.2.0", &NoFaults).unwrap()
    }
    fn run(
        f: &Fixture,
        running: &str,
        swap: &FakeInstaller,
        fault: &dyn Faults,
    ) -> Result<BootDisposition> {
        boot_with(
            &f.root,
            Some(&f.app),
            running,
            swap,
            fault,
            &validate_working,
        )
    }
    fn assert_sanitized(f: &Fixture) {
        let store = Store::open(&f.root).unwrap();
        assert!(store.meta("active_connection").unwrap().is_none());
        let record: Value = store.get("connection", &f.id).unwrap().unwrap();
        assert_eq!(record["session_status"], "logged_out");
        assert!(record["credential_reference"].is_null());
        assert_eq!(record["view"]["revision"], 5);
        assert_eq!(record["authorization_epoch"], 8);
        for reference in [
            "synthetic-session-ref",
            "synthetic-inference-ref",
            "older-cleanup-ref",
        ] {
            assert!(
                record["cleanup_references"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(reference))
            );
        }
        assert!(
            store
                .list::<Value>("account-cache", None)
                .unwrap()
                .is_empty()
        );
        let client: Value = store.get("client-state", "codex").unwrap().unwrap();
        assert!(client["loaded_evidence"].is_null() && client["connection_evidence"].is_null());
        assert_eq!(client["loaded_state"], "unconfirmed");
        assert_eq!(client["connection_state"], "not_tested");
        let binding: HelperBinding = store.get("binding", &f.binding.id).unwrap().unwrap();
        assert!(!binding.enabled);
        assert_eq!(
            store
                .meta(&authorization_epoch_reference(
                    &binding.origin,
                    &binding.installation_id,
                    &binding.account_id
                ))
                .unwrap()
                .as_deref(),
            Some("8")
        );
        struct CountingVault(AtomicU64);
        impl Vault for CountingVault {
            fn get(&self, _: &str) -> Result<zeroize::Zeroizing<Vec<u8>>> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err(fail("unexpected_vault_call"))
            }
            fn put(&self, _: &str, _: &[u8]) -> Result<()> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err(fail("unexpected_vault_call"))
            }
            fn delete(&self, _: &str) -> Result<()> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err(fail("unexpected_vault_call"))
            }
        }
        let vault = CountingVault(AtomicU64::new(0));
        assert!(
            bound_helper_token(&binding, ClientKind::Codex, &binding.id, &store, &vault, 1)
                .is_err()
        );
        assert_eq!(vault.0.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn prepared_before_swap_aborts_and_after_swap_boot_observes_without_blind_reswap() {
        for swapped in [false, true] {
            let (f, store) = fixture();
            let update = prepared(&f, &store);
            drop(store);
            let swap = FakeInstaller::new();
            if swapped {
                assert_eq!(
                    update
                        .swap_with(&swap, &CrashAt("after_swap"))
                        .unwrap_err()
                        .code,
                    "synthetic_crash"
                );
            }
            assert_eq!(load(&f.root).unwrap().unwrap().phase, Phase::Prepared);
            assert_eq!(
                run(
                    &f,
                    if swapped { "0.2.0" } else { "0.1.0" },
                    &swap,
                    &NoFaults
                )
                .unwrap(),
                BootDisposition::Ready
            );
            assert_eq!(swap.calls.load(Ordering::SeqCst), u64::from(swapped));
            assert_eq!(
                load(&f.root).unwrap().unwrap().phase,
                if swapped {
                    Phase::Committed
                } else {
                    Phase::Aborted
                }
            );
        }
    }
    #[test]
    fn validating_crash_and_initialization_failure_restore_without_reviving_authority() {
        for failure in ["validating", "initializer", "new-schema"] {
            let (f, store) = fixture();
            store
                .put(
                    "account-cache",
                    &f.id,
                    "synthetic-account",
                    &json!({"cached":"synthetic-wallet"}),
                )
                .unwrap();
            let update = prepared(&f, &store);
            drop(store);
            let swap = FakeInstaller::new();
            update.swap_with(&swap, &NoFaults).unwrap();
            if failure == "validating" {
                assert_eq!(
                    run(&f, "0.2.0", &swap, &CrashAt("validating"))
                        .unwrap_err()
                        .code,
                    "synthetic_crash"
                );
                assert_eq!(load(&f.root).unwrap().unwrap().phase, Phase::Validating);
            } else {
                let failing = |layout: &Layout| -> Result<PathBuf> {
                    if failure == "new-schema" {
                        let work = layout.file("bad-schema");
                        private_directory(&work)?;
                        atomic_copy(
                            &layout.file("before.sqlite3"),
                            &work.join(FAMILY[0]),
                            &digest_file(&layout.file("before.sqlite3"), MAX_DATABASE)?,
                        )?;
                        let db = Connection::open(work.join(FAMILY[0])).unwrap();
                        db.execute_batch("PRAGMA user_version=2").unwrap();
                        drop(db);
                        assert_eq!(
                            Store::open_for_update_validation(&work).err().unwrap().code,
                            "database_schema_newer"
                        );
                    }
                    Err(fail("synthetic_initialization_failure"))
                };
                assert_eq!(
                    boot_with(&f.root, Some(&f.app), "0.2.0", &swap, &NoFaults, &failing).unwrap(),
                    BootDisposition::RestartRequired
                );
            }
            if failure == "validating" {
                assert_eq!(
                    run(&f, "0.2.0", &swap, &NoFaults).unwrap(),
                    BootDisposition::RestartRequired
                );
            }
            assert_sanitized(&f);
            assert_eq!(swap.calls.load(Ordering::SeqCst), 2);
            assert_eq!(
                run(&f, "0.1.0", &swap, &NoFaults).unwrap(),
                BootDisposition::Ready
            );
            assert_eq!(swap.calls.load(Ordering::SeqCst), 2);
        }
    }
    #[test]
    fn validation_database_publication_crashes_always_rollback_before_normal_open() {
        for checkpoint in ["publication_claim", "orbit.sqlite3", "database_published"] {
            let (f, store) = fixture();
            let update = prepared(&f, &store);
            drop(store);
            let swap = FakeInstaller::new();
            update.swap_with(&swap, &NoFaults).unwrap();
            assert_eq!(
                run(&f, "0.2.0", &swap, &CrashAt(checkpoint))
                    .unwrap_err()
                    .code,
                "synthetic_crash"
            );
            assert_eq!(
                Store::open(&f.root).err().unwrap().code,
                "update_recovery_pending"
            );
            assert_eq!(
                run(&f, "0.2.0", &swap, &NoFaults).unwrap(),
                BootDisposition::RestartRequired
            );
            assert_sanitized(&f);
        }
    }
    #[test]
    fn rollback_database_and_wal_shm_moves_are_durable_and_reentrant() {
        for checkpoint in [
            "publication_claim",
            "orbit.sqlite3",
            "orbit.sqlite3-wal",
            "orbit.sqlite3-shm",
            "database_published",
            "after_rollback_swap",
        ] {
            let (f, store) = fixture();
            let update = prepared(&f, &store);
            drop(store);
            let swap = FakeInstaller::new();
            update.swap_with(&swap, &NoFaults).unwrap();
            let mut journal = load(&f.root).unwrap().unwrap();
            journal.phase = Phase::RollingBack;
            let layout = Layout::new(&f.root, &f.app, &journal.operation_id).unwrap();
            layout.save(&journal).unwrap();
            let output = sanitize_backup(&layout, &journal).unwrap();
            // Synthetic closed, claimed sidecar material is never opened as SQLite.
            fs::write(f.root.join(FAMILY[1]), b"owned synthetic WAL").unwrap();
            fs::write(f.root.join(FAMILY[2]), b"owned synthetic SHM").unwrap();
            journal.publication = Some(Publication {
                purpose: "rollback".into(),
                before: family(&f.root).unwrap(),
                output_hash: digest_file(&output, MAX_DATABASE).unwrap(),
            });
            layout.save(&journal).unwrap();
            let result = if checkpoint == "publication_claim" {
                Err(fail("synthetic_crash"))
            } else {
                run(&f, "0.2.0", &swap, &CrashAt(checkpoint))
            };
            assert_eq!(result.unwrap_err().code, "synthetic_crash", "{checkpoint}");
            assert_eq!(
                run(&f, "0.2.0", &swap, &NoFaults).unwrap(),
                BootDisposition::RestartRequired,
                "{checkpoint}"
            );
            assert_sanitized(&f);
            assert_eq!(
                fs::read(layout.file("rollback-prior").join(FAMILY[1])).unwrap(),
                b"owned synthetic WAL"
            );
            assert_eq!(
                fs::read(layout.file("rollback-prior").join(FAMILY[2])).unwrap(),
                b"owned synthetic SHM"
            );
        }
    }
    #[test]
    fn tampered_tree_backup_live_database_and_pending_json_never_overwrite() {
        for change in [
            "old-tree",
            "new-tree",
            "backup",
            "live-db",
            "foreign-table",
            "journal",
        ] {
            let (f, store) = fixture();
            if change == "foreign-table" {
                let db = Connection::open(f.root.join(FAMILY[0])).unwrap();
                db.execute_batch(
                    "CREATE TABLE extra(value TEXT); INSERT INTO extra VALUES('original')",
                )
                .unwrap();
            }
            let update = prepared(&f, &store);
            drop(store);
            let swap = FakeInstaller::new();
            update.swap_with(&swap, &NoFaults).unwrap();
            let journal = load(&f.root).unwrap().unwrap();
            let layout = Layout::new(&f.root, &f.app, &journal.operation_id).unwrap();
            match change {
                "old-tree" => fs::write(layout.staged.join("changed"), b"foreign").unwrap(),
                "new-tree" => fs::write(f.app.join("changed"), b"foreign").unwrap(),
                "backup" => fs::write(layout.file("before.sqlite3"), b"foreign").unwrap(),
                "live-db" => {
                    let db = Connection::open(f.root.join(FAMILY[0])).unwrap();
                    db.execute("INSERT INTO meta VALUES('foreign','preserve')", [])
                        .unwrap();
                }
                "foreign-table" => {
                    let db = Connection::open(f.root.join(FAMILY[0])).unwrap();
                    db.execute("UPDATE extra SET value='foreign'", []).unwrap();
                }
                "journal" => {
                    let mut raw = serde_json::to_value(journal).unwrap();
                    raw["arbitrary_restore_path"] = json!("/not-permitted");
                    crate::config::atomic_write(
                        &layout.pending(),
                        &serde_json::to_vec(&raw).unwrap(),
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            let before = family(&f.root).unwrap();
            assert!(run(&f, "0.2.0", &swap, &NoFaults).is_err(), "{change}");
            assert_eq!(family(&f.root).unwrap(), before, "{change}");
            assert_eq!(swap.calls.load(Ordering::SeqCst), 1);
        }
    }
    #[test]
    fn committed_boot_never_rolls_back_or_revalidates_normal_later_data_changes() {
        let (f, store) = fixture();
        let update = prepared(&f, &store);
        drop(store);
        let swap = FakeInstaller::new();
        update.swap_with(&swap, &NoFaults).unwrap();
        assert_eq!(
            run(&f, "0.2.0", &swap, &NoFaults).unwrap(),
            BootDisposition::Ready
        );
        let store = Store::open(&f.root).unwrap();
        store.set_meta("normal_later_change", "preserve").unwrap();
        drop(store);
        assert_eq!(
            run(&f, "0.2.0", &swap, &CrashAt("validating")).unwrap(),
            BootDisposition::Ready
        );
        let store = Store::open(&f.root).unwrap();
        assert_eq!(
            store.meta("normal_later_change").unwrap().as_deref(),
            Some("preserve")
        );
        assert_eq!(swap.calls.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn next_update_retires_only_verified_terminal_owned_material_and_reuses_registration() {
        let (f, store) = fixture();
        let update = prepared(&f, &store);
        drop(store);
        let old = load(&f.root).unwrap().unwrap();
        let swap = FakeInstaller::new();
        update.swap_with(&swap, &NoFaults).unwrap();
        run(&f, "0.2.0", &swap, &NoFaults).unwrap();
        fs::create_dir_all(f.extracted.join("Contents/MacOS")).unwrap();
        fs::write(
            f.extracted.join("Contents/MacOS/fixture"),
            b"third synthetic bundle",
        )
        .unwrap();
        let store = Store::open(&f.root).unwrap();
        let next = prepare(&store, &f.app, &f.extracted, "0.2.0", "0.3.0", &NoFaults).unwrap();
        assert_ne!(next.journal.operation_id, old.operation_id);
        assert_eq!(
            next.journal.registration.installation_id,
            old.registration.installation_id
        );
        assert!(!f.root.join("updates").join(old.operation_id).exists());
        assert_eq!(
            fs::read_dir(f.root.join("updates"))
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().unwrap().is_dir())
                .count(),
            1
        );
    }
    #[test]
    fn partial_terminal_retirement_resumes_from_sealed_remaining_material() {
        for checkpoint in [
            "retiring",
            "retirement_entry",
            "retirement_root",
            "retirement_operation",
        ] {
            let (f, store) = fixture();
            let update = prepared(&f, &store);
            drop(store);
            let swap = FakeInstaller::new();
            update.swap_with(&swap, &NoFaults).unwrap();
            run(&f, "0.2.0", &swap, &NoFaults).unwrap();
            let id = load(&f.root).unwrap().unwrap().operation_id;
            let layout = Layout::new(&f.root, &f.app, &id).unwrap();
            assert_eq!(
                retire_completed(&f.root, &f.app, &CrashAt(checkpoint))
                    .unwrap_err()
                    .code,
                "synthetic_crash"
            );
            let journal = load(&f.root).unwrap().unwrap();
            assert!(journal.retiring && journal.retirement.is_some());
            assert_eq!(
                ensure_ready(&f.root).unwrap_err().code,
                "update_recovery_pending"
            );
            assert_eq!(
                run(&f, "0.2.0", &swap, &NoFaults).unwrap(),
                BootDisposition::Ready
            );
            assert!(load(&f.root).unwrap().is_none());
            assert!(!layout.staged.exists() && !layout.operation.exists());
            assert_eq!(swap.calls.load(Ordering::SeqCst), 1);
        }
    }
    #[test]
    fn partial_retirement_preserves_changed_remaining_and_foreign_material() {
        for change in ["staged-foreign", "operation-foreign", "remaining-file"] {
            let (f, store) = fixture();
            let update = prepared(&f, &store);
            drop(store);
            let swap = FakeInstaller::new();
            update.swap_with(&swap, &NoFaults).unwrap();
            run(&f, "0.2.0", &swap, &NoFaults).unwrap();
            let journal = load(&f.root).unwrap().unwrap();
            let layout = Layout::new(&f.root, &f.app, &journal.operation_id).unwrap();
            assert_eq!(
                retire_completed(&f.root, &f.app, &CrashAt("retirement_entry"))
                    .unwrap_err()
                    .code,
                "synthetic_crash"
            );
            match change {
                "staged-foreign" => fs::write(layout.staged.join("foreign"), b"preserve").unwrap(),
                "operation-foreign" => fs::write(layout.file("foreign"), b"preserve").unwrap(),
                "remaining-file" => {
                    fs::write(layout.file("before.sqlite3"), b"changed preserve").unwrap()
                }
                _ => unreachable!(),
            }
            let staged = tree_records_inner(&layout.staged, true).unwrap();
            let operation = tree_records_inner(&layout.operation, true).unwrap();
            let live = family(&f.root).unwrap();
            assert_eq!(
                retire_completed(&f.root, &f.app, &NoFaults)
                    .unwrap_err()
                    .code,
                "update_recovery_conflict"
            );
            assert_eq!(tree_records_inner(&layout.staged, true).unwrap(), staged);
            assert_eq!(
                tree_records_inner(&layout.operation, true).unwrap(),
                operation
            );
            assert_eq!(family(&f.root).unwrap(), live);
            assert_eq!(swap.calls.load(Ordering::SeqCst), 1);
        }
    }
    #[test]
    fn unfinished_configuration_prevents_maintenance_and_preparation_before_any_material_change() {
        for phase in ["prepared", "publishing", "conflict", "unknown"] {
            let (f, store) = fixture();
            let store = Arc::new(store);
            let vault = Arc::new(NoCallsVault::default());
            let service = crate::service::Service::new(
                store.clone(),
                vault.clone(),
                crate::system::SystemPaths::isolated(&f.root).unwrap(),
            )
            .unwrap();
            let journal = json!({"id":uuid(),"client":"codex","account_id":f.binding.account_id,"account_label":"Synthetic","installation":{"id":f.binding.installation_id,"origin":f.binding.origin,"public_key":"synthetic","signing_reference":"synthetic-signing-ref","backup_reference":"synthetic-backup-ref"},"created_at":"2026-10-14T00:00:00Z","expires_at":"2026-11-14T00:00:00Z","phase":phase,"paths":[],"target_version":null,"target_bundle_id":null,"nonce":"synthetic","ciphertext":"synthetic"});
            store
                .put(
                    "journal",
                    journal["id"].as_str().unwrap(),
                    &f.binding.account_id,
                    &journal,
                )
                .unwrap();
            let old_tree = tree_digest(&f.app).unwrap();
            let new_tree = tree_digest(&f.extracted).unwrap();
            let revision = store.snapshot().unwrap().revision;
            let live = family(&f.root).unwrap();
            assert_eq!(
                service.begin_maintenance().err().unwrap().code,
                "update_configuration_pending"
            );
            assert_eq!(
                prepare(&store, &f.app, &f.extracted, "0.1.0", "0.2.0", &NoFaults)
                    .err()
                    .unwrap()
                    .code,
                "update_configuration_pending"
            );
            assert_eq!(tree_digest(&f.app).unwrap(), old_tree);
            assert_eq!(tree_digest(&f.extracted).unwrap(), new_tree);
            assert_eq!(family(&f.root).unwrap(), live);
            assert_eq!(store.snapshot().unwrap().revision, revision);
            assert_eq!(fs::read_dir(f.root.join("updates")).unwrap().count(), 0);
            assert_eq!(vault.0.load(Ordering::SeqCst), 0);
        }
    }
    #[cfg(unix)]
    #[test]
    fn dangling_pending_marker_blocks_ordinary_store_and_helper_without_vault_access() {
        use std::os::unix::fs::symlink;
        let (f, store) = fixture();
        let before = family(&f.root).unwrap();
        symlink(
            f.root.join("missing-update-marker"),
            f.root.join("updates/pending.json"),
        )
        .unwrap();
        let vault = NoCallsVault::default();
        assert_eq!(
            ensure_ready(&f.root).unwrap_err().code,
            "update_recovery_invalid"
        );
        assert_eq!(
            Store::open(&f.root).err().unwrap().code,
            "update_recovery_invalid"
        );
        assert_eq!(
            Store::open_readonly(&f.root).err().unwrap().code,
            "update_recovery_invalid"
        );
        assert_eq!(
            bound_helper_token(
                &f.binding,
                ClientKind::Codex,
                &f.binding.id,
                &store,
                &vault,
                1
            )
            .unwrap_err()
            .code,
            "update_recovery_invalid"
        );
        assert_eq!(vault.0.load(Ordering::SeqCst), 0);
        assert_eq!(family(&f.root).unwrap(), before);
    }
    #[test]
    fn published_owned_schema_two_restores_before_normal_open_but_foreign_schema_two_is_preserved()
    {
        for claimed in [true, false] {
            let (f, store) = fixture();
            let update = prepared(&f, &store);
            drop(store);
            let swap = FakeInstaller::new();
            update.swap_with(&swap, &NoFaults).unwrap();
            if claimed {
                let schema_two = |layout: &Layout| -> Result<PathBuf> {
                    let output = layout.file("validated.sqlite3");
                    atomic_copy(
                        &layout.file("before.sqlite3"),
                        &output,
                        &digest_file(&layout.file("before.sqlite3"), MAX_DATABASE)?,
                    )?;
                    let db = Connection::open(&output).unwrap();
                    db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA user_version=2")
                        .unwrap();
                    assert_eq!(
                        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                            .unwrap(),
                        2
                    );
                    drop(db);
                    Ok(output)
                };
                assert_eq!(
                    boot_with(
                        &f.root,
                        Some(&f.app),
                        "0.2.0",
                        &swap,
                        &CrashAt("database_published"),
                        &schema_two
                    )
                    .unwrap_err()
                    .code,
                    "synthetic_crash"
                );
                let journal = load(&f.root).unwrap().unwrap();
                let publication = journal.publication.as_ref().unwrap();
                assert_eq!(journal.phase, Phase::Validating);
                assert_eq!(publication.purpose, "upgrade");
                assert_eq!(
                    digest_file(&f.root.join(FAMILY[0]), MAX_DATABASE).unwrap(),
                    publication.output_hash
                );
                assert_eq!(
                    Store::open(&f.root).err().unwrap().code,
                    "update_recovery_pending"
                );
                assert_eq!(
                    run(&f, "0.2.0", &swap, &NoFaults).unwrap(),
                    BootDisposition::RestartRequired
                );
                assert_sanitized(&f);
                let store = Store::open(&f.root).unwrap();
                crate::service::Service::validate_upgrade_store(&store).unwrap();
                assert_eq!(swap.calls.load(Ordering::SeqCst), 2);
            } else {
                let db = Connection::open(f.root.join(FAMILY[0])).unwrap();
                db.execute_batch("PRAGMA user_version=2").unwrap();
                drop(db);
                let before = family(&f.root).unwrap();
                let app_before = tree_digest(&f.app).unwrap();
                assert_eq!(
                    run(&f, "0.2.0", &swap, &NoFaults).unwrap_err().code,
                    "update_recovery_conflict"
                );
                assert_eq!(family(&f.root).unwrap(), before);
                assert_eq!(tree_digest(&f.app).unwrap(), app_before);
                assert_eq!(swap.calls.load(Ordering::SeqCst), 1);
            }
        }
    }
    #[test]
    fn private_initialization_rejects_invalid_typed_settings_pending_and_terminal_views() {
        for kind in [
            "settings",
            "authorization-pending",
            "authorization-terminal",
        ] {
            let (f, store) = fixture();
            let update = prepared(&f, &store);
            drop(store);
            let swap = FakeInstaller::new();
            update.swap_with(&swap, &NoFaults).unwrap();
            let invalid_metadata = |layout: &Layout| -> Result<PathBuf> {
                let work = layout.file("validation");
                private_directory(&work)?;
                atomic_copy(
                    &layout.file("before.sqlite3"),
                    &work.join(FAMILY[0]),
                    &digest_file(&layout.file("before.sqlite3"), MAX_DATABASE)?,
                )?;
                let store = Store::open_for_update_validation(&work)?;
                store.put(kind, "local", "local", &json!({"invalid":true}))?;
                assert_eq!(
                    crate::service::Service::validate_upgrade_store(&store)
                        .unwrap_err()
                        .code,
                    "local_metadata_invalid"
                );
                Err(fail("synthetic_initialization_failure"))
            };
            assert_eq!(
                boot_with(
                    &f.root,
                    Some(&f.app),
                    "0.2.0",
                    &swap,
                    &NoFaults,
                    &invalid_metadata
                )
                .unwrap(),
                BootDisposition::RestartRequired
            );
            assert_sanitized(&f);
        }
    }
    #[tokio::test]
    async fn pending_guards_service_helper_paid_vault_and_configuration_paths() {
        let (f, store) = fixture();
        let store = Arc::new(store);
        let vault = Arc::new(NoCallsVault::default());
        let paths = crate::system::SystemPaths::isolated(&f.root).unwrap();
        let service =
            crate::service::Service::new(store.clone(), vault.clone(), paths.clone()).unwrap();
        let paid = Arc::new(AtomicU64::new(0));
        let count = paid.clone();
        service.http.scripted(Arc::new(move |_| {
            let count = count.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                Err(fail("unexpected_http"))
            })
        }));
        let update = prepared(&f, &store);
        assert_eq!(
            Store::open(&f.root).err().unwrap().code,
            "update_recovery_pending"
        );
        assert_eq!(
            Store::open_readonly(&f.root).err().unwrap().code,
            "update_recovery_pending"
        );
        assert_eq!(
            crate::service::Service::new(store.clone(), vault.clone(), paths)
                .err()
                .unwrap()
                .code,
            "update_recovery_pending"
        );
        assert_eq!(
            bound_helper_token(
                &f.binding,
                ClientKind::Codex,
                &f.binding.id,
                &store,
                vault.as_ref(),
                1
            )
            .unwrap_err()
            .code,
            "update_recovery_pending"
        );
        assert_eq!(
            service
                .create_image_task(
                    &f.id,
                    4,
                    "synthetic",
                    "fixture".into(),
                    crate::dto::ImageParameters::default(),
                    vec![]
                )
                .unwrap_err()
                .code,
            "update_recovery_pending"
        );
        assert_eq!(
            service.preview_restore(&uuid()).unwrap_err().code,
            "update_recovery_pending"
        );
        assert_eq!(
            service.list_models(&f.id).await.unwrap_err().code,
            "update_recovery_pending"
        );
        assert_eq!(paid.load(Ordering::SeqCst), 0);
        assert_eq!(vault.0.load(Ordering::SeqCst), 0);
        assert_eq!(tree_digest(&f.app).unwrap(), update.journal.old_tree);
    }
}
