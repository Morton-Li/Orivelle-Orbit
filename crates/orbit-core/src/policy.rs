//! Read-only device policy observations. Raw policy values never leave a probe.
//! Device absence does not establish absence of identity-scoped cloud policy.
use crate::{
    dto::{ClientKind, ClientView},
    error::{Result, fail},
    identity::sha256,
    system::SystemPaths,
};
use serde::Serialize;
use std::{fs, io::Read, path::Path};

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_KEYS: usize = 512;
const MAX_NAME: usize = 256;
const BEHAVIOR: [&str; 7] = [
    "disableAutoUpdates",
    "autoUpdaterEnforcementHours",
    "updateViaUpdatesHost",
    "relaunchEnforcementHours",
    "configRecheckIntervalMinutes",
    "egressProxyUrl",
    "egressProxyPacUrl",
];
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
enum Status {
    Absent,
    Present,
    Denied,
    Malformed,
    Unavailable,
    Changed,
    Limit,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
enum Effect {
    None,
    Behavior,
    Managed,
    Unverified,
}
#[derive(Clone, Serialize)]
struct Source {
    id: &'static str,
    status: Status,
    effect: Effect,
    digest: String,
}
impl Source {
    fn status(id: &'static str, status: Status) -> Self {
        Self {
            id,
            status,
            effect: Effect::Unverified,
            digest: String::new(),
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum State {
    Yes,
    No,
    Unknown,
}
impl State {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Unknown => "unknown",
        }
    }
}
#[derive(Clone)]
pub(crate) struct Snapshot {
    pub(crate) state: State,
    pub(crate) revision: String,
    identity: String,
    local_write: State,
    pub(crate) local_revision: String,
}
impl Snapshot {
    fn resolve(target: &ClientView, sources: &[Source], coverage_complete: bool) -> Self {
        Self::resolve_with_local(target, sources, coverage_complete, coverage_complete)
    }
    fn resolve_with_local(
        target: &ClientView,
        sources: &[Source],
        coverage_complete: bool,
        local_complete: bool,
    ) -> Self {
        let state = if sources
            .iter()
            .any(|s| s.status == Status::Present && s.effect == Effect::Managed)
        {
            State::Yes
        } else if coverage_complete
            && target.installed
            && target.version.is_some()
            && target.bundle_id.is_some()
            && target.platform_supported
            && sources.iter().all(|s| {
                s.status == Status::Absent
                    || s.status == Status::Present
                        && matches!(s.effect, Effect::None | Effect::Behavior)
            })
        {
            State::No
        } else {
            State::Unknown
        };
        // Include every observed source, even a shadowed or equivalent source.
        // Observation time is deliberately excluded from the comparison token.
        let identity = (
            &target.client,
            target.installed,
            &target.version,
            &target.bundle_id,
            &target.app_path,
            target.platform_supported,
        );
        let identity_sources = sources
            .iter()
            .filter(|s| s.id == "client_identity")
            .collect::<Vec<_>>();
        let identity =
            sha256(&serde_json::to_vec(&(identity, identity_sources)).unwrap_or_default());
        let bytes = serde_json::to_vec(&(1u32, &identity, sources, coverage_complete, state))
            .unwrap_or_default();
        let local: Vec<_> = sources
            .iter()
            .filter(|source| source.id != "remote_organization")
            .collect();
        let local_write = if state == State::Yes {
            State::Yes
        } else if (target.client == ClientKind::Codex || claude_pinned_target(target))
            && local_complete
            && target.installed
            && target.version.is_some()
            && target.bundle_id.is_some()
            && target.platform_supported
            && local.iter().all(|s| {
                s.status == Status::Absent
                    || s.status == Status::Present
                        && matches!(s.effect, Effect::None | Effect::Behavior)
            })
        {
            State::No
        } else {
            State::Unknown
        };
        let local_revision = sha256(
            &serde_json::to_vec(&(
                if target.client == ClientKind::Claude {
                    "claude-user-config-v1"
                } else {
                    "codex-user-config-v1"
                },
                &identity,
                &local,
                local_complete,
                local_write,
            ))
            .unwrap_or_default(),
        );
        Self {
            state,
            revision: sha256(&bytes),
            identity,
            local_write,
            local_revision,
        }
    }
    pub(crate) fn user_config_state(&self) -> State {
        self.local_write
    }
    pub(crate) fn require_user_config_clear(&self) -> Result<()> {
        if self.state == State::Yes || self.local_write != State::No {
            return Err(fail("managed_policy_unverified"));
        }
        Ok(())
    }
    pub(crate) fn matches_user_config(&self, current: &Self) -> Result<()> {
        if self.identity != current.identity {
            return Err(fail("client_version_changed"));
        }
        if self.revision != current.revision || self.local_revision != current.local_revision {
            return Err(fail("managed_policy_changed"));
        }
        current.require_user_config_clear()
    }
    pub(crate) fn require_clear(&self) -> Result<()> {
        if self.state != State::No {
            return Err(fail("managed_policy_unverified"));
        }
        Ok(())
    }
    pub(crate) fn matches(&self, current: &Self) -> Result<()> {
        if self.identity != current.identity {
            return Err(fail("client_version_changed"));
        }
        if self.revision != current.revision {
            return Err(fail("managed_policy_changed"));
        }
        current.require_clear()
    }
    pub(crate) fn project(&self, target: &mut ClientView) {
        target.managed = self.state.as_str().into();
        let note = match self.state {
            State::Yes => "已发现设备管理策略；Orbit 不覆盖组织管理的客户端配置。",
            State::Unknown if self.local_write == State::No => {
                "本地用户配置策略已核对；远端组织策略未知，客户端启动后仍可能限制或拒绝配置生效。"
            }
            State::Unknown => "设备策略或远端组织策略的覆盖范围尚未确认；自动接入保持禁用。",
            State::No => "已核验当前受控观测覆盖的客户端策略；写入前仍会重新检查。",
        };
        target.manual_steps.retain(|v| {
            ![
                "已发现设备管理策略；Orbit 不覆盖组织管理的客户端配置。",
                "设备策略或远端组织策略的覆盖范围尚未确认；自动接入保持禁用。",
                "已核验当前受控观测覆盖的客户端策略；写入前仍会重新检查。",
                "本地用户配置策略已核对；远端组织策略未知，客户端启动后仍可能限制或拒绝配置生效。",
            ]
            .contains(&v.as_str())
        });
        if !target.manual_steps.iter().any(|v| v == note) {
            target.manual_steps.push(note.into());
        }
    }
    #[cfg(test)]
    pub(crate) fn fixture(target: &ClientView, state: State, token: &str) -> Self {
        let source = Source {
            id: "synthetic",
            status: Status::Present,
            effect: match state {
                State::Yes => Effect::Managed,
                State::No => Effect::None,
                State::Unknown => Effect::Unverified,
            },
            digest: sha256(token.as_bytes()),
        };
        Self::resolve(target, &[source], state == State::No)
    }
    #[cfg(test)]
    pub(crate) fn fixture_user_config(target: &ClientView, state: State, token: &str) -> Self {
        let mut snapshot = Self::fixture(target, state, token);
        if state == State::No
            && (target.client == ClientKind::Codex || claude_pinned_target(target))
        {
            snapshot.state = State::Unknown;
            snapshot.revision = sha256(format!("local-clear-global-unknown:{token}").as_bytes());
        }
        snapshot
    }
}
pub(crate) fn observe(paths: &SystemPaths, target: &ClientView) -> Snapshot {
    #[cfg(test)]
    if let Some(probe) = &paths.fixture_policy {
        return probe(target);
    }
    if paths.isolated {
        #[cfg(test)]
        {
            // Existing fixtures explicitly install a synthetic ClientView. This
            // branch does not exist in release and never inspects native state.
            let effect = if target.managed == "yes" {
                Effect::Managed
            } else if target.managed == "no" {
                Effect::None
            } else {
                Effect::Unverified
            };
            return Snapshot::resolve(
                target,
                &[Source {
                    id: "synthetic",
                    status: Status::Present,
                    effect,
                    digest: "fixture-policy-v1".into(),
                }],
                target.managed == "no",
            );
        }
        #[cfg(not(test))]
        return Snapshot::resolve(
            target,
            &[Source::status("isolated", Status::Unavailable)],
            false,
        );
    }
    let mut sources = native_sources(paths, target);
    if let Some(app) = &target.app_path {
        #[cfg(target_os = "macos")]
        sources.push(file_source(
            "client_identity",
            &Path::new(app).join("Contents/Info.plist"),
            Format::Identity,
        ));
        #[cfg(not(target_os = "macos"))]
        let _ = app;
    }
    // Neither CFPreferences, filesystem absence nor the Windows registry reports
    // which cloud/Agent Security/admin-console layer the running desktop loaded.
    sources.push(Source::status("remote_organization", Status::Unavailable));
    let local_complete = match target.client {
        ClientKind::Codex => codex_local_coverage(target, &sources),
        ClientKind::Claude => claude_local_coverage(target, &sources),
    };
    Snapshot::resolve_with_local(target, &sources, false, local_complete)
}
fn codex_local_coverage(target: &ClientView, sources: &[Source]) -> bool {
    let required = [
        "codex_system_requirements",
        "codex_legacy_defaults",
        "codex_macos_mdm_defaults",
        "codex_macos_mdm_requirements",
        "client_identity",
    ];
    cfg!(target_os = "macos")
        && target.client == ClientKind::Codex
        && required
            .iter()
            .all(|id| sources.iter().filter(|source| source.id == *id).count() == 1)
}
fn claude_pinned_target(target: &ClientView) -> bool {
    target.client == ClientKind::Claude
        && target.version.as_deref() == Some(crate::claude_config::SUPPORTED_APP_VERSION)
        && target.bundle_id.as_deref() == Some("com.anthropic.claudefordesktop")
}
fn claude_local_coverage(target: &ClientView, sources: &[Source]) -> bool {
    cfg!(target_os = "macos")
        && claude_pinned_target(target)
        && [
            "claude_machine_mdm",
            "claude_user_mdm",
            "claude_macos_forced_keys",
            "claude_schema_pin",
            "claude_default_locations",
            "client_identity",
        ]
        .iter()
        .all(|id| sources.iter().filter(|source| source.id == *id).count() == 1)
}
fn claude_key_effect<'a>(keys: impl Iterator<Item = &'a str>) -> Effect {
    let mut effect = Effect::None;
    for key in keys {
        if BEHAVIOR.contains(&key) {
            if effect == Effect::None {
                effect = Effect::Behavior;
            }
        } else if crate::claude_config::SCHEMA_FLAT_KEYS.contains(&key) {
            return Effect::Managed;
        } else {
            effect = Effect::Unverified;
        }
    }
    effect
}
#[cfg(any(target_os = "macos", test))]
fn claude_forced_observation(before: &[(&str, bool)], after: &[(&str, bool)]) -> Source {
    let id = "claude_macos_forced_keys";
    let complete = |values: &[(&str, bool)]| {
        values.len() == crate::claude_config::SCHEMA_FLAT_KEYS.len()
            && values
                .iter()
                .zip(crate::claude_config::SCHEMA_FLAT_KEYS)
                .all(|((name, _), expected)| name == expected)
    };
    if !complete(before) || !complete(after) {
        return Source::status(id, Status::Unavailable);
    }
    if before != after {
        return Source::status(id, Status::Changed);
    }
    let forced = before
        .iter()
        .filter_map(|(key, forced)| forced.then_some(*key))
        .collect::<Vec<_>>();
    Source {
        id,
        status: if forced.is_empty() {
            Status::Absent
        } else {
            Status::Present
        },
        effect: claude_key_effect(forced.iter().copied()),
        digest: sha256(
            &serde_json::to_vec(&(
                crate::claude_config::SOURCE_MEMBER_SHA256,
                crate::claude_config::SCHEMA_KEYS_SHA256,
                before,
            ))
            .unwrap_or_default(),
        ),
    }
}
#[cfg(any(target_os = "macos", test))]
fn claude_default_locations(user_data_override: bool, managed_override: bool) -> Source {
    Source {
        id: "claude_default_locations",
        status: if user_data_override || managed_override {
            Status::Present
        } else {
            Status::Absent
        },
        effect: if user_data_override || managed_override {
            Effect::Unverified
        } else {
            Effect::None
        },
        // Presence is enough to reject unsupported overrides. Their values are
        // neither read as paths nor included in a policy token.
        digest: sha256(
            &serde_json::to_vec(&(user_data_override, managed_override)).unwrap_or_default(),
        ),
    }
}
#[derive(Clone, Copy)]
enum Format {
    Toml,
    Plist,
    ClaudePlist,
    Identity,
}
#[cfg(any(target_os = "macos", test))]
fn cf_payload(id: &'static str, forced: bool, raw: Option<&[u8]>) -> Source {
    let Some(raw) = raw else {
        return Source::status(
            id,
            if forced {
                Status::Unavailable
            } else {
                Status::Absent
            },
        );
    };
    if raw.len() as u64 > MAX_BYTES {
        return Source::status(id, Status::Limit);
    }
    use base64::Engine;
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(raw) else {
        return Source::status(id, Status::Malformed);
    };
    let decoded = zeroize::Zeroizing::new(decoded);
    let Some(doc) = std::str::from_utf8(&decoded)
        .ok()
        .and_then(|s| s.parse::<toml_edit::DocumentMut>().ok())
    else {
        return Source::status(id, Status::Malformed);
    };
    if doc.iter().count() > MAX_KEYS {
        return Source::status(id, Status::Limit);
    }
    Source {
        id,
        status: Status::Present,
        effect: if forced {
            Effect::Managed
        } else {
            Effect::Unverified
        },
        digest: sha256(format!("{}:{}", forced, sha256(raw)).as_bytes()),
    }
}
fn relevant_effect(keys: impl Iterator<Item = String>) -> Effect {
    let mut behavior = false;
    let mut unknown = false;
    for key in keys {
        if BEHAVIOR.contains(&key.as_str()) {
            behavior = true;
        } else if [
            "inferenceProvider",
            "inferenceGatewayBaseUrl",
            "inferenceGatewayApiKey",
            "inferenceCredentialKind",
            "inferenceModels",
            "bootstrapUrl",
            "bootstrapEnabled",
            "forceLoginOrgUUID",
            "disableDeploymentModeChooser",
            "desktopHome",
            "credentialHelper",
        ]
        .contains(&key.as_str())
        {
            return Effect::Managed;
        } else {
            unknown = true;
        }
    }
    if unknown {
        Effect::Unverified
    } else if behavior {
        Effect::Behavior
    } else {
        Effect::None
    }
}
fn stamp(metadata: &fs::Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        format!(
            "{}:{:?}:{}:{}:{}:{}:{}:{}:{}",
            metadata.len(),
            metadata.modified().ok(),
            metadata.dev(),
            metadata.ino(),
            metadata.mode(),
            metadata.uid(),
            metadata.gid(),
            metadata.ctime(),
            metadata.ctime_nsec()
        )
    }
    #[cfg(not(unix))]
    {
        format!(
            "{}:{:?}:{}",
            metadata.len(),
            metadata.modified().ok(),
            metadata.permissions().readonly()
        )
    }
}
fn file_source(id: &'static str, path: &Path, format: Format) -> Source {
    file_probe(id, path, format, || {})
}
fn open_policy_file(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    // Linux is only a core-fixture host. These ABI constants match the already
    // locked libc definitions for the two supported fixture architectures.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x20000 | 0x800);
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x8000 | 0x800);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "windows",
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    )))]
    {
        let _ = (options, path);
        return Err(std::io::Error::other("unsupported policy reader"));
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "windows",
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    ))]
    options.open(path)
}
fn regular(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    metadata.is_file() && !metadata.file_type().is_symlink()
}
fn file_probe(id: &'static str, path: &Path, format: Format, before_open: impl FnOnce()) -> Source {
    let before = match fs::symlink_metadata(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Source::status(id, Status::Absent);
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return Source::status(id, Status::Denied);
        }
        Err(_) => return Source::status(id, Status::Unavailable),
    };
    if !regular(&before) {
        return Source::status(id, Status::Malformed);
    }
    if before.len() > MAX_BYTES {
        return Source::status(id, Status::Limit);
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    before_open();
    let mut file = match open_policy_file(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return Source::status(id, Status::Denied);
        }
        Err(_) => {
            return Source::status(
                id,
                if fs::symlink_metadata(path)
                    .ok()
                    .is_none_or(|m| !regular(&m) || stamp(&m) != stamp(&before))
                {
                    Status::Changed
                } else {
                    Status::Unavailable
                },
            );
        }
    };
    if file
        .metadata()
        .ok()
        .is_none_or(|m| !regular(&m) || stamp(&m) != stamp(&before))
    {
        return Source::status(id, Status::Changed);
    }
    if file
        .by_ref()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Source::status(id, Status::Unavailable);
    }
    if bytes.len() as u64 > MAX_BYTES {
        return Source::status(id, Status::Limit);
    }
    if fs::symlink_metadata(path)
        .ok()
        .is_none_or(|m| !regular(&m) || stamp(&m) != stamp(&before))
    {
        return Source::status(id, Status::Changed);
    }
    let effect = match format {
        Format::Toml => {
            let Some(doc) = std::str::from_utf8(&bytes)
                .ok()
                .and_then(|v| v.parse::<toml_edit::DocumentMut>().ok())
            else {
                return Source::status(id, Status::Malformed);
            };
            if doc.iter().count() > MAX_KEYS {
                return Source::status(id, Status::Limit);
            }
            if doc.is_empty() {
                Effect::None
            } else {
                Effect::Managed
            }
        }
        Format::Plist | Format::ClaudePlist | Format::Identity => {
            let Ok(value) = plist::Value::from_reader(std::io::Cursor::new(bytes.as_slice()))
            else {
                return Source::status(id, Status::Malformed);
            };
            let Some(dict) = value.as_dictionary() else {
                return Source::status(id, Status::Malformed);
            };
            if dict.len() > MAX_KEYS || dict.keys().any(|k| k.len() > MAX_NAME) {
                return Source::status(id, Status::Limit);
            }
            if matches!(format, Format::Identity) {
                Effect::None
            } else if matches!(format, Format::ClaudePlist) {
                claude_key_effect(dict.keys().map(String::as_str))
            } else {
                relevant_effect(dict.keys().cloned())
            }
        }
    };
    Source {
        id,
        status: Status::Present,
        effect,
        digest: sha256(format!("{}:{}", stamp(&before), sha256(&bytes)).as_bytes()),
    }
}
fn native_sources(paths: &SystemPaths, target: &ClientView) -> Vec<Source> {
    #[cfg(target_os = "macos")]
    {
        let _ = paths;
        match target.client {
            ClientKind::Codex => {
                let mut sources = vec![
                    file_source(
                        "codex_system_requirements",
                        Path::new("/etc/codex/requirements.toml"),
                        Format::Toml,
                    ),
                    file_source(
                        "codex_legacy_defaults",
                        Path::new("/etc/codex/managed_config.toml"),
                        Format::Toml,
                    ),
                ];
                sources.extend(macos::codex_mdm());
                sources
            }
            ClientKind::Claude => {
                let format = if claude_pinned_target(target) {
                    Format::ClaudePlist
                } else {
                    Format::Plist
                };
                let mut sources = vec![file_source(
                    "claude_machine_mdm",
                    Path::new("/Library/Managed Preferences/com.anthropic.claudefordesktop.plist"),
                    format,
                )];
                match macos::username() {
                    Some(user) => sources.push(file_source(
                        "claude_user_mdm",
                        &Path::new("/Library/Managed Preferences")
                            .join(user)
                            .join("com.anthropic.claudefordesktop.plist"),
                        format,
                    )),
                    None => sources.push(Source::status("claude_user_mdm", Status::Unavailable)),
                }
                sources.push(macos::claude_forced_keys());
                sources.push(if claude_pinned_target(target) {
                    Source {
                        id: "claude_schema_pin",
                        status: Status::Present,
                        effect: Effect::None,
                        digest: sha256(
                            &serde_json::to_vec(&(
                                crate::claude_config::SUPPORTED_APP_VERSION,
                                crate::claude_config::SOURCE_MEMBER_SHA256,
                                crate::claude_config::SCHEMA_KEYS_SHA256,
                            ))
                            .unwrap_or_default(),
                        ),
                    }
                } else {
                    Source::status("claude_schema_pin", Status::Unavailable)
                });
                sources.push(claude_default_locations(
                    std::env::var_os("CLAUDE_USER_DATA_DIR").is_some(),
                    std::env::var_os("CLAUDE_E2E_MANAGED_PLIST").is_some(),
                ));
                sources
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        match target.client {
            ClientKind::Codex => {
                let mut sources = vec![file_source(
                    "codex_legacy_defaults",
                    &paths.codex_config.with_file_name("managed_config.toml"),
                    Format::Toml,
                )];
                if let Some(root) = std::env::var_os("ProgramData") {
                    sources.push(file_source(
                        "codex_system_requirements",
                        &std::path::PathBuf::from(root).join("OpenAI/Codex/requirements.toml"),
                        Format::Toml,
                    ));
                } else {
                    sources.push(Source::status(
                        "codex_system_requirements",
                        Status::Unavailable,
                    ));
                }
                sources
            }
            ClientKind::Claude => windows::claude_sources(target.version.as_deref()),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (paths, target);
        vec![Source::status("unsupported_platform", Status::Unavailable)]
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::ffi::{CStr, c_char, c_void};
    type Ref = *const c_void;
    const UTF8: u32 = 0x08000100;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(allocator: Ref, value: *const c_char, encoding: u32) -> Ref;
        fn CFStringGetTypeID() -> usize;
        fn CFStringGetLength(value: Ref) -> isize;
        fn CFStringGetCString(value: Ref, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFGetTypeID(value: Ref) -> usize;
        fn CFRelease(value: Ref);
        fn CFPreferencesCopyAppValue(key: Ref, application: Ref) -> Ref;
        fn CFPreferencesAppValueIsForced(key: Ref, application: Ref) -> u8;
    }
    struct Owned(Ref);
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }
    fn string(value: &CStr) -> Option<Owned> {
        let result = unsafe { CFStringCreateWithCString(std::ptr::null(), value.as_ptr(), UTF8) };
        (!result.is_null()).then_some(Owned(result))
    }
    pub(super) fn username() -> Option<String> {
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut buffer = vec![0u8; 64 * 1024];
        let mut result = std::ptr::null_mut();
        if unsafe {
            libc::getpwuid_r(
                libc::geteuid(),
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        } != 0
            || result.is_null()
        {
            return None;
        }
        let entry = unsafe { entry.assume_init() };
        if entry.pw_name.is_null() {
            return None;
        }
        let name = unsafe { CStr::from_ptr(entry.pw_name) }.to_str().ok()?;
        if name.is_empty()
            || name.len() > MAX_NAME
            || name.contains(['/', '\\'])
            || name == "."
            || name == ".."
        {
            return None;
        }
        Some(name.into())
    }
    pub(super) fn codex_mdm() -> Vec<Source> {
        let Some(domain) = string(c"com.openai.codex") else {
            return vec![Source::status("codex_macos_mdm", Status::Unavailable)];
        };
        [
            ("codex_macos_mdm_defaults", c"config_toml_base64"),
            ("codex_macos_mdm_requirements", c"requirements_toml_base64"),
        ]
        .into_iter()
        .map(|(id, literal)| value_source(id, literal, &domain))
        .collect()
    }
    fn value_source(id: &'static str, literal: &CStr, domain: &Owned) -> Source {
        let Some(key) = string(literal) else {
            return Source::status(id, Status::Unavailable);
        };
        let forced = unsafe { CFPreferencesAppValueIsForced(key.0, domain.0) };
        let value = Owned(unsafe { CFPreferencesCopyAppValue(key.0, domain.0) });
        if unsafe { CFPreferencesAppValueIsForced(key.0, domain.0) } != forced {
            return Source::status(id, Status::Changed);
        }
        if value.0.is_null() {
            return cf_payload(id, forced != 0, None);
        }
        if unsafe { CFGetTypeID(value.0) != CFStringGetTypeID() } {
            return Source::status(id, Status::Malformed);
        }
        let length = unsafe { CFStringGetLength(value.0) };
        if length < 0 || length as u64 > MAX_BYTES / 4 {
            return Source::status(id, Status::Limit);
        }
        let mut raw = zeroize::Zeroizing::new(vec![0u8; (length as usize) * 4 + 1]);
        if unsafe { CFStringGetCString(value.0, raw.as_mut_ptr().cast(), raw.len() as isize, UTF8) }
            == 0
        {
            return Source::status(id, Status::Malformed);
        }
        let length = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
        cf_payload(id, forced != 0, Some(&raw[..length]))
    }
    pub(super) fn claude_forced_keys() -> Source {
        let id = "claude_macos_forced_keys";
        let Some(domain) = string(c"com.anthropic.claudefordesktop") else {
            return Source::status(id, Status::Unavailable);
        };
        let mut keys = Vec::with_capacity(crate::claude_config::SCHEMA_FLAT_KEYS.len());
        for literal in crate::claude_config::SCHEMA_FLAT_KEYS {
            let Ok(literal) = std::ffi::CString::new(*literal) else {
                return Source::status(id, Status::Unavailable);
            };
            let Some(key) = string(&literal) else {
                return Source::status(id, Status::Unavailable);
            };
            keys.push(key);
        }
        let query = || {
            crate::claude_config::SCHEMA_FLAT_KEYS
                .iter()
                .zip(&keys)
                .map(|(name, key)| {
                    (
                        *name,
                        unsafe { CFPreferencesAppValueIsForced(key.0, domain.0) } != 0,
                    )
                })
                .collect::<Vec<_>>()
        };
        // No CFPreferencesCopyAppValue: credential values are never requested.
        // Complete fixed-key status passes must agree before absence is usable.
        claude_forced_observation(&query(), &query())
    }
}

// Registry metadata only: no value data, process execution, writes, or subkeys.
#[cfg(any(target_os = "windows", test))]
fn registry_observation(
    id: &'static str,
    values: &std::collections::BTreeMap<String, (u32, u32)>,
    before: u64,
    after: u64,
) -> (Source, bool) {
    if before != after {
        return (Source::status(id, Status::Changed), false);
    }
    if values.len() > MAX_KEYS || values.keys().any(|v| v.len() > MAX_NAME) {
        return (Source::status(id, Status::Limit), false);
    }
    let visible = values.values().any(|(kind, _)| matches!(*kind, 1 | 2 | 4));
    let keys = values
        .iter()
        .filter(|(_, (kind, _))| matches!(*kind, 1 | 4))
        .map(|(name, _)| name.clone());
    let effect = if values.values().any(|(kind, _)| *kind == 2) {
        Effect::Unverified
    } else {
        relevant_effect(keys)
    };
    (
        Source {
            id,
            status: if visible {
                Status::Present
            } else {
                Status::Absent
            },
            effect,
            digest: sha256(&serde_json::to_vec(&(values, before)).unwrap_or_default()),
        },
        visible,
    )
}
#[cfg(any(target_os = "windows", test))]
fn choose_registry_sources(
    machine: (Source, bool),
    user: impl FnOnce() -> (Source, bool),
) -> Vec<Source> {
    if machine.1 || machine.0.status != Status::Absent {
        return vec![
            machine.0,
            Source::status("claude_user_registry_shadowed", Status::Unavailable),
        ];
    }
    vec![machine.0, user().0]
}
#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use std::collections::BTreeMap;
    use std::ffi::c_void;
    type HKey = *mut c_void;
    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegOpenKeyExW(
            key: HKey,
            subkey: *const u16,
            options: u32,
            access: u32,
            result: *mut HKey,
        ) -> i32;
        fn RegCloseKey(key: HKey) -> i32;
        fn RegEnumValueW(
            key: HKey,
            index: u32,
            name: *mut u16,
            name_len: *mut u32,
            reserved: *mut u32,
            kind: *mut u32,
            data: *mut u8,
            data_len: *mut u32,
        ) -> i32;
        fn RegQueryInfoKeyW(
            key: HKey,
            class: *mut u16,
            class_len: *mut u32,
            reserved: *mut u32,
            subkeys: *mut u32,
            max_subkey: *mut u32,
            max_class: *mut u32,
            values: *mut u32,
            max_name: *mut u32,
            max_data: *mut u32,
            security: *mut u32,
            last_write: *mut FileTime,
        ) -> i32;
    }
    struct Key(HKey);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe { RegCloseKey(self.0) };
        }
    }
    fn info(key: &Key) -> Option<(u32, u32, FileTime)> {
        let (mut count, mut max_name) = (0, 0);
        let mut time = FileTime { low: 0, high: 0 };
        let code = unsafe {
            RegQueryInfoKeyW(
                key.0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut count,
                &mut max_name,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut time,
            )
        };
        (code == 0).then_some((count, max_name, time))
    }
    fn probe(id: &'static str, hive: isize) -> (Source, bool) {
        let path = "SOFTWARE\\Policies\\Claude\0"
            .encode_utf16()
            .collect::<Vec<_>>();
        let mut raw = std::ptr::null_mut();
        let code =
            unsafe { RegOpenKeyExW(hive as HKey, path.as_ptr(), 0, 0x0001 | 0x0100, &mut raw) };
        if code != 0 {
            return (
                Source::status(
                    id,
                    if code == 2 || code == 3 {
                        Status::Absent
                    } else if code == 5 {
                        Status::Denied
                    } else {
                        Status::Unavailable
                    },
                ),
                false,
            );
        }
        let key = Key(raw);
        let Some(before) = info(&key) else {
            return (Source::status(id, Status::Unavailable), false);
        };
        if before.0 as usize > MAX_KEYS || before.1 as usize > MAX_NAME {
            return (Source::status(id, Status::Limit), false);
        }
        let mut values = BTreeMap::new();
        for i in 0..before.0 {
            let mut buffer = [0u16; MAX_NAME + 1];
            let (mut length, mut kind, mut data_length) = (buffer.len() as u32, 0, 0);
            let code = unsafe {
                RegEnumValueW(
                    key.0,
                    i,
                    buffer.as_mut_ptr(),
                    &mut length,
                    std::ptr::null_mut(),
                    &mut kind,
                    std::ptr::null_mut(),
                    &mut data_length,
                )
            };
            if code != 0 || length as usize > MAX_NAME {
                return (
                    Source::status(
                        id,
                        if code == 234 {
                            Status::Limit
                        } else {
                            Status::Changed
                        },
                    ),
                    false,
                );
            }
            let Ok(name) = String::from_utf16(&buffer[..length as usize]) else {
                return (Source::status(id, Status::Malformed), false);
            };
            values.insert(name, (kind, data_length));
        }
        if info(&key) != Some(before) {
            return (Source::status(id, Status::Changed), false);
        }
        let time = (u64::from(before.2.high) << 32) | u64::from(before.2.low);
        registry_observation(id, &values, time, time)
    }
    pub(super) fn claude_sources(version: Option<&str>) -> Vec<Source> {
        let parts = version.and_then(|v| {
            v.split('.')
                .map(str::parse::<u32>)
                .collect::<std::result::Result<Vec<_>, _>>()
                .ok()
        });
        if !parts
            .as_ref()
            .is_some_and(|v| (3..=4).contains(&v.len()) && (v[0], v[1], v[2]) >= (1, 19367, 0))
        {
            return vec![Source::status(
                "claude_registry_contract_version",
                Status::Unavailable,
            )];
        }
        choose_registry_sources(probe("claude_machine_registry", -2147483646isize), || {
            probe("claude_user_registry", -2147483647isize)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn claude_target() -> ClientView {
        let mut target = target();
        target.client = ClientKind::Claude;
        target.version = Some(crate::claude_config::SUPPORTED_APP_VERSION.into());
        target.bundle_id = Some("com.anthropic.claudefordesktop".into());
        target
    }
    fn claude_forced(keys: &[&str]) -> Source {
        let values = crate::claude_config::SCHEMA_FLAT_KEYS
            .iter()
            .map(|name| (*name, keys.contains(name)))
            .collect::<Vec<_>>();
        claude_forced_observation(&values, &values)
    }
    fn claude_sources() -> Vec<Source> {
        vec![
            Source::status("claude_machine_mdm", Status::Absent),
            Source::status("claude_user_mdm", Status::Absent),
            claude_forced(&[]),
            Source {
                id: "claude_schema_pin",
                status: Status::Present,
                effect: Effect::None,
                digest: crate::claude_config::SCHEMA_KEYS_SHA256.into(),
            },
            claude_default_locations(false, false),
            Source {
                id: "client_identity",
                status: Status::Present,
                effect: Effect::None,
                digest: "synthetic-identity".into(),
            },
            Source::status("remote_organization", Status::Unavailable),
        ]
    }
    #[test]
    fn claude_b1_forced_status_requires_all_pinned_keys_and_stable_passes() {
        let empty = claude_forced(&[]);
        assert!(empty.status == Status::Absent && empty.effect == Effect::None);
        let outside_old_four = claude_forced(&["inferenceCredentialHelperArgs"]);
        assert!(
            outside_old_four.status == Status::Present
                && outside_old_four.effect == Effect::Managed
        );
        let behavior = claude_forced(&["disableAutoUpdates", "configRecheckIntervalMinutes"]);
        assert!(behavior.effect == Effect::Behavior);
        let values = crate::claude_config::SCHEMA_FLAT_KEYS
            .iter()
            .map(|name| (*name, false))
            .collect::<Vec<_>>();
        assert!(
            claude_forced_observation(&values[..4], &values[..4]).status == Status::Unavailable
        );
        let mut changed = values.clone();
        changed[0].1 = true;
        assert!(claude_forced_observation(&values, &changed).status == Status::Changed);
        changed = values.clone();
        changed[0].0 = changed[1].0;
        assert!(claude_forced_observation(&changed, &changed).status == Status::Unavailable);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn claude_b1_complete_local_policy_keeps_remote_unknown_and_refuses_missing_probes() {
        let target = claude_target();
        let sources = claude_sources();
        let resolve = |target: &ClientView, sources: &[Source]| {
            Snapshot::resolve_with_local(
                target,
                sources,
                false,
                claude_local_coverage(target, sources),
            )
        };
        let clear = resolve(&target, &sources);
        assert!(clear.state == State::Unknown && clear.user_config_state() == State::No);
        assert!(clear.require_clear().is_err() && clear.require_user_config_clear().is_ok());
        let mut projected = target.clone();
        clear.project(&mut projected);
        assert_eq!(projected.managed, "unknown");
        let mut behavior = sources.clone();
        behavior[2] = claude_forced(&["disableAutoUpdates"]);
        let next = resolve(&target, &behavior);
        assert!(next.require_user_config_clear().is_ok());
        assert!(clear.matches_user_config(&next).is_err());
        for index in 0..sources.len() - 1 {
            let mut missing = sources.clone();
            missing.remove(index);
            assert!(
                resolve(&target, &missing)
                    .require_user_config_clear()
                    .is_err()
            );
        }
        for status in [
            Status::Denied,
            Status::Malformed,
            Status::Unavailable,
            Status::Changed,
            Status::Limit,
        ] {
            let mut unreadable = sources.clone();
            unreadable[0] = Source::status("claude_machine_mdm", status);
            assert!(
                resolve(&target, &unreadable)
                    .require_user_config_clear()
                    .is_err()
            );
        }
        for override_state in [(true, false), (false, true)] {
            let mut overridden = sources.clone();
            overridden[4] = claude_default_locations(override_state.0, override_state.1);
            assert!(
                resolve(&target, &overridden)
                    .require_user_config_clear()
                    .is_err()
            );
        }
        for index in [2, sources.len() - 1] {
            let mut managed = sources.clone();
            managed[index].status = Status::Present;
            managed[index].effect = Effect::Managed;
            let snapshot = resolve(&target, &managed);
            assert!(snapshot.state == State::Yes && snapshot.require_user_config_clear().is_err());
        }
        let mut unsupported = target;
        unsupported.version = Some("2.19675.1".into());
        assert!(
            resolve(&unsupported, &sources)
                .require_user_config_clear()
                .is_err()
        );
    }

    #[test]
    fn claude_b1_mdm_full_key_probe_preserves_unknown_and_managed_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("synthetic-managed.plist");
        let probe = |key: &str| {
            let mut dictionary = plist::Dictionary::new();
            dictionary.insert(key.into(), "synthetic-value-never-exposed".into());
            plist::Value::Dictionary(dictionary)
                .to_file_xml(&path)
                .unwrap();
            file_source("claude_machine_mdm", &path, Format::ClaudePlist)
        };
        assert!(probe("inferenceVertexOAuthClientSecret").effect == Effect::Managed);
        assert!(probe("disableAutoUpdates").effect == Effect::Behavior);
        assert!(probe("unknownNewManagedKey").effect == Effect::Unverified);
        fs::write(&path, b"not-a-plist").unwrap();
        assert!(
            file_source("claude_machine_mdm", &path, Format::ClaudePlist).status
                == Status::Malformed
        );
        fs::remove_file(&path).unwrap();
        assert!(
            file_source("claude_machine_mdm", &path, Format::ClaudePlist).status == Status::Absent
        );
    }
    #[cfg(not(windows))]
    #[test]
    fn desktop_config_flow_local_qualification_never_claims_global_policy_clear() {
        let target = target();
        let local = [
            "codex_system_requirements",
            "codex_legacy_defaults",
            "codex_macos_mdm_defaults",
            "codex_macos_mdm_requirements",
            "client_identity",
        ];
        let mut sources: Vec<_> = local
            .into_iter()
            .map(|id| Source::status(id, Status::Absent))
            .collect();
        sources.push(Source::status("remote_organization", Status::Unavailable));
        let snapshot = Snapshot::resolve_with_local(
            &target,
            &sources,
            false,
            codex_local_coverage(&target, &sources),
        );
        assert!(snapshot.state == State::Unknown);
        assert!(snapshot.require_clear().is_err() && snapshot.require_user_config_clear().is_ok());
        let mut view = target.clone();
        snapshot.project(&mut view);
        assert_eq!(view.managed, "unknown");
        for status in [
            Status::Denied,
            Status::Malformed,
            Status::Unavailable,
            Status::Changed,
            Status::Limit,
        ] {
            let mut changed = sources.clone();
            changed[0] = Source::status(local[0], status);
            assert!(
                Snapshot::resolve_with_local(
                    &target,
                    &changed,
                    false,
                    codex_local_coverage(&target, &changed)
                )
                .require_user_config_clear()
                .is_err()
            );
        }
        let mut missing = sources.clone();
        missing.remove(0);
        assert!(
            Snapshot::resolve_with_local(
                &target,
                &missing,
                false,
                codex_local_coverage(&target, &missing)
            )
            .require_user_config_clear()
            .is_err()
        );
        for index in [0, sources.len() - 1] {
            let mut managed = sources.clone();
            managed[index].status = Status::Present;
            managed[index].effect = Effect::Managed;
            assert!(
                Snapshot::resolve_with_local(&target, &managed, false, true)
                    .require_user_config_clear()
                    .is_err()
            );
        }
        let mut unknown = sources;
        unknown[0].status = Status::Present;
        unknown[0].effect = Effect::Unverified;
        assert!(
            Snapshot::resolve_with_local(&target, &unknown, false, true)
                .require_user_config_clear()
                .is_err()
        );
    }
    use std::collections::BTreeMap;
    fn target() -> ClientView {
        serde_json::from_value(serde_json::json!({"client":"codex","installed":true,"display_name":"Synthetic","version":"1.2.3","bundle_id":"synthetic.codex","app_path":null,"config_paths":[],"platform_supported":true,"managed":"unknown","configuration_state":"not_configured","loaded_state":"unconfirmed","connection_state":"not_tested","configuration_revision":null,"loaded_evidence":null,"connection_evidence":null,"manual_steps":[]})).unwrap()
    }
    #[test]
    fn ordinary_cf_defaults_are_not_reported_as_managed_and_forced_absence_is_unknown() {
        use base64::Engine;
        let raw = base64::engine::general_purpose::STANDARD.encode(b"model='synthetic'\n");
        let ordinary = cf_payload("defaults", false, Some(raw.as_bytes()));
        assert!(ordinary.effect == Effect::Unverified);
        assert!(Snapshot::resolve(&target(), &[ordinary], false).state == State::Unknown);
        let forced = cf_payload("requirements", true, Some(raw.as_bytes()));
        assert!(forced.effect == Effect::Managed);
        assert!(
            Snapshot::resolve(
                &target(),
                &[forced, cf_payload("defaults", true, None)],
                false
            )
            .state
                == State::Yes
        );
        assert!(cf_payload("defaults", true, None).status == Status::Unavailable);
        assert!(cf_payload("defaults", true, Some(b"bad-base64")).status == Status::Malformed);
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn stat_open_replacement_with_fifo_or_symlink_returns_without_following_or_blocking() {
        use std::os::unix::{ffi::OsStrExt, fs::symlink};
        for fifo in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("policy.toml");
            fs::write(&path, b"model='synthetic'\n").unwrap();
            let secret = dir.path().join("other.toml");
            fs::write(&secret, b"SYNTHETIC_DO_NOT_READ").unwrap();
            let (send, receive) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let source = file_probe("system", &path, Format::Toml, || {
                    fs::remove_file(&path).unwrap();
                    if fifo {
                        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
                        #[cfg(target_os = "macos")]
                        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
                        #[cfg(target_os = "linux")]
                        {
                            unsafe extern "C" {
                                fn mkfifo(path: *const std::ffi::c_char, mode: u32) -> i32;
                            }
                            assert_eq!(unsafe { mkfifo(name.as_ptr(), 0o600) }, 0);
                        }
                    } else {
                        symlink(&secret, &path).unwrap();
                    }
                });
                send.send((source.status, source.digest, dir)).unwrap();
            });
            let (status, digest, _owned) = receive
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            assert!(status == Status::Changed);
            assert!(digest.is_empty());
        }
    }
    #[test]
    fn local_absence_does_not_prove_cloud_or_console_absence_and_positive_hits_win() {
        for client in [ClientKind::Codex, ClientKind::Claude] {
            let mut target = target();
            target.client = client;
            let absent = vec![Source::status("device", Status::Absent)];
            assert!(Snapshot::resolve(&target, &absent, false).state == State::Unknown);
            assert!(Snapshot::resolve(&target, &absent, true).state == State::No);
            let mut incomplete = absent;
            incomplete.push(Source::status("unreadable", Status::Denied));
            assert!(Snapshot::resolve(&target, &incomplete, true).state == State::Unknown);
            incomplete.push(Source {
                id: "managed",
                status: Status::Present,
                effect: Effect::Managed,
                digest: "synthetic".into(),
            });
            assert!(Snapshot::resolve(&target, &incomplete, false).state == State::Yes);
        }
    }
    #[test]
    fn unreadable_changed_malformed_unavailable_or_missing_identity_never_yields_clear() {
        for status in [
            Status::Denied,
            Status::Malformed,
            Status::Unavailable,
            Status::Changed,
            Status::Limit,
        ] {
            assert!(
                Snapshot::resolve(&target(), &[Source::status("synthetic", status)], true)
                    .require_clear()
                    .is_err()
            );
        }
        let mut target = target();
        target.bundle_id = None;
        assert!(
            Snapshot::resolve(
                &target,
                &[Source::status("synthetic", Status::Absent)],
                true
            )
            .require_clear()
            .is_err()
        );
    }
    #[test]
    fn all_seven_behavior_keys_stay_local_but_unknown_keys_do_not_prove_clear() {
        assert!(relevant_effect(BEHAVIOR.iter().map(|v| (*v).into())) == Effect::Behavior);
        assert!(relevant_effect(["inferenceProvider".into()].into_iter()) == Effect::Managed);
        assert!(
            relevant_effect(["unrecognizedFuturePolicy".into()].into_iter()) == Effect::Unverified
        );
        let source = Source {
            id: "behavior",
            status: Status::Present,
            effect: Effect::Behavior,
            digest: "synthetic".into(),
        };
        assert!(Snapshot::resolve(&target(), &[source.clone()], false).state == State::Unknown);
        assert!(Snapshot::resolve(&target(), &[source], true).state == State::No);
    }
    #[test]
    fn source_and_identity_changes_invalidate_equivalent_complete_observations() {
        let target = target();
        let before = Snapshot::fixture(&target, State::No, "source-A");
        assert!(
            before
                .matches(&Snapshot::fixture(&target, State::No, "source-A"))
                .is_ok()
        );
        assert_eq!(
            before
                .matches(&Snapshot::fixture(&target, State::No, "source-B"))
                .unwrap_err()
                .code,
            "managed_policy_changed"
        );
        let mut other = target.clone();
        other.version = Some("1.2.4".into());
        assert!(
            before
                .matches(&Snapshot::fixture(&other, State::No, "source-A"))
                .is_err()
        );
    }
    #[test]
    fn file_probes_are_bounded_syntax_checked_and_never_publish_policy_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("synthetic.toml");
        assert!(file_source("system", &path, Format::Toml).status == Status::Absent);
        fs::write(&path, b"model='SYNTHETIC_SECRET_DO_NOT_PUBLISH'\n").unwrap();
        let source = file_source("system", &path, Format::Toml);
        assert!(source.status == Status::Present && source.effect == Effect::Managed);
        let mut view = target();
        Snapshot::resolve(&view, &[source.clone()], false).project(&mut view);
        for text in [
            serde_json::to_string(&view).unwrap(),
            serde_json::to_string(&source).unwrap(),
        ] {
            assert!(!text.contains("SYNTHETIC_SECRET") && !text.contains(path.to_str().unwrap()));
        }
        fs::write(&path, b"broken = [").unwrap();
        assert!(file_source("system", &path, Format::Toml).status == Status::Malformed);
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_BYTES + 1).unwrap();
        drop(file);
        assert!(file_source("system", &path, Format::Toml).status == Status::Limit);
    }
    #[cfg(unix)]
    #[test]
    fn byte_identical_policy_replacement_is_a_new_source_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.toml");
        fs::write(&path, b"# empty managed document\n").unwrap();
        let first = file_source("system", &path, Format::Toml);
        let next = dir.path().join("new.toml");
        fs::write(&next, b"# empty managed document\n").unwrap();
        fs::rename(next, &path).unwrap();
        assert_ne!(
            first.digest,
            file_source("system", &path, Format::Toml).digest
        );
    }
    #[cfg(unix)]
    #[test]
    fn symlink_sources_are_unknown_and_never_followed_as_absence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.toml");
        std::os::unix::fs::symlink(dir.path().join("missing"), &path).unwrap();
        assert!(file_source("system", &path, Format::Toml).status == Status::Malformed);
    }
    #[test]
    fn registry_machine_presence_is_whole_hive_without_reading_value_data() {
        for (name, kind, length) in [
            ("", 1, 0),
            ("unknown", 1, 2),
            ("egressProxyUrl", 2, 20),
            ("disableAutoUpdates", 4, 4),
        ] {
            let values = BTreeMap::from([(name.into(), (kind, length))]);
            let machine = registry_observation("machine", &values, 5, 5);
            assert!(machine.1);
            let reads = std::cell::Cell::new(0);
            let result = choose_registry_sources(machine, || {
                reads.set(reads.get() + 1);
                registry_observation("user", &BTreeMap::new(), 1, 1)
            });
            assert_eq!(reads.get(), 0);
            assert_eq!(result[1].id, "claude_user_registry_shadowed");
        }
        for kind in [7, 11, 3] {
            let machine = registry_observation(
                "machine",
                &BTreeMap::from([("ignored".into(), (kind, 50))]),
                5,
                5,
            );
            assert!(!machine.1);
            let result = choose_registry_sources(machine, || {
                registry_observation(
                    "user",
                    &BTreeMap::from([("inferenceProvider".into(), (1, 14))]),
                    1,
                    1,
                )
            });
            assert_eq!(result[1].id, "user");
            assert!(result[1].effect == Effect::Managed);
        }
    }
    #[test]
    fn registry_errors_do_not_fall_back_and_lastwrite_changes_invalidate() {
        for status in [Status::Denied, Status::Changed, Status::Limit] {
            let sources =
                choose_registry_sources((Source::status("machine", status), false), || {
                    panic!("must not query user policy")
                });
            assert!(Snapshot::resolve(&target(), &sources, true).state == State::Unknown);
        }
        let values = BTreeMap::from([("inferenceProvider".into(), (1, 14))]);
        let (first, _) = registry_observation("machine", &values, 5, 5);
        let (changed, _) = registry_observation("machine", &values, 5, 6);
        assert!(changed.status == Status::Changed);
        let (second, _) = registry_observation("machine", &values, 6, 6);
        assert_ne!(first.digest, second.digest);
    }
}
