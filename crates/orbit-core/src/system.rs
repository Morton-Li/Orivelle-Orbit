use crate::{
    dto::{ClientKind, ClientView, CodexRuntimeView},
    error::{Result, fail},
    store::private_directory,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct SystemPaths {
    pub codex_config: PathBuf,
    pub claude_library: PathBuf,
    pub helper: PathBuf,
    pub helper_expected_sha256: Option<String>,
    pub configuration_roots: Vec<PathBuf>,
    pub writes_authorized: bool,
    pub isolated: bool,
    pub(crate) codex_user_home: Option<PathBuf>,
    #[cfg(target_os = "macos")]
    claude_configuration_cache: std::sync::Arc<std::sync::Mutex<Option<ClaudeConfigurationTarget>>>,
    #[cfg(test)]
    pub fixture_clients: Option<Vec<ClientView>>,
    #[cfg(test)]
    pub(crate) fixture_policy:
        Option<std::sync::Arc<dyn Fn(&ClientView) -> crate::policy::Snapshot + Send + Sync>>,
    #[cfg(test)]
    pub(crate) fixture_codex_runtime:
        Option<std::sync::Arc<dyn Fn() -> CodexRuntimeSnapshot + Send + Sync>>,
    #[cfg(test)]
    pub(crate) fixture_configuration_target: Option<
        std::sync::Arc<dyn Fn(&ClientView, bool) -> Result<CodexConfigurationTarget> + Send + Sync>,
    >,
    #[cfg(test)]
    pub(crate) fixture_claude_configuration_target: Option<
        std::sync::Arc<
            dyn Fn(&ClientView, bool) -> Result<ClaudeConfigurationTarget> + Send + Sync,
        >,
    >,
}

const CONFIGURATION_DESKTOP_VERSION: &str = "26.1013.40934";
const CONFIGURATION_BUNDLE_ID: &str = "com.openai.codex";
const CONFIGURATION_CLI: &str = "Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex";
const CONFIGURATION_CLI_BYTES: u64 = 249_954_368;
const CONFIGURATION_CLI_SHA: &str =
    "1afb28c01d7d49e892bd75f11076b10f1451e34e0ee308a95e5ab7f69f9c84fe";
const CONFIGURATION_MATERIALS: [(&str, u64, &str); 3] = [
    (
        "Contents/Info.plist",
        20_738,
        "8520b12f2bcf41fe7bb2d7d0fca7043ed495497c5a5405aafea01933f1b53058",
    ),
    (
        "Contents/Resources/codex-cli/codex-package.json",
        211,
        "558c7aabb05ccc87bdb8c8514f87659fec72929a39919c7156f8b8889f7589b3",
    ),
    (
        "Contents/Resources/codex-cli/CodexCLI.app/Contents/Info.plist",
        525,
        "c40b0e6b87b94ef5a232ab8312678c5db2156428f57c3f3d6c32525373be7f59",
    ),
];
const CLAUDE_BUNDLE_ID: &str = "com.anthropic.claudefordesktop";
const CLAUDE_CONFIGURATION_IMPACT: &str = "Claude 第三方推理模式影响整个桌面应用，包括 Chat、Code 与 Cowork；请先核对安装版本与本地策略，未通过核验时自动接入保持禁用。";
const CLAUDE_LEGACY_GUIDANCE: &str =
    "Claude 第三方推理模式影响整个桌面应用；configLibrary envelope 尚未验证，请使用官方导入流程。";
#[cfg(target_os = "macos")]
const CLAUDE_MATERIALS: [(&str, u64, &str); 2] = [
    (
        "Contents/Info.plist",
        12_938,
        "7ff3c89ba08993679d13e2b5d3b3f6de331233901aa069ae83591a5168e42344",
    ),
    (
        "Contents/Resources/app.asar",
        64_685_328,
        "0f8f7d420223874fbcfc8de8e29a978d0a04bb2ae620d4b454dc3714f275035c",
    ),
];
#[cfg(target_os = "macos")]
const MAX_CONFIGURATION_INFO_BYTES: u64 = 1024 * 1024;

#[cfg(target_os = "macos")]
fn bounded_configuration_info(reader: impl Read) -> Option<plist::Value> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_CONFIGURATION_INFO_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_CONFIGURATION_INFO_BYTES {
        return None;
    }
    plist::Value::from_reader(std::io::Cursor::new(bytes)).ok()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ConfigurationRootAnchor {
    device: u64,
    inode: u64,
    owner: u32,
}

/// An installed Claude schema and the intended default local user destination.
/// This receipt says nothing about a running client, its selected configuration,
/// loaded state, connectivity or remote organization policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeConfigurationTarget {
    contract_version: u32,
    pub app_path: String,
    pub bundle_id: String,
    pub app_version: String,
    pub source_member_sha256: String,
    pub config_root: PathBuf,
    root_anchors: Vec<ConfigurationRootAnchor>,
    materials: Vec<String>,
    pub revision: String,
}

impl ClaudeConfigurationTarget {
    pub fn validate_root(&self) -> Result<()> {
        if self.contract_version != 1
            || self.bundle_id != CLAUDE_BUNDLE_ID
            || self.app_version != crate::claude_config::SUPPORTED_APP_VERSION
            || self.source_member_sha256 != crate::claude_config::SOURCE_MEMBER_SHA256
            || !configuration_digest(&self.revision)
        {
            return Err(fail("claude_configuration_target_unverified"));
        }
        if claude_configuration_root(&self.config_root)? != self.root_anchors {
            return Err(fail("claude_configuration_target_changed"));
        }
        Ok(())
    }

    /// Only the confirmed, locked journal may create the two missing leaves.
    /// The caller rechecks its entire operation before each mkdir boundary.
    pub(crate) fn ensure_root(&self, before_creation: &dyn Fn() -> Result<()>) -> Result<()> {
        self.validate_root()?;
        #[cfg(target_os = "macos")]
        {
            use std::os::fd::AsRawFd;
            for (parent, name, path) in [
                (
                    self.config_root
                        .parent()
                        .ok_or_else(|| fail("invalid_path"))?
                        .to_path_buf(),
                    c"Claude-3p",
                    self.config_root.clone(),
                ),
                (
                    self.config_root.clone(),
                    c"configLibrary",
                    self.config_root.join("configLibrary"),
                ),
            ] {
                before_creation()?;
                self.validate_root()?;
                if !path
                    .try_exists()
                    .map_err(|_| fail("configuration_conflict"))?
                {
                    let parent = configuration_directory(&parent).map_err(claude_target_error)?;
                    if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                        return Err(fail("configuration_conflict"));
                    }
                    parent.sync_all().map_err(|_| fail("local_save_failed"))?;
                }
                self.validate_root()?;
            }
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = before_creation;
            Err(fail("claude_configuration_target_unverified"))
        }
    }

    #[cfg(test)]
    pub(crate) fn synthetic(config_root: PathBuf) -> Result<Self> {
        let root_anchors = claude_configuration_root(&config_root)?;
        Ok(Self {
            contract_version: 1,
            app_path: "synthetic-installed-Claude.app".into(),
            bundle_id: CLAUDE_BUNDLE_ID.into(),
            app_version: crate::claude_config::SUPPORTED_APP_VERSION.into(),
            source_member_sha256: crate::claude_config::SOURCE_MEMBER_SHA256.into(),
            config_root,
            root_anchors,
            materials: vec!["synthetic-public-material".into()],
            revision: crate::identity::sha256(b"synthetic-installed-Claude-target"),
        })
    }
}

#[cfg(target_os = "macos")]
fn claude_target_error(error: crate::error::PublicError) -> crate::error::PublicError {
    fail(if error.code == "codex_configuration_target_changed" {
        "claude_configuration_target_changed"
    } else {
        "claude_configuration_target_unverified"
    })
}

