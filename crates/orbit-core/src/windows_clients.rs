//! Windows installation discovery shared by refresh, preview and commit.
//! Both Store/MSIX and signed traditional desktop layouts use the same writer.
use super::*;
use std::os::windows::ffi::OsStrExt;
use windows_sys::Win32::{Foundation::FreeLibrary, System::LibraryLoader::*};

#[derive(Clone, serde::Deserialize)]
struct Installation {
    identity: String,
    version: String,
    path: PathBuf,
    executable: PathBuf,
    cli: Option<PathBuf>,
    cli_version: Option<String>,
}
#[derive(Clone)]
pub(super) struct CachedInstallation {
    installation: Installation,
    registration: String,
    materials: Vec<String>,
}
fn registration(client: ClientKind) -> Option<String> {
    use windows_sys::Win32::Storage::Packaging::Appx::GetPackagesByPackageFamily;
    let family = if client == ClientKind::Codex {
        "OpenAI.Codex_2p2nqsd0c76g0"
    } else {
        "Claude_pzs8sxrjxfjjc"
    };
    let family = family.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let (mut count, mut length) = (0, 0);
    let code = unsafe {
        GetPackagesByPackageFamily(
            family.as_ptr(),
            &mut count,
            std::ptr::null_mut(),
            &mut length,
            std::ptr::null_mut(),
        )
    };
    if code == 0 && count == 0 {
        return Some("absent".into());
    }
    if code != 122 || count > 64 || length > 65536 || length == 0 {
        return None;
    }
    let mut names = vec![std::ptr::null_mut(); count as usize];
    let mut buffer = vec![0u16; length as usize];
    if unsafe {
        GetPackagesByPackageFamily(
            family.as_ptr(),
            &mut count,
            names.as_mut_ptr(),
            &mut length,
            buffer.as_mut_ptr(),
        )
    } != 0
        || count as usize > names.len()
        || length as usize > buffer.len()
    {
        return None;
    }
    let start = buffer.as_ptr() as usize;
    let end = start + buffer.len() * 2;
    let mut packages = Vec::new();
    for name in names.into_iter().take(count as usize) {
        let address = name as usize;
        if address < start || address >= end || (address - start) % 2 != 0 {
            return None;
        }
        let chars = &buffer[(address - start) / 2..];
        let len = chars.iter().position(|c| *c == 0)?;
        packages.push(String::from_utf16(&chars[..len]).ok()?);
    }
    packages.sort();
    Some(crate::identity::sha256(
        &serde_json::to_vec(&packages).ok()?,
    ))
}
fn installation_materials(installed: &Installation) -> Result<Vec<String>> {
    let mut stamps = vec![
        material(&installed.executable)?,
        material(&installed.path.join("resources/app.asar"))?,
    ];
    if let Some(cli) = &installed.cli {
        stamps.push(material(cli)?);
    }
    if installed
        .path
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.starts_with("app-"))
    {
        stamps.push(material_directory(
            installed
                .path
                .parent()
                .ok_or_else(|| fail("invalid_path"))?,
        )?);
    }
    Ok(stamps)
}
fn metadata(
    paths: &SystemPaths,
    client: ClientKind,
    candidate: Option<&str>,
) -> Option<Installation> {
    let registered = registration(client)?;
    let cached = paths
        .windows_installation_cache
        .lock()
        .ok()?
        .get(client.as_str())
        .cloned();
    if let Some(cached) = cached {
        if cached.registration == registered
            && installation_materials(&cached.installation).ok().as_ref() == Some(&cached.materials)
        {
            return Some(cached.installation);
        }
    }
    let installed = probe(client, candidate)?;
    let materials = installation_materials(&installed).ok()?;
    if registration(client)? != registered {
        return None;
    }
    paths.windows_installation_cache.lock().ok()?.insert(
        client.as_str().into(),
        CachedInstallation {
            installation: installed.clone(),
            registration: registered,
            materials,
        },
    );
    Some(installed)
}
const PROBE: &str = r#"
$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);
$codex=$env:ORBIT_PROBE_CLIENT -eq 'codex';
$name=if($codex){'OpenAI.Codex'}else{'Claude'};
$family=if($codex){'OpenAI.Codex_2p2nqsd0c76g0'}else{'Claude_pzs8sxrjxfjjc'};
$leaf=if($codex){'Codex.exe'}else{'Claude.exe'};
$identity=if($codex){'OpenAI.Codex'}else{'com.anthropic.claudefordesktop'};
$signer=if($codex){'OpenAI OpCo, LLC'}else{'Anthropic, PBC'};
function Signed($p) {
 if(!(Test-Path -LiteralPath $p -PathType Leaf)){return $false};
 $s=Get-AuthenticodeSignature -LiteralPath $p;
 return $s.Status -eq 'Valid' -and $s.SignerCertificate.GetNameInfo([Security.Cryptography.X509Certificates.X509NameType]::SimpleName,$false) -eq $signer;
}
$candidates=[Collections.Generic.List[string]]::new();
$p=Get-AppxPackage -Name $name -ErrorAction SilentlyContinue | Where-Object { $_.PackageFamilyName -eq $family -and $_.Status -eq 'Ok' -and $_.Architecture -eq 'X64' } | Select-Object -First 1;
if($env:ORBIT_PROBE_PATH){
 $candidate=$env:ORBIT_PROBE_PATH;
 if((Split-Path -Leaf $candidate) -like 'app-*'){$candidate=Split-Path -Parent $candidate};
 if($candidate -notlike '*\WindowsApps\*' -or ($p -and $candidate -eq (Join-Path $p.InstallLocation 'app'))){$candidates.Add($candidate)}
};
if($p){$candidates.Add((Join-Path $p.InstallLocation 'app'))};
$dirs=if($codex){@('Programs\Codex','Codex','Programs\ChatGPT','OpenAI\Codex')}else{@('AnthropicClaude','Programs\Claude')};
foreach($d in $dirs){if($env:LOCALAPPDATA){$candidates.Add((Join-Path $env:LOCALAPPDATA $d))}};
foreach($root in @($env:ProgramFiles,${env:ProgramFiles(x86)})) { if($root){$candidates.Add((Join-Path $root $(if($codex){'Codex'}else{'Claude'})))}};
foreach($registry in @('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*')) {
 foreach($r in @(Get-ItemProperty $registry -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -eq $(if($codex){'Codex'}else{'Claude'}) } | Select-Object -First 8)) {
  if($r.InstallLocation){$candidates.Add($r.InstallLocation)};
 }
};
$expanded=[Collections.Generic.List[string]]::new();
foreach($d in @($candidates | Select-Object -Unique)) {
 $expanded.Add($d);
 foreach($a in @(Get-ChildItem -LiteralPath $d -Directory -Filter 'app-*' -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '^app-[0-9]+(?:\.[0-9]+){1,3}(?:[-+][0-9A-Za-z.-]+)?$' } | Sort-Object { [version](($_.Name -replace '^app-','') -replace '[-+].*$','') } -Descending | Select-Object -First 8)){ $expanded.Add($a.FullName) };
};
foreach($d in @($expanded | Select-Object -Unique)) {
 $exe=Join-Path $d $leaf;
 if(!(Test-Path -LiteralPath (Join-Path $d 'resources\app.asar') -PathType Leaf) -or !(Signed $exe)){continue};
 $cli=$null; $cv=$null;
 if($codex){
  $cli=Join-Path $d 'resources\codex.exe';
  if(!(Signed $cli)){continue};
  $cv=(& $cli --version 2>$null | Out-String).Trim();
  if($LASTEXITCODE -ne 0 -or $cv -notmatch '^codex-cli ([0-9][0-9A-Za-z.+-]{0,100})$'){continue};
  $cv=$Matches[1];
 };
 $v=(Get-Item -LiteralPath $exe).VersionInfo.ProductVersion;
 if(!$v -and $p){$v=[string]$p.Version};
 if(!$v){continue};
 [pscustomobject]@{identity=$identity;version=$v;path=$d;executable=$exe;cli=$cli;cli_version=$cv}|ConvertTo-Json -Compress;
 exit 0;
}
exit 1;
"#;
fn probe(client: ClientKind, candidate: Option<&str>) -> Option<Installation> {
    let root = PathBuf::from(std::env::var_os("SystemRoot")?);
    let mut command = Command::new(root.join("System32/WindowsPowerShell/v1.0/powershell.exe"));
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            PROBE,
        ])
        .env("ORBIT_PROBE_CLIENT", client.as_str());
    if let Some(path) = candidate {
        command.env("ORBIT_PROBE_PATH", path);
    } else {
        command.env_remove("ORBIT_PROBE_PATH");
    }
    serde_json::from_slice(&bounded_command_with_timeout(
        command,
        Duration::from_secs(30),
    )?)
    .ok()
}
pub(super) fn refresh(paths: &SystemPaths, target: &mut ClientView) {
    if let Some(installed) = metadata(paths, target.client, target.app_path.as_deref()) {
        target.installed = true;
        target.version = Some(installed.version);
        target.bundle_id = Some(installed.identity);
        target.app_path = Some(installed.path.display().to_string());
    } else {
        target.installed = false;
        target.version = None;
        target.bundle_id = None;
        target.app_path = None;
    }
}
pub(super) fn verify(target: &ClientView, cli_version: Option<&str>) -> Result<()> {
    if target.client == ClientKind::Codex
        && let (Some(path), Some(app), Some(cli)) = (
            target.app_path.as_deref(),
            target.version.as_deref(),
            cli_version,
        )
    {
        return CodexVerificationScope::verify(path, app, cli, || {
            verify_installation(target, cli_version)
        });
    }
    verify_installation(target, cli_version).map(drop)
}

