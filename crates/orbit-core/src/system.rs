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
            #[cfg(test)]
            fixture_clients: None,
            #[cfg(test)]
            fixture_policy: None,
            #[cfg(test)]
            fixture_codex_runtime: None,
            #[cfg(test)]
            fixture_configuration_target: None,
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
            #[cfg(test)]
            fixture_clients: None,
            #[cfg(test)]
            fixture_policy: None,
            #[cfg(test)]
            fixture_codex_runtime: None,
            #[cfg(test)]
            fixture_configuration_target: None,
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
            vec!["Claude 第三方推理模式影响整个桌面应用；configLibrary envelope 尚未验证，请使用官方导入流程。".into()]
        } else {
            vec!["Codex、CLI 与 IDE 可能共用用户 config.toml，保存后需重启目标应用。".into()]
        },
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