fn claude_configuration_root(root: &Path) -> Result<Vec<ConfigurationRootAnchor>> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::MetadataExt;
        let home = root
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .ok_or_else(|| fail("claude_configuration_target_unverified"))?;
        if root != home.join("Library/Application Support/Claude-3p") {
            return Err(fail("claude_configuration_target_unverified"));
        }
        let owner = unsafe { libc::geteuid() };
        let mut anchors = Vec::new();
        for (index, path) in [
            home.to_path_buf(),
            home.join("Library"),
            home.join("Library/Application Support"),
            root.to_path_buf(),
            root.join("configLibrary"),
        ]
        .iter()
        .enumerate()
        {
            match fs::symlink_metadata(path) {
                Err(error) if index >= 3 && error.kind() == std::io::ErrorKind::NotFound => {
                    continue;
                }
                Err(_) => return Err(fail("claude_configuration_target_unverified")),
                Ok(_) => {}
            }
            let file = configuration_directory(path).map_err(claude_target_error)?;
            let before = file
                .metadata()
                .map_err(|_| fail("claude_configuration_target_unverified"))?;
            if before.uid() != owner || before.mode() & 0o022 != 0 {
                return Err(fail("claude_configuration_target_unverified"));
            }
            let after = configuration_directory(path)
                .map_err(claude_target_error)?
                .metadata()
                .map_err(|_| fail("claude_configuration_target_unverified"))?;
            if before.dev() != after.dev()
                || before.ino() != after.ino()
                || before.uid() != after.uid()
            {
                return Err(fail("claude_configuration_target_changed"));
            }
            // The three stable ancestors establish the intended default scope.
            // Creating our two directories or changing their contents must not
            // invalidate this receipt while its journal is publishing.
            if index < 3 {
                anchors.push(ConfigurationRootAnchor {
                    device: before.dev(),
                    inode: before.ino(),
                    owner,
                });
            }
        }
        Ok(anchors)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = root;
        Err(fail("claude_configuration_target_unverified"))
    }
}

#[cfg(any(target_os = "macos", test))]
fn claude_verify_cached(
    cache: &std::sync::Mutex<Option<ClaudeConfigurationTarget>>,
    before: ClaudeConfigurationTarget,
    current: impl FnOnce() -> Result<ClaudeConfigurationTarget>,
    verify_signature: impl FnOnce() -> Result<()>,
    verify_large_hash: impl FnOnce() -> Result<()>,
) -> Result<ClaudeConfigurationTarget> {
    let cached = cache
        .lock()
        .map_err(|_| fail("claude_configuration_target_unverified"))?
        .as_ref()
        == Some(&before);
    if !cached {
        verify_large_hash()?;
    }
    // The stamp cache covers only the large pinned ASAR hash. A signature
    // covers additional executable/resources that are deliberately not part of
    // that cache key, so its result must never be reused.
    verify_signature()?;
    if before != current()? {
        return Err(fail("claude_configuration_target_changed"));
    }
    *cache
        .lock()
        .map_err(|_| fail("claude_configuration_target_unverified"))? = Some(before.clone());
    Ok(before)
}

/// An installed schema and an explicit user configuration destination. This
/// receipt makes no statement about a running process, effective runtime home,
/// loaded configuration, connectivity, or remote organization policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CodexConfigurationTarget {
    contract_version: u32,
    pub(crate) app_path: String,
    pub(crate) bundle_id: String,
    pub(crate) desktop_version: String,
    pub(crate) cli_version: String,
    pub(crate) profile_id: String,
    pub(crate) config_root: PathBuf,
    root_anchor: ConfigurationRootAnchor,
    pub(crate) revision: String,
}
impl CodexConfigurationTarget {
    pub(crate) fn validate_root(&self) -> Result<()> {
        if self.contract_version != 1
            || self.cli_version != crate::codex_catalog::SUPPORTED_CLI_VERSION
            || self.profile_id != crate::codex_binding::PROFILE_ID
            || !configuration_digest(&self.revision)
        {
            return Err(fail("codex_configuration_target_unverified"));
        }
        if configuration_root(&self.config_root)?.1 != self.root_anchor {
            return Err(fail("codex_configuration_target_changed"));
        }
        Ok(())
    }
    /// Called only inside the confirmed journal's existing file lock, after all
    /// before/after checks and publishing persistence. Never used by a preview.
    pub(crate) fn ensure_root(&self) -> Result<()> {
        self.validate_root()?;
        #[cfg(target_os = "macos")]
        {
            use std::os::fd::AsRawFd;
            let (home, anchor) = configuration_root(&self.config_root)?;
            if anchor != self.root_anchor {
                return Err(fail("codex_configuration_target_changed"));
            }
            if !self
                .config_root
                .try_exists()
                .map_err(|_| fail("configuration_conflict"))?
            {
                if unsafe { libc::mkdirat(home.as_raw_fd(), c".codex".as_ptr(), 0o700) } != 0 {
                    return Err(fail("configuration_conflict"));
                }
                home.sync_all().map_err(|_| fail("local_save_failed"))?;
            }
            self.validate_root()
        }
        #[cfg(not(target_os = "macos"))]
        Err(fail("codex_configuration_target_unverified"))
    }
    #[cfg(test)]
    pub(crate) fn synthetic(
        paths: &SystemPaths,
        target: &ClientView,
        marker: &str,
    ) -> Result<Self> {
        let config_root = paths
            .codex_config
            .parent()
            .ok_or_else(|| fail("invalid_path"))?
            .to_path_buf();
        let (_, root_anchor) = configuration_root(&config_root)?;
        Ok(Self {
            contract_version: 1,
            app_path: target
                .app_path
                .clone()
                .unwrap_or_else(|| "synthetic-installed-app".into()),
            bundle_id: target.bundle_id.clone().unwrap_or_default(),
            desktop_version: target.version.clone().unwrap_or_default(),
            cli_version: crate::codex_catalog::SUPPORTED_CLI_VERSION.into(),
            profile_id: crate::codex_binding::PROFILE_ID.into(),
            config_root,
            root_anchor,
            revision: crate::identity::sha256(marker.as_bytes()),
        })
    }
}
fn configuration_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[cfg(target_os = "macos")]
fn configuration_directory(path: &Path) -> Result<fs::File> {
    use std::{
        ffi::CString,
        os::{fd::FromRawFd, unix::ffi::OsStrExt},
    };
    if !path.is_absolute() || path.as_os_str().len() > 4096 {
        return Err(fail("codex_configuration_target_unverified"));
    }
    let fd = unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(fail("codex_configuration_target_unverified"));
    }
    let mut file = unsafe { fs::File::from_raw_fd(fd) };
    use std::os::fd::AsRawFd;
    for component in path.components().skip(1) {
        let std::path::Component::Normal(name) = component else {
            return Err(fail("codex_configuration_target_unverified"));
        };
        let name = CString::new(name.as_bytes())
            .map_err(|_| fail("codex_configuration_target_unverified"))?;
        let fd = unsafe {
            libc::openat(
                file.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(fail("codex_configuration_target_unverified"));
        }
        file = unsafe { fs::File::from_raw_fd(fd) };
    }
    Ok(file)
}
fn configuration_root(root: &Path) -> Result<(fs::File, ConfigurationRootAnchor)> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::MetadataExt;
        if root.file_name().is_none_or(|name| name != ".codex") {
            return Err(fail("codex_configuration_target_unverified"));
        }
        let home = root
            .parent()
            .ok_or_else(|| fail("codex_configuration_target_unverified"))?;
        let file = configuration_directory(home)?;
        let metadata = file
            .metadata()
            .map_err(|_| fail("codex_configuration_target_unverified"))?;
        let owner = unsafe { libc::geteuid() };
        if metadata.uid() != owner || metadata.mode() & 0o022 != 0 {
            return Err(fail("codex_configuration_target_unverified"));
        }
        match fs::symlink_metadata(root) {
            Ok(_) => {
                let directory = configuration_directory(root)?;
                let metadata = directory
                    .metadata()
                    .map_err(|_| fail("codex_configuration_target_unverified"))?;
                if metadata.uid() != owner || metadata.mode() & 0o022 != 0 {
                    return Err(fail("codex_configuration_target_unverified"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(fail("codex_configuration_target_unverified")),
        }
        let current = configuration_directory(home)?
            .metadata()
            .map_err(|_| fail("codex_configuration_target_unverified"))?;
        if metadata.dev() != current.dev()
            || metadata.ino() != current.ino()
            || metadata.uid() != current.uid()
        {
            return Err(fail("codex_configuration_target_changed"));
        }
        Ok((
            file,
            ConfigurationRootAnchor {
                device: metadata.dev(),
                inode: metadata.ino(),
                owner,
            },
        ))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = root;
        Err(fail("codex_configuration_target_unverified"))
    }
}

#[cfg(target_os = "macos")]
fn configuration_file(path: &Path) -> Result<fs::File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
    };
    let parent = configuration_directory(
        path.parent()
            .ok_or_else(|| fail("codex_configuration_target_unverified"))?,
    )?;
    let name = CString::new(
        path.file_name()
            .ok_or_else(|| fail("codex_configuration_target_unverified"))?
            .as_bytes(),
    )
    .map_err(|_| fail("codex_configuration_target_unverified"))?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(fail("codex_configuration_target_unverified"));
    }
    let file = unsafe { fs::File::from_raw_fd(fd) };
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Err(fail("codex_configuration_target_unverified"));
    }
    Ok(file)
}
#[cfg(target_os = "macos")]
fn configuration_stamp(metadata: &fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    format!(
        "{}:{}:{}:{}:{}:{}:{}:{}",
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mode(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec()
    )
}
#[cfg(target_os = "macos")]
fn configuration_hash(path: &Path, bytes: u64) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = configuration_file(path)?;
    let before = file
        .metadata()
        .map_err(|_| fail("codex_configuration_target_unverified"))?;
    if before.len() != bytes {
        return Err(fail("codex_configuration_target_unverified"));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut remaining = bytes;
    while remaining > 0 {
        let length = remaining.min(buffer.len() as u64) as usize;
        let read = file
            .read(&mut buffer[..length])
            .map_err(|_| fail("codex_configuration_target_unverified"))?;
        if read == 0 {
            return Err(fail("codex_configuration_target_unverified"));
        }
        digest.update(&buffer[..read]);
        remaining -= read as u64;
    }
    if file
        .read(&mut buffer[..1])
        .map_err(|_| fail("codex_configuration_target_unverified"))?
        != 0
        || configuration_stamp(&before)
            != configuration_stamp(
                &file
                    .metadata()
                    .map_err(|_| fail("codex_configuration_target_unverified"))?,
            )
        || configuration_stamp(&before)
            != configuration_stamp(
                &configuration_file(path)?
                    .metadata()
                    .map_err(|_| fail("codex_configuration_target_unverified"))?,
            )
    {
        return Err(fail("codex_configuration_target_changed"));
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(target_os = "macos")]
const MAX_CLAUDE_MIGRATION_ENTRIES: usize = crate::claude_config::MAX_ENTRIES * 4;
#[cfg(target_os = "macos")]
const MAX_CLAUDE_MIGRATION_BYTES: usize = crate::claude_config::MAX_FILE_BYTES * 16;

#[cfg(target_os = "macos")]
fn claude_migration_profile_name(name: &[u8]) -> bool {
    // Match Desktop's /^[a-f0-9-]{36}$/ ID, not the compiler's narrower UUID
    // contract: orphan and noncanonical entries are swept by Desktop too.
    name.len() == 41
        && name.ends_with(b".json")
        && name[..36]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c) || *c == b'-')
}