fn verify_installation(target: &ClientView, cli_version: Option<&str>) -> Result<Vec<fs::File>> {
    let locks = lock_installation(target)?;
    let root = std::env::var_os("SystemRoot")
        .ok_or_else(|| fail("codex_configuration_target_unverified"))?;
    let mut command =
        Command::new(PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe"));
    const SCRIPT: &str = r#"$ErrorActionPreference='Stop';
$codex=$env:ORBIT_VERIFY_CLIENT -eq 'codex';
$signer=if($codex){'OpenAI OpCo, LLC'}else{'Anthropic, PBC'};
$exe=Join-Path $env:ORBIT_VERIFY_PATH $(if($codex){'Codex.exe'}else{'Claude.exe'});
$files=@($exe); if($codex){$files+=Join-Path $env:ORBIT_VERIFY_PATH 'resources\codex.exe'};
foreach($file in $files){$s=Get-AuthenticodeSignature -LiteralPath $file; if($s.Status -ne 'Valid' -or $s.SignerCertificate.GetNameInfo([Security.Cryptography.X509Certificates.X509NameType]::SimpleName,$false) -ne $signer){exit 1}};
if(!$codex -and (Get-Item -LiteralPath $exe).VersionInfo.ProductVersion -ne $env:ORBIT_VERIFY_VERSION){exit 1};
if($codex){$actual=(& $files[1] --version 2>$null | Out-String).Trim(); if($actual -ne ('codex-cli '+$env:ORBIT_VERIFY_CLI_VERSION)){exit 1}};
[Console]::Write('ORBIT_SIGNATURE_OK');"#;
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            SCRIPT,
        ])
        .env("ORBIT_VERIFY_CLIENT", target.client.as_str())
        .env(
            "ORBIT_VERIFY_PATH",
            target
                .app_path
                .as_deref()
                .ok_or_else(|| fail("codex_configuration_target_unverified"))?,
        )
        .env(
            "ORBIT_VERIFY_VERSION",
            target.version.as_deref().unwrap_or_default(),
        )
        .env("ORBIT_VERIFY_CLI_VERSION", cli_version.unwrap_or_default());
    if bounded_command_with_timeout(command, Duration::from_secs(30)).as_deref()
        != Some(b"ORBIT_SIGNATURE_OK")
    {
        return Err(fail("codex_configuration_target_unverified"));
    }
    let app = Path::new(
        target
            .app_path
            .as_deref()
            .ok_or_else(|| fail("codex_configuration_target_unverified"))?,
    );
    if target.client == ClientKind::Claude {
        let expected = asar_header_hash(&app.join("Claude.exe"))?;
        crate::installed_asar::claude_features(
            configuration_file(&app.join("resources/app.asar"))?,
            &expected,
        )?;
    } else {
        crate::client_compatibility::observe(
            configuration_file(&app.join("resources/codex.exe"))?,
            crate::client_compatibility::CODEX_FEATURES,
            1024 * 1024 * 1024,
        )?;
    }
    Ok(locks)
}
fn qualify(paths: &SystemPaths, target: &ClientView) -> Result<Installation> {
    let installed = metadata(paths, target.client, target.app_path.as_deref())
        .ok_or_else(|| fail("codex_configuration_target_unverified"))?;
    if target.app_path.as_ref() != Some(&installed.path.display().to_string())
        || target.version.as_ref() != Some(&installed.version)
        || target.bundle_id.as_ref() != Some(&installed.identity)
    {
        return Err(fail("codex_configuration_target_changed"));
    }
    Ok(installed)
}
fn traced<T>(result: Result<T>, stage: &str) -> Result<T> {
    #[cfg(test)]
    if result.is_err() {
        eprintln!("Windows qualification failed at {stage}");
    }
    #[cfg(not(test))]
    let _ = stage;
    result
}
fn installation_scope(path: &Path, identity: &str) -> String {
    let package = match identity {
        "OpenAI.Codex" => ("OpenAI.Codex_", "_x64__2p2nqsd0c76g0"),
        _ => ("Claude_", "_x64__pzs8sxrjxfjjc"),
    };
    let parent = path.parent();
    let package_folder = parent.and_then(Path::file_name).and_then(|s| s.to_str());
    let scope = if path
        .file_name()
        .is_some_and(|s| s.eq_ignore_ascii_case("app"))
        && package_folder.is_some_and(|n| n.starts_with(package.0) && n.ends_with(package.1))
    {
        ("msix", parent.and_then(Path::parent).unwrap_or(path))
    } else if path.file_name().and_then(|s| s.to_str()).is_some_and(|n| {
        n.strip_prefix("app-")
            .is_some_and(|v| !v.is_empty() && v.bytes().all(|c| c.is_ascii_digit() || c == b'.'))
    }) {
        ("squirrel", parent.unwrap_or(path))
    } else {
        ("directory", path)
    };
    crate::identity::sha256(&serde_json::to_vec(&(identity, scope.0, scope.1)).unwrap_or_default())
}
fn material(path: &Path) -> Result<String> {
    crate::windows_fs::stamp(&configuration_file(path)?)
}
fn lock_installation(target: &ClientView) -> Result<Vec<fs::File>> {
    let root = Path::new(
        target
            .app_path
            .as_deref()
            .ok_or_else(|| fail("codex_configuration_target_unverified"))?,
    );
    let mut locks = crate::windows_fs::lock_directory(&root.join("resources"))?;
    for relative in [
        if target.client == ClientKind::Codex {
            "Codex.exe"
        } else {
            "Claude.exe"
        },
        "resources/app.asar",
    ] {
        locks.push(crate::windows_fs::lock_installed_file(
            &root.join(relative),
        )?);
    }
    if target.client == ClientKind::Codex {
        locks.push(crate::windows_fs::lock_installed_file(
            &root.join("resources/codex.exe"),
        )?);
    }
    Ok(locks)
}
fn root_anchor(path: &Path) -> Result<ConfigurationRootAnchor> {
    Ok(ConfigurationRootAnchor {
        device: 0,
        inode: 0,
        owner: 0,
        windows: Some(crate::windows_fs::identity(
            &configuration_directory(path)?,
            true,
        )?),
    })
}
pub(super) fn codex_root(root: &Path) -> Result<(fs::File, ConfigurationRootAnchor)> {
    if root.file_name().is_none_or(|n| n != ".codex") {
        return Err(fail("codex_configuration_target_unverified"));
    }
    let home = root
        .parent()
        .ok_or_else(|| fail("codex_configuration_target_unverified"))?;
    let anchor = root_anchor(home)?;
    match fs::symlink_metadata(root) {
        Ok(_) => {
            root_anchor(root)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(fail("codex_configuration_target_unverified")),
    }
    if root_anchor(home)? != anchor {
        return Err(fail("codex_configuration_target_changed"));
    }
    Ok((configuration_directory(home)?, anchor))
}
pub(super) fn claude_root(root: &Path) -> Result<Vec<ConfigurationRootAnchor>> {
    // The client selects LocalAppData, including on relocated profiles.
    // Unit fixtures use their own private parent/Claude-3p scope.
    if root.file_name().is_none_or(|n| n != "Claude-3p") {
        return Err(fail("claude_configuration_target_unverified"));
    }
    let parent = root
        .parent()
        .ok_or_else(|| fail("claude_configuration_target_unverified"))?;
    let anchor = root_anchor(parent).map_err(claude_target_error)?;
    for leaf in [root.to_path_buf(), root.join("configLibrary")] {
        match fs::symlink_metadata(&leaf) {
            Ok(_) => {
                root_anchor(&leaf).map_err(claude_target_error)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(fail("claude_configuration_target_unverified")),
        }
    }
    if root_anchor(parent).map_err(claude_target_error)? != anchor {
        return Err(fail("claude_configuration_target_changed"));
    }
    Ok(vec![anchor])
}
pub(super) fn codex_draft(
    paths: &SystemPaths,
    target: &ClientView,
) -> Result<(CodexConfigurationTarget, Vec<String>)> {
    if paths.isolated
        || !target.installed
        || !target.platform_supported
        || target.client != ClientKind::Codex
        || target.bundle_id.as_deref() != Some("OpenAI.Codex")
    {
        return Err(fail("codex_configuration_target_unverified"));
    }
    let _installed_locks = traced(lock_installation(target), "Codex installed file locks")?;
    let installed = traced(qualify(paths, target), "Codex fresh signature and identity")?;
    let home = paths
        .codex_user_home
        .as_ref()
        .ok_or_else(|| fail("codex_configuration_target_unverified"))?;
    let root = home.join(".codex");
    if paths.codex_config != root.join("config.toml") || !paths.configuration_roots.contains(&root)
    {
        return Err(fail("codex_configuration_target_unverified"));
    }
    let (_, root_anchor) = traced(codex_root(&root), "Codex private default root")?;
    let cli = installed
        .cli
        .as_ref()
        .ok_or_else(|| fail("codex_configuration_target_unverified"))?;
    let materials = vec![material(&installed.executable)?, material(cli)?];
    let features =
        paths.configuration_features(cli, crate::client_compatibility::CODEX_FEATURES)?;
    let cli_version = installed
        .cli_version
        .ok_or_else(|| fail("codex_configuration_target_unverified"))?;
    let revision = crate::identity::sha256(
        &serde_json::to_vec(&(
            1u32,
            &installed.path,
            &installed.identity,
            &installed.version,
            &cli_version,
            features,
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
            installation_scope: installation_scope(&installed.path, &installed.identity),
            app_path: installed.path.display().to_string(),
            bundle_id: installed.identity,
            desktop_version: installed.version,
            cli_version,
            profile_id: crate::codex_binding::PROFILE_ID.into(),
            config_root: root,
            root_anchor,
            revision,
        },
        materials,
    ))
}
fn asar_header_hash(executable: &Path) -> Result<String> {
    let bad = || fail("claude_configuration_target_unverified");
    let path = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // LOAD_LIBRARY_AS_DATAFILE_EXCLUSIVE never executes the inspected program.
    let module = unsafe {
        LoadLibraryExW(
            path.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_AS_DATAFILE_EXCLUSIVE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
        )
    };
    if module.is_null() {
        return Err(bad());
    }
    struct Module(windows_sys::Win32::Foundation::HMODULE);
    impl Drop for Module {
        fn drop(&mut self) {
            unsafe {
                FreeLibrary(self.0);
            }
        }
    }
    let module = Module(module);
    for (kind, name) in [("INTEGRITY", "ELECTRONASAR"), ("Integrity", "ElectronAsar")] {
        let kind = kind.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let name = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let resource = unsafe { FindResourceW(module.0, name.as_ptr(), kind.as_ptr()) };
        if resource.is_null() {
            continue;
        }
        let size = unsafe { SizeofResource(module.0, resource) };
        if size == 0 || size > 65536 {
            return Err(bad());
        }
        let loaded = unsafe { LoadResource(module.0, resource) };
        let data = unsafe { LockResource(loaded) };
        if data.is_null() {
            return Err(bad());
        }
        let entries: serde_json::Value = serde_json::from_slice(unsafe {
            std::slice::from_raw_parts(data.cast::<u8>(), size as usize)
        })
        .map_err(|_| bad())?;
        let hashes = entries
            .as_array()
            .ok_or_else(bad)?
            .iter()
            .filter(|e| {
                e["file"].as_str().is_some_and(|f| {
                    f.replace('\\', "/")
                        .eq_ignore_ascii_case("resources/app.asar")
                }) && e["alg"]
                    .as_str()
                    .is_some_and(|a| a.eq_ignore_ascii_case("sha256"))
            })
            .collect::<Vec<_>>();
        if hashes.len() != 1 {
            return Err(bad());
        }
        return hashes[0]["value"]
            .as_str()
            .filter(|h| configuration_digest(h))
            .map(str::to_owned)
            .ok_or_else(bad);
    }
    Err(bad())
}
pub(super) fn claude_draft(
    paths: &SystemPaths,
    target: &ClientView,
) -> Result<ClaudeConfigurationTarget> {
    let bad = || fail("claude_configuration_target_unverified");
    if paths.isolated
        || !target.installed
        || !target.platform_supported
        || target.client != ClientKind::Claude
        || target.bundle_id.as_deref() != Some(CLAUDE_BUNDLE_ID)
        || std::env::var_os("CLAUDE_USER_DATA_DIR").is_some()
        || std::env::var_os("CLAUDE_E2E_MANAGED_PLIST").is_some()
    {
        return Err(bad());
    }
    let _installed_locks = lock_installation(target).map_err(claude_target_error)?;
    let installed = qualify(paths, target).map_err(claude_target_error)?;
    let root = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or_else(bad)?).join("Claude-3p");
    if paths.claude_library != root.join("configLibrary")
        || !paths.configuration_roots.contains(&root)
    {
        return Err(bad());
    }
    let root_anchors = claude_root(&root)?;
    let asar = installed.path.join("resources/app.asar");
    let materials = vec![
        material(&installed.executable).map_err(claude_target_error)?,
        material(&asar).map_err(claude_target_error)?,
    ];
    let stamp = materials.join("|");
    let source_digest = {
        let cached = paths
            .capability_cache
            .lock()
            .map_err(|_| bad())?
            .get(&asar)
            .filter(|(s, _)| *s == stamp)
            .map(|(_, d)| d.clone());
        if let Some(digest) = cached {
            digest
        } else {
            let hash = asar_header_hash(&installed.executable)?;
            let digest = crate::installed_asar::claude_features(
                configuration_file(&asar).map_err(claude_target_error)?,
                &hash,
            )?;
            if material(&asar).map_err(claude_target_error)? != materials[1]
                || material(&installed.executable).map_err(claude_target_error)? != materials[0]
            {
                return Err(fail("claude_configuration_target_changed"));
            }
            paths
                .capability_cache
                .lock()
                .map_err(|_| bad())?
                .insert(asar, (stamp, digest.clone()));
            digest
        }
    };
    let revision = crate::identity::sha256(
        &serde_json::to_vec(&(
            "claude-windows-user-config-v1",
            &installed.path,
            &installed.identity,
            &installed.version,
            &source_digest,
            crate::claude_config::SCHEMA_KEYS_SHA256,
            &root,
            &root_anchors,
            &materials,
        ))
        .map_err(|_| fail("serialization_failed"))?,
    );
    Ok(ClaudeConfigurationTarget {
        contract_version: 1,
        installation_scope: installation_scope(&installed.path, &installed.identity),
        app_path: installed.path.display().to_string(),
        bundle_id: installed.identity,
        app_version: installed.version,
        source_member_sha256: source_digest,
        config_root: root,
        root_anchors,
        materials,
        revision,
    })
}
pub(super) fn migration_clear(root: &Path) -> Result<()> {
    let bad = || fail("claude_configuration_target_unverified");
    if !root.try_exists().map_err(|_| bad())? {
        return Ok(());
    }
    let _lock = crate::windows_fs::lock_directory(root).map_err(claude_target_error)?;
    let before =
        crate::windows_fs::stamp(&configuration_directory(root).map_err(claude_target_error)?)
            .map_err(claude_target_error)?;
    let mut paths = vec![(root.join("claude_desktop_config.json"), true)];
    let library = root.join("configLibrary");
    let _library_lock = if library.try_exists().map_err(|_| bad())? {
        Some(crate::windows_fs::lock_directory(&library).map_err(claude_target_error)?)
    } else {
        None
    };
    let library_before = _library_lock
        .as_ref()
        .map(|_| material_directory(&library))
        .transpose()?;
    if _library_lock.is_some() {
        let mut count = 0;
        for entry in fs::read_dir(&library).map_err(|_| bad())? {
            let entry = entry.map_err(|_| bad())?;
            count += 1;
            if count > MAX_CLAUDE_MIGRATION_ENTRIES {
                return Err(bad());
            }
            if entry
                .file_name()
                .to_str()
                .is_some_and(|n| claude_migration_profile_name(n.as_bytes()))
            {
                paths.push((entry.path(), false));
            }
        }
    }
    if paths.len() > crate::claude_config::MAX_ENTRIES + 1 {
        return Err(bad());
    }
    let mut total = 0;
    for (path, main) in paths {
        if main && !path.try_exists().map_err(|_| bad())? {
            continue;
        }
        let mut file = configuration_file(&path).map_err(claude_target_error)?;
        let stamp = crate::windows_fs::stamp(&file).map_err(claude_target_error)?;
        let limit = crate::claude_config::MAX_FILE_BYTES
            .min(MAX_CLAUDE_MIGRATION_BYTES.saturating_sub(total));
        if file.metadata().map_err(|_| bad())?.len() > limit as u64 {
            return Err(bad());
        }
        let mut bytes = zeroize::Zeroizing::new(Vec::new());
        Read::by_ref(&mut file)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| bad())?;
        if bytes.len() > limit || material(&path).map_err(claude_target_error)? != stamp {
            return Err(bad());
        }
        total += bytes.len();
        if crate::claude_config::native_migration_required(&bytes, main)? {
            return Err(fail("claude_native_migration_required"));
        }
    }
    if before != material_directory(root)?
        || library_before
            .as_ref()
            .is_some_and(|s| material_directory(&library).as_ref() != Ok(s))
    {
        return Err(fail("claude_configuration_target_changed"));
    }
    Ok(())
}
fn material_directory(path: &Path) -> Result<String> {
    crate::windows_fs::stamp(&configuration_directory(path).map_err(claude_target_error)?)
        .map_err(claude_target_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_installation_scope_survives_store_and_squirrel_version_directories() {
        let a =
            Path::new(r"C:\Program Files\WindowsApps\OpenAI.Codex_26.1.0.0_x64__2p2nqsd0c76g0\app");
        let b =
            Path::new(r"C:\Program Files\WindowsApps\OpenAI.Codex_27.2.0.0_x64__2p2nqsd0c76g0\app");
        assert_eq!(
            installation_scope(a, "OpenAI.Codex"),
            installation_scope(b, "OpenAI.Codex")
        );
        let a = Path::new(r"C:\Users\user\AppData\Local\AnthropicClaude\app-5.1.0");
        let b = Path::new(r"C:\Users\user\AppData\Local\AnthropicClaude\app-6.2.0");
        assert_eq!(
            installation_scope(a, CLAUDE_BUNDLE_ID),
            installation_scope(b, CLAUDE_BUNDLE_ID)
        );
        assert_ne!(
            installation_scope(a, CLAUDE_BUNDLE_ID),
            installation_scope(Path::new(r"C:\Elsewhere\app-6.2.0"), CLAUDE_BUNDLE_ID)
        );
    }
}
