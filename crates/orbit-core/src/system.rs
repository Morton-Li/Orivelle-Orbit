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
    #[cfg(test)]
    pub fixture_clients: Option<Vec<ClientView>>,
    #[cfg(test)]
    pub(crate) fixture_policy:
        Option<std::sync::Arc<dyn Fn(&ClientView) -> crate::policy::Snapshot + Send + Sync>>,
    #[cfg(test)]
    pub(crate) fixture_codex_runtime:
        Option<std::sync::Arc<dyn Fn() -> CodexRuntimeSnapshot + Send + Sync>>,
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
    #[cfg(test)]
    pub(crate) fn synthetic(marker: &str) -> Self {
        Self {
            identity: Some(CodexRuntimeIdentity {
                cli_version: crate::codex_catalog::SUPPORTED_CLI_VERSION.into(),
                revision: crate::identity::sha256(format!("synthetic-runtime:{marker}").as_bytes()),
            }),
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
            #[cfg(test)]
            fixture_clients: None,
            #[cfg(test)]
            fixture_policy: None,
            #[cfg(test)]
            fixture_codex_runtime: None,
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
            #[cfg(test)]
            fixture_clients: None,
            #[cfg(test)]
            fixture_policy: None,
            #[cfg(test)]
            fixture_codex_runtime: None,
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