#[cfg(target_os = "macos")]
fn claude_migration_open(
    parent: &fs::File,
    name: &std::ffi::CStr,
    directory: bool,
) -> std::io::Result<fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | libc::O_CLOEXEC
                | if directory { libc::O_DIRECTORY } else { 0 },
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { fs::File::from_raw_fd(fd) })
    }
}

#[cfg(target_os = "macos")]
fn claude_migration_same(before: &fs::Metadata, after: &fs::Metadata) -> Result<()> {
    if configuration_stamp(before) != configuration_stamp(after) {
        return Err(fail("claude_configuration_target_changed"));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn claude_migration_file(
    directory: &fs::File,
    name: &std::ffi::CStr,
    main_config: bool,
    total: &mut usize,
) -> Result<()> {
    let unverified = || fail("claude_configuration_target_unverified");
    let mut file = match claude_migration_open(directory, name, false) {
        Ok(file) => file,
        Err(error) if main_config && error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(unverified()),
    };
    let before = file.metadata().map_err(|_| unverified())?;
    let remaining = MAX_CLAUDE_MIGRATION_BYTES.saturating_sub(*total);
    let limit = crate::claude_config::MAX_FILE_BYTES.min(remaining);
    if !before.is_file() || before.len() > limit as u64 {
        return Err(unverified());
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(limit + 1));
    Read::by_ref(&mut file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unverified())?;
    if bytes.len() > limit {
        return Err(unverified());
    }
    claude_migration_same(&before, &file.metadata().map_err(|_| unverified())?)?;
    let named = claude_migration_open(directory, name, false).map_err(|_| unverified())?;
    claude_migration_same(&before, &named.metadata().map_err(|_| unverified())?)?;
    *total += bytes.len();
    if crate::claude_config::native_migration_required(&bytes, main_config)? {
        return Err(fail("claude_native_migration_required"));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn claude_native_migration_clear(root: &Path) -> Result<()> {
    use std::{
        ffi::{CStr, CString},
        os::fd::AsRawFd,
    };
    let unverified = || fail("claude_configuration_target_unverified");
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(unverified()),
        Ok(_) => {}
    }
    let directory = configuration_directory(root).map_err(claude_target_error)?;
    let root_before = directory.metadata().map_err(|_| unverified())?;
    let mut total = 0;
    claude_migration_file(&directory, c"claude_desktop_config.json", true, &mut total)?;
    match claude_migration_open(&directory, c"configLibrary", true) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(unverified()),
        Ok(library) => {
            let before = library.metadata().map_err(|_| unverified())?;
            let fd = unsafe { libc::fcntl(library.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
            if fd < 0 {
                return Err(unverified());
            }
            let raw = unsafe { libc::fdopendir(fd) };
            if raw.is_null() {
                unsafe { libc::close(fd) };
                return Err(unverified());
            }
            struct Entries(*mut libc::DIR);
            impl Drop for Entries {
                fn drop(&mut self) {
                    unsafe { libc::closedir(self.0) };
                }
            }
            let entries = Entries(raw);
            let mut count = 0;
            let mut profiles = Vec::new();
            loop {
                unsafe { *libc::__error() = 0 };
                let entry = unsafe { libc::readdir(entries.0) };
                if entry.is_null() {
                    if std::io::Error::last_os_error().raw_os_error() != Some(0) {
                        return Err(unverified());
                    }
                    break;
                }
                let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
                if name.to_bytes() == b"." || name.to_bytes() == b".." {
                    continue;
                }
                count += 1;
                if count > MAX_CLAUDE_MIGRATION_ENTRIES {
                    return Err(unverified());
                }
                if claude_migration_profile_name(name.to_bytes()) {
                    if profiles.len() >= crate::claude_config::MAX_ENTRIES {
                        return Err(unverified());
                    }
                    profiles.push(CString::from(name));
                }
            }
            drop(entries);
            for name in profiles {
                claude_migration_file(&library, &name, false, &mut total)?;
            }
            claude_migration_same(&before, &library.metadata().map_err(|_| unverified())?)?;
            let named = claude_migration_open(&directory, c"configLibrary", true)
                .map_err(|_| unverified())?;
            claude_migration_same(&before, &named.metadata().map_err(|_| unverified())?)?;
        }
    }
    claude_migration_same(
        &root_before,
        &directory.metadata().map_err(|_| unverified())?,
    )?;
    let named = configuration_directory(root).map_err(claude_target_error)?;
    claude_migration_same(&root_before, &named.metadata().map_err(|_| unverified())?)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CodexRuntimeIdentity {
    pub(crate) cli_version: String,
    pub(crate) revision: String,
}

pub(crate) struct CodexRuntimeSnapshot {
    identity: Option<CodexRuntimeIdentity>,
}
impl CodexRuntimeSnapshot {
    pub(crate) fn unverified() -> Self {
        Self { identity: None }
    }
    pub(crate) fn require_verified(&self) -> Result<CodexRuntimeIdentity> {
        self.identity
            .clone()
            .filter(|identity| {
                identity.cli_version == crate::codex_catalog::SUPPORTED_CLI_VERSION
                    && identity.revision.len() == 64
                    && identity
                        .revision
                        .bytes()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            })
            .ok_or_else(|| fail("codex_runtime_unverified"))
    }
    pub(crate) fn view(&self) -> CodexRuntimeView {
        let verified = self.require_verified().ok();
        CodexRuntimeView {
            state: if verified.is_some() {
                "verified"
            } else {
                "unverified"
            }
            .into(),
            cli_version: verified
                .as_ref()
                .map(|identity| identity.cli_version.clone()),
            revision: verified.map(|identity| identity.revision),
        }
    }
}
impl SystemPaths {
    /// Metadata-only installation freshness; configuration state and existing
    /// saved/conflict evidence stay with the caller's cached view.
    pub fn fresh_claude_client(&self, cached: &ClientView) -> ClientView {
        let mut target = cached.clone();
        // Retire only the obsolete detection note; keep saved/recovery and
        // policy instructions while refreshing installed metadata.
        target
            .manual_steps
            .retain(|note| note != CLAUDE_LEGACY_GUIDANCE);
        if !target
            .manual_steps
            .iter()
            .any(|note| note == CLAUDE_CONFIGURATION_IMPACT)
        {
            target.manual_steps.push(CLAUDE_CONFIGURATION_IMPACT.into());
        }
        if self.isolated {
            return target;
        }
        target.installed = false;
        target.version = None;
        target.bundle_id = None;
        target.app_path = None;
        #[cfg(target_os = "macos")]
        {
            let mut candidates = cached
                .app_path
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            candidates.push(PathBuf::from("/Applications/Claude.app"));
            if let Some(home) = &self.codex_user_home {
                candidates.push(home.join("Applications/Claude.app"));
            }
            for app in candidates {
                let Ok(file) = configuration_file(&app.join("Contents/Info.plist")) else {
                    continue;
                };
                let Some(value) = bounded_configuration_info(file) else {
                    continue;
                };
                let Some(dict) = value.as_dictionary() else {
                    continue;
                };
                if dict
                    .get("CFBundleIdentifier")
                    .and_then(plist::Value::as_string)
                    != Some(CLAUDE_BUNDLE_ID)
                {
                    continue;
                }
                target.installed = true;
                target.bundle_id = Some(CLAUDE_BUNDLE_ID.into());
                target.version = dict
                    .get("CFBundleShortVersionString")
                    .and_then(plist::Value::as_string)
                    .map(str::to_owned);
                target.app_path = Some(app.display().to_string());
                break;
            }
        }
        target
    }

    /// Lightweight qualification projection, without a signature process or a
    /// full ASAR hash. It is not a receipt and does not enable any writer.
    pub fn claude_configuration_hint(&self, target: &ClientView) -> Result<String> {
        #[cfg(test)]
        if let Some(adapter) = &self.fixture_claude_configuration_target {
            return Ok(adapter(target, false)?.revision);
        }
        Ok(self.claude_configuration_draft(target)?.revision)
    }

    pub fn claude_configuration_target(
        &self,
        target: &ClientView,
    ) -> Result<ClaudeConfigurationTarget> {
        #[cfg(test)]
        if let Some(adapter) = &self.fixture_claude_configuration_target {
            return adapter(target, true);
        }
        let before = self.claude_configuration_draft(target)?;
        #[cfg(target_os = "macos")]
        {
            claude_native_migration_clear(&before.config_root)?;
            claude_verify_cached(
                &self.claude_configuration_cache,
                before,
                || {
                    let current = self.claude_configuration_draft(target)?;
                    claude_native_migration_clear(&current.config_root)?;
                    Ok(current)
                },
                || {
                    let app = Path::new(
                        target
                            .app_path
                            .as_deref()
                            .ok_or_else(|| fail("claude_configuration_target_unverified"))?,
                    );
                    let mut command = Command::new("/usr/bin/codesign");
                    command.args([
                        "--verify", "--strict", "--deep", "-R",
                        "=identifier \"com.anthropic.claudefordesktop\" and anchor apple generic and certificate leaf[subject.OU] = \"Q6L2SF6YDW\"",
                    ]).arg(app);
                    claude_signature_verification(command)
                },
                || {
                    let app = Path::new(
                        target
                            .app_path
                            .as_deref()
                            .ok_or_else(|| fail("claude_configuration_target_unverified"))?,
                    );
                    let (relative, bytes, digest) = CLAUDE_MATERIALS[1];
                    if configuration_hash(&app.join(relative), bytes)
                        .map_err(claude_target_error)?
                        != digest
                    {
                        return Err(fail("claude_configuration_target_unverified"));
                    }
                    Ok(())
                },
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = before;
            Err(fail("claude_configuration_target_unverified"))
        }
    }

    fn claude_configuration_draft(&self, target: &ClientView) -> Result<ClaudeConfigurationTarget> {
        #[cfg(target_os = "macos")]
        {
            if self.isolated
                || !target.installed
                || !target.platform_supported
                || target.client != ClientKind::Claude
                || target.version.as_deref() != Some(crate::claude_config::SUPPORTED_APP_VERSION)
                || target.bundle_id.as_deref() != Some(CLAUDE_BUNDLE_ID)
                || std::env::var_os("CLAUDE_USER_DATA_DIR").is_some()
                || std::env::var_os("CLAUDE_E2E_MANAGED_PLIST").is_some()
            {
                return Err(fail("claude_configuration_target_unverified"));
            }
            let app_path = target
                .app_path
                .clone()
                .ok_or_else(|| fail("claude_configuration_target_unverified"))?;
            let home = self
                .codex_user_home
                .as_ref()
                .ok_or_else(|| fail("claude_configuration_target_unverified"))?;
            let root = home.join("Library/Application Support/Claude-3p");
            if self.claude_library != root.join("configLibrary")
                || !self.configuration_roots.contains(&root)
            {
                return Err(fail("claude_configuration_target_unverified"));
            }
            let root_anchors = claude_configuration_root(&root)?;
            let mut materials = Vec::new();
            for (index, (relative, bytes, digest)) in CLAUDE_MATERIALS.iter().enumerate() {
                let path = Path::new(&app_path).join(relative);
                let file = configuration_file(&path).map_err(claude_target_error)?;
                let before = file
                    .metadata()
                    .map_err(|_| fail("claude_configuration_target_unverified"))?;
                if before.len() != *bytes {
                    return Err(fail("claude_configuration_target_unverified"));
                }
                // Info is small and pins version/bundle identity on each hint.
                // ASAR is streamed only for a new full receipt; its complete
                // metadata stamp participates in the receipt/cache revision.
                if index == 0
                    && configuration_hash(&path, *bytes).map_err(claude_target_error)? != *digest
                {
                    return Err(fail("claude_configuration_target_unverified"));
                }
                let after = configuration_file(&path)
                    .map_err(claude_target_error)?
                    .metadata()
                    .map_err(|_| fail("claude_configuration_target_unverified"))?;
                if configuration_stamp(&before) != configuration_stamp(&after) {
                    return Err(fail("claude_configuration_target_changed"));
                }
                materials.push(configuration_stamp(&after));
            }
            let revision = crate::identity::sha256(
                &serde_json::to_vec(&(
                    "claude-installed-user-config-v1",
                    &app_path,
                    CLAUDE_BUNDLE_ID,
                    crate::claude_config::SUPPORTED_APP_VERSION,
                    crate::claude_config::SOURCE_MEMBER_SHA256,
                    crate::claude_config::SCHEMA_KEYS_SHA256,
                    &root,
                    &root_anchors,
                    &materials,
                ))
                .map_err(|_| fail("serialization_failed"))?,
            );
            Ok(ClaudeConfigurationTarget {
                contract_version: 1,
                app_path,
                bundle_id: CLAUDE_BUNDLE_ID.into(),
                app_version: crate::claude_config::SUPPORTED_APP_VERSION.into(),
                source_member_sha256: crate::claude_config::SOURCE_MEMBER_SHA256.into(),
                config_root: root,
                root_anchors,
                materials,
                revision,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = target;
            Err(fail("claude_configuration_target_unverified"))
        }
    }

    pub fn codex_catalog(&self) -> PathBuf {
        self.codex_config
            .with_file_name("orivelle-orbit.models.json")
    }
    pub(crate) fn codex_runtime(&self) -> CodexRuntimeSnapshot {
        #[cfg(test)]
        if let Some(adapter) = &self.fixture_codex_runtime {
            return adapter();
        }
        // An installed package/version is not evidence of the executable or
        // daemon selected by Desktop. No production verified producer exists.
        CodexRuntimeSnapshot::unverified()
    }
    /// Metadata-only freshness for the installed configuration target. This is
    /// a UI hint, never a signed receipt and never a running-runtime assertion.
    pub(crate) fn fresh_codex_client(&self, cached: &ClientView) -> ClientView {
        if self.isolated {
            return cached.clone();
        }
        let mut target = cached.clone();
        target.installed = false;
        target.version = None;
        target.bundle_id = None;
        target.app_path = None;
        #[cfg(target_os = "macos")]
        {
            let mut candidates = cached
                .app_path
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            for directory in [
                Some(PathBuf::from("/Applications")),
                self.codex_user_home
                    .as_ref()
                    .map(|home| home.join("Applications")),
            ]
            .into_iter()
            .flatten()
            {
                for name in ["Codex.app", "ChatGPT.app"] {
                    candidates.push(directory.join(name));
                }
            }
            for app in candidates {
                let Ok(file) = configuration_file(&app.join("Contents/Info.plist")) else {
                    continue;
                };
                if !file
                    .metadata()
                    .is_ok_and(|m| m.len() <= MAX_CONFIGURATION_INFO_BYTES)
                {
                    continue;
                }
                let Some(value) = bounded_configuration_info(file) else {
                    continue;
                };
                let Some(dict) = value.as_dictionary() else {
                    continue;
                };
                if dict
                    .get("CFBundleIdentifier")
                    .and_then(plist::Value::as_string)
                    != Some(CONFIGURATION_BUNDLE_ID)
                {
                    continue;
                }
                target.installed = true;
                target.bundle_id = Some(CONFIGURATION_BUNDLE_ID.into());
                target.version = dict
                    .get("CFBundleShortVersionString")
                    .and_then(plist::Value::as_string)
                    .map(str::to_owned);
                target.app_path = Some(app.display().to_string());
                break;
            }
        }
        target
    }
    pub(crate) fn codex_configuration_hint(&self, target: &ClientView) -> Result<String> {
        #[cfg(test)]
        if let Some(adapter) = &self.fixture_configuration_target {
            return Ok(adapter(target, false)?.revision);
        }
        Ok(self.configuration_draft(target)?.0.revision)
    }
    pub(crate) fn codex_configuration_target(
        &self,
        target: &ClientView,
    ) -> Result<CodexConfigurationTarget> {
        #[cfg(test)]
        if let Some(adapter) = &self.fixture_configuration_target {
            return adapter(target, true);
        }
        let (receipt, before) = self.configuration_draft(target)?;
        #[cfg(target_os = "macos")]
        {
            let app = Path::new(&receipt.app_path);
            let requirement = "=identifier \"com.openai.codex\" and anchor apple generic and certificate leaf[subject.OU] = \"2DC432GLL2\"";
            let mut command = Command::new("/usr/bin/codesign");
            command
                .args(["--verify", "--strict", "--deep", "-R", requirement])
                .arg(app);
            if bounded_command(command).is_none() {
                return Err(fail("codex_configuration_target_unverified"));
            }
            let mut command = Command::new("/usr/bin/codesign");
            command.args(["--verify", "--strict", "-R", "=identifier \"codex\" and anchor apple generic and certificate leaf[subject.OU] = \"2DC432GLL2\""]).arg(app.join(CONFIGURATION_CLI));
            if bounded_command(command).is_none()
                || configuration_hash(&app.join(CONFIGURATION_CLI), CONFIGURATION_CLI_BYTES)?
                    != CONFIGURATION_CLI_SHA
            {
                return Err(fail("codex_configuration_target_unverified"));
            }
        }
        let (after, materials) = self.configuration_draft(target)?;
        if receipt != after || before != materials {
            return Err(fail("codex_configuration_target_changed"));
        }
        Ok(receipt)
    }
    fn configuration_draft(
        &self,
        target: &ClientView,
    ) -> Result<(CodexConfigurationTarget, Vec<String>)> {
        #[cfg(target_os = "macos")]
        {
            if self.isolated
                || !target.installed
                || !target.platform_supported
                || target.client != ClientKind::Codex
                || target.version.as_deref() != Some(CONFIGURATION_DESKTOP_VERSION)
                || target.bundle_id.as_deref() != Some(CONFIGURATION_BUNDLE_ID)
            {
                return Err(fail("codex_configuration_target_unverified"));
            }
            let app_path = target
                .app_path
                .clone()
                .ok_or_else(|| fail("codex_configuration_target_unverified"))?;
            let app = Path::new(&app_path);
            let home = self
                .codex_user_home
                .as_ref()
                .ok_or_else(|| fail("codex_configuration_target_unverified"))?;
            let root = home.join(".codex");
            if self.codex_config != root.join("config.toml")
                || !self.configuration_roots.contains(&root)
            {
                return Err(fail("codex_configuration_target_unverified"));
            }
            let (_, root_anchor) = configuration_root(&root)?;
            let mut materials = Vec::new();
            for (relative, bytes, digest) in CONFIGURATION_MATERIALS {
                let path = app.join(relative);
                let before = configuration_stamp(
                    &configuration_file(&path)?
                        .metadata()
                        .map_err(|_| fail("codex_configuration_target_unverified"))?,
                );
                if configuration_hash(&path, bytes)? != digest {
                    return Err(fail("codex_configuration_target_unverified"));
                }
                let after = configuration_stamp(
                    &configuration_file(&path)?
                        .metadata()
                        .map_err(|_| fail("codex_configuration_target_unverified"))?,
                );
                if before != after {
                    return Err(fail("codex_configuration_target_changed"));
                }
                materials.push(after);
            }
            let cli = configuration_file(&app.join(CONFIGURATION_CLI))?;
            let metadata = cli
                .metadata()
                .map_err(|_| fail("codex_configuration_target_unverified"))?;
            if metadata.len() != CONFIGURATION_CLI_BYTES {
                return Err(fail("codex_configuration_target_unverified"));
            }
            materials.push(configuration_stamp(&metadata));
            let revision = crate::identity::sha256(
                &serde_json::to_vec(&(
                    1u32,
                    &app_path,
                    CONFIGURATION_BUNDLE_ID,
                    CONFIGURATION_DESKTOP_VERSION,
                    crate::codex_catalog::SUPPORTED_CLI_VERSION,
                    crate::codex_binding::PROFILE_ID,
                    crate::codex_catalog::SOURCE_SHA256,
                    &root,
                    &root_anchor,
                    &materials,
                ))
                .map_err(|_| fail("serialization_failed"))?,
            );
            Ok((
                CodexConfigurationTarget {
                    contract_version: 1,
                    app_path,
                    bundle_id: CONFIGURATION_BUNDLE_ID.into(),
                    desktop_version: CONFIGURATION_DESKTOP_VERSION.into(),
                    cli_version: crate::codex_catalog::SUPPORTED_CLI_VERSION.into(),
                    profile_id: crate::codex_binding::PROFILE_ID.into(),
                    config_root: root,
                    root_anchor,
                    revision,
                },
                materials,
            ))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = target;
            Err(fail("codex_configuration_target_unverified"))
        }
    }
    pub fn isolated(root: &Path) -> Result<Self> {
        let client = root.join("client-fixtures");
        private_directory(&client)?;
        let codex = client.join("codex");
        let claude = client.join("claude");
        private_directory(&codex)?;
        private_directory(&claude)?;
        Ok(Self {
            codex_config: codex.join("config.toml"),
            claude_library: claude.join("configLibrary"),
            helper: root.join(if cfg!(windows) {
                "orbit-credential-helper.exe"
            } else {
                "orbit-credential-helper"
            }),
            helper_expected_sha256: None,
            configuration_roots: vec![client],
            writes_authorized: false,
            isolated: true,
            codex_user_home: None,
            #[cfg(target_os = "macos")]
            claude_configuration_cache: Default::default(),
            #[cfg(test)]
            fixture_clients: None,
            #[cfg(test)]
            fixture_policy: None,
            #[cfg(test)]
            fixture_codex_runtime: None,
            #[cfg(test)]
            fixture_configuration_target: None,
            #[cfg(test)]
            fixture_claude_configuration_target: None,
        })
    }
    /// Called only by a build with the explicitly authorized system-integration feature.
    pub fn native(data_root: &Path, home: &Path) -> Result<Self> {
        #[cfg(target_os = "macos")]
        let claude = home.join("Library/Application Support/Claude-3p");
        #[cfg(target_os = "windows")]
        let claude = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| fail("platform_unsupported"))?
            .join("Claude-3p");
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let claude = home.join("unsupported");
        let codex = home.join(".codex");
        Ok(Self {
            codex_config: codex.join("config.toml"),
            claude_library: claude.join("configLibrary"),
            helper: data_root.join("bin").join(if cfg!(windows) {
                "orbit-credential-helper.exe"
            } else {
                "orbit-credential-helper"
            }),
            helper_expected_sha256: None,
            configuration_roots: vec![codex, claude],
            writes_authorized: true,
            isolated: false,
            codex_user_home: Some(home.into()),
            #[cfg(target_os = "macos")]
            claude_configuration_cache: Default::default(),
            #[cfg(test)]
            fixture_clients: None,
            #[cfg(test)]
            fixture_policy: None,
            #[cfg(test)]
            fixture_codex_runtime: None,
            #[cfg(test)]
            fixture_configuration_target: None,
            #[cfg(test)]
            fixture_claude_configuration_target: None,
        })
    }
    pub fn ensure_configuration_parent(&self, client: ClientKind) -> Result<()> {
        if !self.writes_authorized {
            return Err(fail("integration_authorization_required"));
        }
        let path = match client {
            ClientKind::Codex => self
                .codex_config
                .parent()
                .ok_or_else(|| fail("invalid_path"))?,
            ClientKind::Claude => return Err(fail("schema_unverified")),
        };
        let metadata =
            fs::symlink_metadata(path).map_err(|_| fail("configuration_location_missing"))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(fail("configuration_location_missing"));
        }
        Ok(())
    }
    pub fn helper_available(&self) -> bool {
        self.helper_expected_sha256
            .as_ref()
            .is_some_and(|expected| {
                crate::images::bounded_read(&self.helper, crate::helper_artifact::MAX_HELPER_BYTES)
                    .is_ok_and(|bytes| crate::identity::sha256(&bytes) == *expected)
            })
    }
}
fn base_view(client: ClientKind, paths: &SystemPaths) -> ClientView {
    ClientView {
        client,
        installed: false,
        display_name: match client {
            ClientKind::Codex => "Codex",
            ClientKind::Claude => "Claude",
        }
        .into(),
        version: None,
        bundle_id: None,
        app_path: None,
        config_paths: match client {
            ClientKind::Codex => vec![
                paths.codex_catalog().display().to_string(),
                paths.codex_config.display().to_string(),
            ],
            ClientKind::Claude => vec![paths.claude_library.display().to_string()],
        },
        platform_supported: cfg!(any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "windows", target_arch = "x86_64")
        )),
        managed: "unknown".into(),
        configuration_access: crate::dto::ConfigurationAccess::Unverified,
        configuration_access_revision: None,
        configuration_state: if client == ClientKind::Claude {
            "unsupported"
        } else {
            "not_configured"
        }
        .into(),
        loaded_state: "unconfirmed".into(),
        connection_state: "not_tested".into(),
        configuration_revision: None,
        loaded_evidence: None,
        connection_evidence: None,
        manual_steps: if client == ClientKind::Claude {
            vec![CLAUDE_CONFIGURATION_IMPACT.into()]
        } else {
            vec!["Codex、CLI 与 IDE 可能共用用户 config.toml，保存后需重启目标应用。".into()]
        },
    }
}
#[cfg(target_os = "macos")]
fn claude_signature_verification(mut command: Command) -> Result<()> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| fail("claude_configuration_target_unverified"))?;
    claude_signature_wait(
        &mut child,
        #[cfg(test)]
        None,
    )
}

#[cfg(target_os = "macos")]
fn claude_signature_wait(
    child: &mut std::process::Child,
    #[cfg(test)] timeout_override: Option<Duration>,
) -> Result<()> {
    let timeout = Duration::from_secs(30);
    // Production has no configurable budget; isolated tests shorten it only.
    #[cfg(test)]
    let timeout = timeout_override.unwrap_or(timeout);
    let until = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(fail("claude_configuration_target_unverified"))
                };
            }
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(fail("claude_configuration_target_unverified"));
            }
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            return Err(fail("claude_signature_verification_timeout"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn bounded_command(mut command: Command) -> Option<Vec<u8>> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().ok()? {
            if !status.success() {
                return None;
            }
            let mut bytes = Vec::new();
            child
                .stdout
                .take()?
                .take(64 * 1024)
                .read_to_end(&mut bytes)
                .ok()?;
            return Some(bytes);
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
pub fn detect_clients(paths: &SystemPaths) -> Vec<ClientView> {
    #[cfg(test)]
    if paths.isolated {
        if let Some(clients) = &paths.fixture_clients {
            return clients.clone();
        }
    }
    let mut clients = vec![
        base_view(ClientKind::Codex, paths),
        base_view(ClientKind::Claude, paths),
    ];
    if paths.isolated {
        for c in &mut clients {
            c.manual_steps
                .push("当前隔离构建不读取或写入真实客户端配置及安全存储。".into());
        }
        return clients;
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut directories = vec![PathBuf::from("/Applications")];
        if let Some(home) = home {
            directories.push(home.join("Applications"));
        }
        for client in &mut clients {
            let (names, expected): (&[&str], &str) = match client.client {
                ClientKind::Codex => (&["Codex.app", "ChatGPT.app"], "com.openai.codex"),
                ClientKind::Claude => (&["Claude.app"], "com.anthropic.claudefordesktop"),
            };
            for directory in &directories {
                for name in names {
                    let app = directory.join(name);
                    let info = app.join("Contents/Info.plist");
                    let Ok(meta) = fs::symlink_metadata(&info) else {
                        continue;
                    };
                    if !meta.is_file() || meta.len() > 1024 * 1024 {
                        continue;
                    }
                    let Ok(value) = plist::Value::from_file(&info) else {
                        continue;
                    };
                    let Some(dict) = value.as_dictionary() else {
                        continue;
                    };
                    if dict
                        .get("CFBundleIdentifier")
                        .and_then(plist::Value::as_string)
                        != Some(expected)
                    {
                        continue;
                    }
                    // Candidate pins come from the reviewed installed formal app signatures.
                    // Apple anchoring prevents a self-signed bundle from copying these IDs.
                    let team = match client.client {
                        ClientKind::Codex => "2DC432GLL2",
                        ClientKind::Claude => "Q6L2SF6YDW",
                    };
                    let requirement = format!(
                        "=identifier \"{expected}\" and anchor apple generic and certificate leaf[subject.OU] = \"{team}\""
                    );
                    let mut verify = Command::new("/usr/bin/codesign");
                    verify
                        .args(["--verify", "--strict", "--deep", "-R", &requirement])
                        .arg(&app);
                    if bounded_command(verify).is_none() {
                        continue;
                    }
                    client.installed = true;
                    client.bundle_id = Some(expected.into());
                    client.app_path = Some(app.display().to_string());
                    client.version = dict
                        .get("CFBundleShortVersionString")
                        .and_then(plist::Value::as_string)
                        .map(str::to_owned);
                    break;
                }
                if client.installed {
                    break;
                }
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        // Fixed read-only metadata probes; no caller-provided command or executable path.
        for client in &mut clients {
            let script = match client.client {
                ClientKind::Codex => {
                    "$p=Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction SilentlyContinue | Select-Object -First 1; if($p){[pscustomobject]@{identity=$p.Name;version=$p.Version;path=$p.InstallLocation}|ConvertTo-Json -Compress}"
                }
                ClientKind::Claude => {
                    "$p=Join-Path $env:LOCALAPPDATA 'AnthropicClaude\\Claude.exe'; if(Test-Path -LiteralPath $p){$s=Get-AuthenticodeSignature -LiteralPath $p; if($s.Status -eq 'Valid' -and $s.SignerCertificate.Subject -match 'Anthropic'){[pscustomobject]@{identity='com.anthropic.claudefordesktop';version=(Get-Item -LiteralPath $p).VersionInfo.ProductVersion;path=$p}|ConvertTo-Json -Compress}}"
                }
            };
            let mut command = Command::new("powershell.exe");
            command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
            if let Some(bytes) = bounded_command(command) {
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                    if let (Some(identity), Some(version), Some(path)) = (
                        value["identity"].as_str(),
                        value["version"].as_str(),
                        value["path"].as_str(),
                    ) {
                        client.installed = true;
                        client.bundle_id = Some(identity.into());
                        client.version = Some(version.into());
                        client.app_path = Some(path.into());
                    }
                }
            }
        }
    }
    for client in &mut clients {
        crate::policy::observe(paths, client).project(client);
    }
    clients
}
#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn claude_native_migration_missing_and_sealed_profiles_are_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Claude-3p");
        claude_native_migration_clear(&root).unwrap();
        assert!(!root.exists());
        fs::create_dir(&root).unwrap();
        claude_native_migration_clear(&root).unwrap();
        let library = root.join("configLibrary");
        fs::create_dir(&library).unwrap();
        let main = root.join("claude_desktop_config.json");
        fs::write(&main, b"{\"deploymentMode\":\"3p\"}").unwrap();
        for (index, bytes) in [
            br#"{"inferenceGatewayApiKey":""}"#.as_slice(),
            br#"{"inferenceGatewayApiKey":"__sealed__:v1:synthetic"}"#,
            br#"{"inferenceGatewayApiKey":"\uFEFF __sealed__:v1:synthetic"}"#,
            br#"{"inferenceCredentialHelper":"/synthetic/helper"}"#,
        ]
        .iter()
        .enumerate()
        {
            fs::write(library.join(format!("{index:036x}.json")), bytes).unwrap();
        }
        fs::write(
            library.join("_meta.json"),
            b"not inspected by the migration sweep",
        )
        .unwrap();
        let before = fs::read(&main).unwrap();
        claude_native_migration_clear(&root).unwrap();
        assert_eq!(fs::read(&main).unwrap(), before);
        assert_eq!(fs::read_dir(&library).unwrap().count(), 5);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn claude_native_migration_scans_orphan_and_noncanonical_official_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Claude-3p");
        let library = root.join("configLibrary");
        fs::create_dir_all(&library).unwrap();
        for name in [
            "00000000-0000-4000-8000-000000000001.json".to_owned(),
            format!("{}.json", "-".repeat(36)),
            format!("{}.json", "a".repeat(36)),
        ] {
            assert!(claude_migration_profile_name(name.as_bytes()));
            let path = library.join(name);
            fs::write(&path, br#"{"inferenceGatewayApiKey":"synthetic"}"#).unwrap();
            let before = fs::read(&path).unwrap();
            assert_eq!(
                claude_native_migration_clear(&root).unwrap_err(),
                fail("claude_native_migration_required")
            );
            assert_eq!(fs::read(&path).unwrap(), before);
            fs::remove_file(path).unwrap();
        }
        for name in [
            "A".repeat(36) + ".json",
            "g".repeat(36) + ".json",
            "a".repeat(35) + ".json",
            "a".repeat(36) + ".json.tmp",
            "_meta.json".into(),
        ] {
            assert!(!claude_migration_profile_name(name.as_bytes()));
            fs::write(library.join(name), b"not an official profile").unwrap();
        }
        claude_native_migration_clear(&root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn claude_native_migration_main_enterprise_and_invalid_json_block() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Claude-3p");
        fs::create_dir(&root).unwrap();
        let path = root.join("claude_desktop_config.json");
        for bytes in [
            br#"{"enterpriseConfig":{}}"#.as_slice(),
            br#"{"enterpriseConfig":null}"#,
        ] {
            fs::write(&path, bytes).unwrap();
            assert_eq!(
                claude_native_migration_clear(&root).unwrap_err(),
                fail("claude_native_migration_required")
            );
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::write(&path, b"{").unwrap();
        assert_eq!(
            claude_native_migration_clear(&root).unwrap_err(),
            fail("claude_configuration_target_unverified")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn claude_native_migration_rejects_links_nonfiles_and_identity_changes() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Claude-3p");
        let library = root.join("configLibrary");
        fs::create_dir_all(&library).unwrap();
        let external = dir.path().join("external.json");
        fs::write(&external, b"{}").unwrap();
        let main = root.join("claude_desktop_config.json");
        symlink(&external, &main).unwrap();
        assert!(claude_native_migration_clear(&root).is_err());
        fs::remove_file(&main).unwrap();
        let path = library.join("00000000-0000-4000-8000-000000000001.json");
        symlink(&external, &path).unwrap();
        assert!(claude_native_migration_clear(&root).is_err());
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(claude_native_migration_clear(&root).is_err());
        fs::remove_dir(&path).unwrap();
        let original = library.metadata().unwrap();
        fs::write(&path, b"{}").unwrap();
        assert_eq!(
            claude_migration_same(&original, &library.metadata().unwrap()).unwrap_err(),
            fail("claude_configuration_target_changed")
        );
        fs::write(&path, br#"{"a":1}"#).unwrap();
        let original = path.metadata().unwrap();
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"{}").unwrap();
        assert!(claude_migration_same(&original, &path.metadata().unwrap()).is_err());
        assert_eq!(fs::read(&external).unwrap(), b"{}");
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&library).unwrap();
        symlink(dir.path(), &library).unwrap();
        assert!(claude_native_migration_clear(&root).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn claude_native_migration_file_count_size_and_total_bounds_refuse_incomplete_scans() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Claude-3p");
        let library = root.join("configLibrary");
        fs::create_dir_all(&library).unwrap();
        let path = library.join("00000000-0000-4000-8000-000000000001.json");
        fs::write(&path, vec![b' '; crate::claude_config::MAX_FILE_BYTES + 1]).unwrap();
        assert!(claude_native_migration_clear(&root).is_err());
        fs::remove_file(&path).unwrap();
        for index in 0..=crate::claude_config::MAX_ENTRIES {
            fs::write(library.join(format!("{index:036x}.json")), b"{}").unwrap();
        }
        assert!(claude_native_migration_clear(&root).is_err());
        fs::remove_dir_all(&library).unwrap();
        fs::create_dir(&library).unwrap();
        for index in 0..=MAX_CLAUDE_MIGRATION_ENTRIES {
            fs::write(library.join(format!("ignored-{index}")), b"").unwrap();
        }
        assert!(claude_native_migration_clear(&root).is_err());
        fs::remove_dir_all(&library).unwrap();
        fs::create_dir(&library).unwrap();
        let mut bytes = b"{}".to_vec();
        bytes.resize(crate::claude_config::MAX_FILE_BYTES, b' ');
        for index in 0..=16 {
            fs::write(library.join(format!("{index:036x}.json")), &bytes).unwrap();
        }
        assert!(claude_native_migration_clear(&root).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn claude_native_migration_bookend_refuses_new_migration_and_own_files_do_not_change_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap().join("home");
        let root = home.join("Library/Application Support/Claude-3p");
        let library = root.join("configLibrary");
        fs::create_dir_all(&library).unwrap();
        let mut receipt = claude_receipt(root.clone());
        receipt.root_anchors = claude_configuration_root(&root).unwrap();
        let cache = std::sync::Mutex::new(None);
        claude_native_migration_clear(&root).unwrap();
        let path = library.join("00000000-0000-4000-8000-000000000001.json");
        let result = claude_verify_cached(
            &cache,
            receipt.clone(),
            || {
                claude_native_migration_clear(&root)?;
                Ok(receipt.clone())
            },
            || {
                fs::write(&path, br#"{"inferenceGatewayApiKey":"synthetic"}"#).unwrap();
                Ok(())
            },
            || Ok(()),
        );
        assert_eq!(
            result.unwrap_err(),
            fail("claude_native_migration_required")
        );
        assert!(cache.lock().unwrap().is_none());
        fs::write(
            path,
            br#"{"inferenceGatewayApiKey":"__sealed__:v1:synthetic"}"#,
        )
        .unwrap();
        claude_native_migration_clear(&root).unwrap();
        assert_eq!(
            claude_configuration_root(&root).unwrap(),
            receipt.root_anchors
        );
        receipt.validate_root().unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn claude_signature_process_success_and_failure_use_static_safe_results() {
        let mut success = Command::new("/bin/sh");
        success.args(["-c", "exit 0"]);
        claude_signature_verification(success).unwrap();

        let mut rejected = Command::new("/bin/sh");
        rejected.args([
            "-c",
            "printf 'synthetic-stdout'; printf 'synthetic-stderr' >&2; exit 7",
        ]);
        let error = claude_signature_verification(rejected).unwrap_err();
        assert_eq!(error, fail("claude_configuration_target_unverified"));
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(
            !serialized.contains("synthetic-stdout") && !serialized.contains("synthetic-stderr")
        );

        let dir = tempfile::tempdir().unwrap();
        let absent = Command::new(dir.path().join("absent-verifier"));
        assert_eq!(claude_signature_verification(absent).unwrap_err(), error);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn claude_signature_process_timeout_terminates_and_reaps_only_its_owned_child() {
        use std::os::unix::process::ExitStatusExt;
        let mut child = Command::new("/bin/sleep")
            .arg("1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        assert_eq!(
            claude_signature_wait(&mut child, Some(Duration::from_millis(40))).unwrap_err(),
            fail("claude_signature_verification_timeout")
        );
        let mut raw_status = 0;
        let waited =
            unsafe { libc::waitpid(child.id() as libc::pid_t, &mut raw_status, libc::WNOHANG) };
        let wait_error = std::io::Error::last_os_error().raw_os_error();
        assert_eq!(waited, -1);
        assert_eq!(
            wait_error,
            Some(libc::ECHILD),
            "the verifier already reaped its child"
        );
        assert_eq!(
            child.try_wait().unwrap().unwrap().signal(),
            Some(libc::SIGKILL)
        );
    }

    fn claude_receipt(root: PathBuf) -> ClaudeConfigurationTarget {
        ClaudeConfigurationTarget {
            contract_version: 1,
            app_path: "synthetic-installed-Claude.app".into(),
            bundle_id: CLAUDE_BUNDLE_ID.into(),
            app_version: crate::claude_config::SUPPORTED_APP_VERSION.into(),
            source_member_sha256: crate::claude_config::SOURCE_MEMBER_SHA256.into(),
            config_root: root,
            root_anchors: Vec::new(),
            materials: vec!["synthetic-public-material".into()],
            revision: crate::identity::sha256(b"synthetic-installed-target"),
        }
    }
    #[test]
    fn claude_b1_cached_verification_rechecks_bookends_and_revalidates_changed_materials() {
        use std::cell::Cell;
        let receipt = claude_receipt(PathBuf::from("synthetic-default-root"));
        let cache = std::sync::Mutex::new(None);
        let hash_calls = Cell::new(0);
        let signature_calls = Cell::new(0);
        let verify_signature = || {
            signature_calls.set(signature_calls.get() + 1);
            Ok(())
        };
        let verify_hash = || {
            hash_calls.set(hash_calls.get() + 1);
            Ok(())
        };
        assert_eq!(
            claude_verify_cached(
                &cache,
                receipt.clone(),
                || Ok(receipt.clone()),
                verify_signature,
                verify_hash
            )
            .unwrap(),
            receipt
        );
        claude_verify_cached(
            &cache,
            receipt.clone(),
            || Ok(receipt.clone()),
            verify_signature,
            verify_hash,
        )
        .unwrap();
        assert_eq!(
            hash_calls.get(),
            1,
            "identical public materials reuse only the large hash"
        );
        assert_eq!(signature_calls.get(), 2);
        assert!(
            claude_verify_cached(
                &cache,
                receipt.clone(),
                || Ok(receipt.clone()),
                || Err(fail("claude_configuration_target_unverified")),
                verify_hash
            )
            .is_err_and(|error| error.code == "claude_configuration_target_unverified")
        );
        assert_eq!(
            hash_calls.get(),
            1,
            "a cached hash cannot qualify a bad signature"
        );
        let mut changed = receipt.clone();
        changed.materials[0] = "replaced-entity".into();
        changed.revision = crate::identity::sha256(b"replacement");
        assert!(
            claude_verify_cached(
                &cache,
                changed.clone(),
                || Ok(changed.clone()),
                verify_signature,
                || {
                    hash_calls.set(hash_calls.get() + 1);
                    Err(fail("claude_configuration_target_unverified"))
                }
            )
            .is_err()
        );
        assert_eq!(hash_calls.get(), 2);
        // Cache hits still observe the current target; they cannot skip a
        // replacement that happens during the observation itself.
        assert!(
            claude_verify_cached(
                &cache,
                receipt.clone(),
                || Ok(changed),
                verify_signature,
                verify_hash
            )
            .is_err_and(|error| error.code == "claude_configuration_target_changed")
        );
        assert_eq!(hash_calls.get(), 2);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn claude_b1_default_root_qualification_is_read_only_and_stable_across_owned_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap().join("home");
        fs::create_dir_all(home.join("Library/Application Support")).unwrap();
        let root = home.join("Library/Application Support/Claude-3p");
        let mut receipt = claude_receipt(root.clone());
        receipt.root_anchors = claude_configuration_root(&root).unwrap();
        receipt.validate_root().unwrap();
        assert!(!root.exists(), "qualification never creates the user root");
        fs::create_dir_all(root.join("configLibrary")).unwrap();
        fs::write(root.join("configLibrary/_meta.json"), b"{}").unwrap();
        fs::write(root.join("claude_desktop_config.json"), b"{}").unwrap();
        receipt.validate_root().unwrap();
        fs::set_permissions(
            root.join("configLibrary"),
            fs::Permissions::from_mode(0o777),
        )
        .unwrap();
        assert!(receipt.validate_root().is_err());
        fs::set_permissions(
            root.join("configLibrary"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        fs::remove_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(&home, &root).unwrap();
        assert!(receipt.validate_root().is_err());
        fs::remove_file(&root).unwrap();
        let support = home.join("Library/Application Support");
        fs::rename(&support, home.join("Library/previous-support")).unwrap();
        fs::create_dir(&support).unwrap();
        assert!(receipt.validate_root().is_err());
    }

    #[test]
    fn claude_b1_unknown_installation_schema_and_root_cannot_supply_a_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let paths = SystemPaths::isolated(dir.path()).unwrap();
        let mut target = base_view(ClientKind::Claude, &paths);
        target.installed = true;
        target.bundle_id = Some(CLAUDE_BUNDLE_ID.into());
        target.version = Some(crate::claude_config::SUPPORTED_APP_VERSION.into());
        assert!(paths.claude_configuration_hint(&target).is_err());
        assert!(paths.claude_configuration_target(&target).is_err());
        assert!(
            paths
                .ensure_configuration_parent(ClientKind::Claude)
                .is_err()
        );
        let mut receipt = claude_receipt(dir.path().join("different-root"));
        receipt.app_version = "2.19675.1".into();
        assert!(receipt.validate_root().is_err());
        receipt.app_version = crate::claude_config::SUPPORTED_APP_VERSION.into();
        receipt.source_member_sha256 = "unknown".into();
        assert!(receipt.validate_root().is_err());
        receipt.source_member_sha256 = crate::claude_config::SOURCE_MEMBER_SHA256.into();
        assert!(receipt.validate_root().is_err());
        let fresh = paths.fresh_claude_client(&target);
        assert_eq!(fresh.configuration_state, target.configuration_state);
        assert_eq!(fresh.loaded_state, "unconfirmed");
        assert_eq!(fresh.connection_state, "not_tested");
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn desktop_config_flow_info_growth_after_metadata_is_read_capped_before_parsing() {
        use std::{cell::Cell, rc::Rc};
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert("CFBundleIdentifier".into(), CONFIGURATION_BUNDLE_ID.into());
        let value = plist::Value::Dictionary(dictionary);
        let mut prefix = Vec::new();
        value.to_writer_xml(&mut prefix).unwrap();
        assert_eq!(bounded_configuration_info(prefix.as_slice()), Some(value));
        // A metadata check can observe this small valid prefix before the file
        // grows. The stream may never end; its old length is not a read bound.
        assert!((prefix.len() as u64) < MAX_CONFIGURATION_INFO_BYTES);
        struct Growing<'a> {
            prefix: &'a [u8],
            consumed: Rc<Cell<usize>>,
        }
        impl Read for Growing<'_> {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                for (offset, byte) in bytes.iter_mut().enumerate() {
                    *byte = self
                        .prefix
                        .get(self.consumed.get() + offset)
                        .copied()
                        .unwrap_or(b' ');
                }
                self.consumed.set(self.consumed.get() + bytes.len());
                Ok(bytes.len())
            }
        }
        let consumed = Rc::new(Cell::new(0));
        assert!(
            bounded_configuration_info(Growing {
                prefix: &prefix,
                consumed: consumed.clone()
            })
            .is_none()
        );
        assert_eq!(consumed.get() as u64, MAX_CONFIGURATION_INFO_BYTES + 1);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn desktop_config_flow_public_file_checks_are_bounded_regular_and_do_not_run_a_client() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("public-material");
        fs::write(&path, b"public configuration schema material").unwrap();
        assert_eq!(
            configuration_hash(&path, 36).unwrap(),
            crate::identity::sha256(b"public configuration schema material")
        );
        assert!(configuration_hash(&path, 35).is_err());
        let link = path.with_file_name("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(configuration_hash(&link, 36).is_err());
        assert!(configuration_file(path.parent().unwrap()).is_err());
        assert!(configuration_hash(&path.with_file_name("missing"), 36).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn desktop_config_flow_root_anchor_survives_owned_creation_but_rejects_scope_changes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let home = base.join("home");
        fs::create_dir(&home).unwrap();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
        let paths = SystemPaths::native(&base.join("data"), &home).unwrap();
        let target = base_view(ClientKind::Codex, &paths);
        let receipt =
            CodexConfigurationTarget::synthetic(&paths, &target, "test-only installed target")
                .unwrap();
        let root = home.join(".codex");
        assert!(!root.exists());
        receipt.validate_root().unwrap();
        assert!(
            !root.exists(),
            "observing a target cannot create a directory"
        );
        receipt.ensure_root().unwrap();
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        receipt.validate_root().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        receipt.ensure_root().unwrap();
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o755,
            "existing permissions are not rewritten"
        );
        fs::remove_dir(&root).unwrap();
        std::os::unix::fs::symlink(&base, &root).unwrap();
        assert!(receipt.validate_root().is_err());
        fs::remove_file(&root).unwrap();
        fs::rename(&home, base.join("previous-home")).unwrap();
        fs::create_dir(&home).unwrap();
        assert!(receipt.validate_root().is_err());
    }
    #[test]
    fn isolated_detection_never_claims_real_clients_or_loaded_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let paths = SystemPaths::isolated(dir.path()).unwrap();
        for client in detect_clients(&paths) {
            assert!(!client.installed);
            assert!(client.loaded_evidence.is_none());
            assert_eq!(client.loaded_state, "unconfirmed");
        }
        assert_eq!(
            paths
                .ensure_configuration_parent(ClientKind::Codex)
                .unwrap_err()
                .code,
            "integration_authorization_required"
        );
    }
}
