use crate::{
    dto::{UpdateProgress, UpdateView},
    error::{Result, fail},
    identity::{sha256, uuid},
    network::Http,
    store::{Store, private_directory, private_file},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use fs2::FileExt;
use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
    time::Duration,
};
use url::Url;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MAX_UPDATE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_EXPANDED_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_ARCHIVE_ENTRIES: usize = 20_000;
/// The default build has no channel or trust key. These public values are compiled by
/// an explicitly reviewed release build, never accepted from webview/settings/HTTP.
pub struct ReleaseTrust {
    pub public_key: &'static str,
    pub endpoint: &'static str,
    pub mac_certificate_sha256: Option<&'static str>,
}
pub fn compiled_trust() -> Option<ReleaseTrust> {
    if cfg!(target_os = "windows") {
        return None;
    }
    match (
        option_env!("ORBIT_RELEASE_UPDATER_PUBLIC_KEY"),
        option_env!("ORBIT_RELEASE_UPDATE_ENDPOINT"),
    ) {
        (Some(key), Some(endpoint))
            if !key.is_empty() && !key.contains("REQUIRED") && !endpoint.is_empty() =>
        {
            Some(ReleaseTrust {
                public_key: key,
                endpoint,
                mac_certificate_sha256: option_env!("ORBIT_RELEASE_MAC_CERTIFICATE_SHA256"),
            })
        }
        _ => None,
    }
}
pub fn target() -> Result<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Ok("darwin-aarch64")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Ok("windows-x86_64")
    } else {
        Err(fail("platform_unsupported"))
    }
}
pub fn disabled() -> UpdateView {
    UpdateView {
        status: "disabled".into(),
        current_version: CURRENT_VERSION.into(),
        version: None,
        reason: Some("当前版本需手动下载安装包更新".into()),
    }
}
fn version_parts(version: &str) -> Result<[u64; 3]> {
    let fields = version.split('.').collect::<Vec<_>>();
    if fields.len() != 3
        || fields.iter().any(|s| {
            s.is_empty()
                || s.len() > 10
                || !s.bytes().all(|b| b.is_ascii_digit())
                || (s.len() > 1 && s.starts_with('0'))
        })
    {
        return Err(fail("invalid_update_version"));
    }
    Ok([
        fields[0]
            .parse()
            .map_err(|_| fail("invalid_update_version"))?,
        fields[1]
            .parse()
            .map_err(|_| fail("invalid_update_version"))?,
        fields[2]
            .parse()
            .map_err(|_| fail("invalid_update_version"))?,
    ])
}
fn verified_version(comment: &str, expected: &str) -> Result<()> {
    let matches = comment
        .split('\t')
        .filter_map(|part| part.strip_prefix("version:"))
        .collect::<Vec<_>>();
    if matches.len() != 1 || matches[0] != expected {
        return Err(fail("update_version_signature_mismatch"));
    }
    version_parts(expected)?;
    Ok(())
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub version: String,
    pub target: String,
    pub url: String,
    pub signature: String,
    pub file_name: Option<String>,
    pub hash: Option<String>,
    pub verified: bool,
}
pub struct VerifiedArtifact {
    candidate: Candidate,
    bytes: Vec<u8>,
}
impl VerifiedArtifact {
    pub fn candidate(&self) -> &Candidate {
        &self.candidate
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

struct UpdateChannel {
    manifest: Url,
    github_repository: Option<String>,
}

const GITHUB_MANIFEST_SUFFIX: &str = "/releases/latest/download/latest.json";

impl UpdateChannel {
    fn new(http: &Http, endpoint: &str) -> Result<Self> {
        let manifest = http.validate_url(endpoint)?;
        if manifest.query().is_some() {
            return Err(fail("invalid_update_channel"));
        }
        let github_repository = if manifest.host_str() == Some("github.com") {
            let repository = manifest
                .path()
                .strip_suffix(GITHUB_MANIFEST_SUFFIX)
                .and_then(|prefix| prefix.strip_prefix('/'))
                .and_then(|name| name.split_once('/'))
                .filter(|(owner, repo)| release_segment(owner) && release_segment(repo))
                .filter(|_| manifest.scheme() == "https" && manifest.port().is_none())
                .ok_or_else(|| fail("invalid_update_channel"))?;
            Some(format!("/{}/{}", repository.0, repository.1))
        } else {
            None
        };
        Ok(Self {
            manifest,
            github_repository,
        })
    }

    fn artifact_url(&self, http: &Http, raw: &str, version: &str) -> Result<Url> {
        let url = http.validate_url(raw)?;
        if url.origin() != self.manifest.origin() || url.query().is_some() {
            return Err(fail("invalid_update_artifact_url"));
        }
        if let Some(repository) = &self.github_repository
            && !github_release_asset(&url, repository).is_some_and(|(tag, asset)| {
                tag.strip_prefix('v') == Some(version) && asset.ends_with(".app.tar.gz")
            })
        {
            return Err(fail("invalid_update_artifact_url"));
        }
        Ok(url)
    }

    async fn get(
        &self,
        http: &Http,
        url: &Url,
        version: Option<&str>,
        accept: &str,
        limit: u64,
        timeout: Duration,
        progress: Option<&(dyn Fn(u64, Option<u64>) + Send + Sync)>,
    ) -> Result<crate::network::RemoteResponse> {
        if let Some(repository) = &self.github_repository {
            let asset = url
                .path_segments()
                .and_then(Iterator::last)
                .ok_or_else(|| fail("invalid_update_channel"))?;
            http.get_update(
                url.clone(),
                accept,
                limit,
                timeout,
                |from, to| github_update_redirect(from, to, repository, asset, version),
                progress,
            )
            .await
        } else {
            http.get_update(url.clone(), accept, limit, timeout, |_, _| false, progress)
                .await
        }
    }
}

fn release_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 200
        && !matches!(segment, "." | "..")
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn github_release_asset<'a>(url: &'a Url, repository: &str) -> Option<(&'a str, &'a str)> {
    if url.host_str() != Some("github.com")
        || url.port().is_some()
        || url.query().is_some()
        || !url
            .path()
            .get(..repository.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(repository))
    {
        return None;
    }
    let (tag, asset) = url.path()[repository.len()..]
        .strip_prefix("/releases/download/")?
        .split_once('/')?;
    (version_parts(tag.strip_prefix('v')?).is_ok() && release_segment(asset))
        .then_some((tag, asset))
}

fn github_update_redirect(
    from: &Url,
    to: &Url,
    repository: &str,
    asset: &str,
    version: Option<&str>,
) -> bool {
    if from.host_str() != Some("github.com")
        || to.scheme() != "https"
        || to.port().is_some()
        || !to.username().is_empty()
        || to.password().is_some()
        || to.fragment().is_some()
    {
        return false;
    }
    match to.host_str() {
        Some("github.com") => github_release_asset(to, repository).is_some_and(|(tag, name)| {
            name == asset && version.is_none_or(|expected| tag.strip_prefix('v') == Some(expected))
        }),
        Some("release-assets.githubusercontent.com" | "objects.githubusercontent.com") => matches!(
            to.path_segments().and_then(|mut segments| segments.next()),
            Some("github-production-release-asset" | "github-production-release-asset-2e65be")
        ),
        _ => false,
    }
}

pub async fn check(http: &Http, store: &Store) -> Result<UpdateView> {
    let Some(trust) = compiled_trust() else {
        return Ok(disabled());
    };
    let channel = UpdateChannel::new(http, trust.endpoint)?;
    let response = channel
        .get(
            http,
            &channel.manifest,
            None,
            "application/json",
            64 * 1024,
            Duration::from_secs(15),
            None,
        )
        .await?;
    if response.status == 204 {
        return Ok(UpdateView {
            status: "up_to_date".into(),
            current_version: CURRENT_VERSION.into(),
            version: None,
            reason: None,
        });
    }
    if response.status != 200 {
        return Err(fail("update_channel_unavailable"));
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&response.bytes).map_err(|_| fail("invalid_update_manifest"))?;
    let version = manifest["version"]
        .as_str()
        .ok_or_else(|| fail("invalid_update_manifest"))?;
    if version_parts(version)? <= version_parts(CURRENT_VERSION)? {
        return Ok(UpdateView {
            status: "up_to_date".into(),
            current_version: CURRENT_VERSION.into(),
            version: None,
            reason: None,
        });
    }
    let target = target()?;
    let platform = &manifest["platforms"][target];
    let download = platform["url"]
        .as_str()
        .ok_or_else(|| fail("invalid_update_manifest"))?;
    let download_url = channel.artifact_url(http, download, version)?;
    let signature = platform["signature"]
        .as_str()
        .filter(|v| v.len() <= 16 * 1024)
        .ok_or_else(|| fail("invalid_update_manifest"))?;
    Signature::decode(&decode_signature_box(signature)?)
        .map_err(|_| fail("invalid_update_signature"))?;
    let candidate = Candidate {
        version: version.into(),
        target: target.into(),
        url: download_url.into(),
        signature: signature.into(),
        file_name: None,
        hash: None,
        verified: false,
    };
    store.put("update-candidate", version, "local", &candidate)?;
    Ok(UpdateView {
        status: "available".into(),
        current_version: CURRENT_VERSION.into(),
        version: Some(version.into()),
        reason: None,
    })
}
pub fn verify_artifact(
    public_key: &str,
    signature: &str,
    version: &str,
    bytes: &[u8],
) -> Result<()> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_UPDATE_BYTES {
        return Err(fail("update_resource_limit"));
    }
    let key_bytes = STANDARD
        .decode(public_key)
        .map_err(|_| fail("invalid_update_trust_key"))?;
    let key_text = std::str::from_utf8(&key_bytes).map_err(|_| fail("invalid_update_trust_key"))?;
    let key = PublicKey::decode(key_text).map_err(|_| fail("invalid_update_trust_key"))?;
    let signature = Signature::decode(&decode_signature_box(signature)?)
        .map_err(|_| fail("invalid_update_signature"))?;
    key.verify(bytes, &signature, false)
        .map_err(|_| fail("invalid_update_signature"))?;
    verified_version(signature.trusted_comment(), version)
}
fn decode_signature_box(encoded: &str) -> Result<String> {
    if encoded.len() > 16 * 1024 {
        return Err(fail("invalid_update_signature"));
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| fail("invalid_update_signature"))?;
    String::from_utf8(bytes).map_err(|_| fail("invalid_update_signature"))
}
pub async fn download(
    http: &Http,
    store: &Store,
    version: &str,
    confirmed: bool,
    request_id: &str,
    progress: &(dyn Fn(UpdateProgress) + Send + Sync),
) -> Result<UpdateView> {
    if !confirmed {
        return Err(fail("confirmation_required"));
    }
    let Some(trust) = compiled_trust() else {
        return Ok(disabled());
    };
    let mut candidate = store
        .get::<Candidate>("update-candidate", version)?
        .ok_or_else(|| fail("update_not_available"))?;
    if candidate.target != target()?
        || candidate.version != version
        || version_parts(version)? <= version_parts(CURRENT_VERSION)?
    {
        return Err(fail("update_target_mismatch"));
    }
    let channel = UpdateChannel::new(http, trust.endpoint)?;
    let url = channel.artifact_url(http, &candidate.url, version)?;
    let downloaded = |downloaded_bytes, total_bytes| {
        progress(UpdateProgress {
            request_id: request_id.into(),
            phase: "downloading".into(),
            downloaded_bytes,
            total_bytes,
        })
    };
    let response = channel
        .get(
            http,
            &url,
            Some(version),
            "application/octet-stream",
            MAX_UPDATE_BYTES,
            Duration::from_secs(15 * 60),
            Some(&downloaded),
        )
        .await?;
    if response.status != 200 {
        return Err(fail("update_download_failed"));
    }
    progress(UpdateProgress {
        request_id: request_id.into(),
        phase: "preparing".into(),
        downloaded_bytes: response.bytes.len() as u64,
        total_bytes: Some(response.bytes.len() as u64),
    });
    verify_artifact(
        trust.public_key,
        &candidate.signature,
        &candidate.version,
        &response.bytes,
    )?;
    let name = format!("{}.artifact", uuid());
    let path = store.root.join("updates").join(&name);
    crate::config::atomic_write(&path, &response.bytes)?;
    candidate.hash = Some(sha256(&response.bytes));
    candidate.file_name = Some(name);
    candidate.verified = true;
    store.put("update-candidate", version, "local", &candidate)?;
    Ok(UpdateView {
        status: "verified".into(),
        current_version: CURRENT_VERSION.into(),
        version: Some(version.into()),
        reason: None,
    })
}
pub fn verified_local_candidate(store: &Store, version: &str) -> Result<VerifiedArtifact> {
    let Some(trust) = compiled_trust() else {
        return Err(fail("update_disabled"));
    };
    let candidate = store
        .get::<Candidate>("update-candidate", version)?
        .ok_or_else(|| fail("update_not_available"))?;
    if !candidate.verified || candidate.target != target()? || candidate.version != version {
        return Err(fail("update_not_verified"));
    }
    let name = candidate
        .file_name
        .as_ref()
        .filter(|name| !name.contains('/') && !name.contains('\\') && !name.contains(".."))
        .ok_or_else(|| fail("invalid_update_artifact"))?;
    let path = store.root.join("updates").join(name);
    let bytes = crate::images::bounded_read(&path, MAX_UPDATE_BYTES)?;
    if candidate.hash.as_deref() != Some(&sha256(&bytes)) {
        return Err(fail("update_artifact_changed"));
    }
    verify_artifact(trust.public_key, &candidate.signature, version, &bytes)?;
    Ok(VerifiedArtifact { candidate, bytes })
}
/// Helpers hold a shared lock for their lifetime; update and helper replacement require
/// the exclusive lock. The fixed helper path remains independent of app bundle versions.
pub fn exclusive_helper_lock(root: &Path) -> Result<File> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("locks/helper-update.lock"))
        .map_err(|_| fail("update_busy"))?;
    private_file(&root.join("locks/helper-update.lock"))?;
    lock.try_lock_exclusive().map_err(|_| fail("update_busy"))?;
    Ok(lock)
}
pub fn extract_macos_artifact(artifact: &VerifiedArtifact, root: &Path) -> Result<PathBuf> {
    private_directory(root)?;
    if fs::read_dir(root)
        .map_err(|_| fail("update_extract_failed"))?
        .next()
        .is_some()
    {
        return Err(fail("update_extract_conflict"));
    }
    let decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(artifact.bytes()))
        .take(MAX_EXPANDED_BYTES + 1);
    let mut archive = tar::Archive::new(decoder);
    let mut count = 0usize;
    let mut expanded = 0u64;
    let mut bundle = None;
    for item in archive
        .entries()
        .map_err(|_| fail("update_extract_failed"))?
    {
        let mut entry = item.map_err(|_| fail("update_extract_failed"))?;
        count += 1;
        if count > MAX_ARCHIVE_ENTRIES {
            return Err(fail("update_resource_limit"));
        }
        let path = entry
            .path()
            .map_err(|_| fail("update_extract_failed"))?
            .into_owned();
        let mut parts = path.components();
        let Some(Component::Normal(first)) = parts.next() else {
            return Err(fail("update_archive_path_invalid"));
        };
        if !first.to_string_lossy().ends_with(".app")
            || parts.clone().any(|p| !matches!(p, Component::Normal(_)))
        {
            return Err(fail("update_archive_path_invalid"));
        }
        if let Some(existing) = &bundle {
            if existing != first {
                return Err(fail("update_archive_path_invalid"));
            }
        } else {
            bundle = Some(first.to_os_string());
        }
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() && !kind.is_symlink() {
            return Err(fail("update_archive_path_invalid"));
        }
        expanded = expanded
            .checked_add(entry.size())
            .ok_or_else(|| fail("update_resource_limit"))?;
        if expanded > MAX_EXPANDED_BYTES || entry.size() > MAX_UPDATE_BYTES {
            return Err(fail("update_resource_limit"));
        }
        if kind.is_symlink() {
            let link = entry
                .link_name()
                .map_err(|_| fail("update_archive_path_invalid"))?
                .ok_or_else(|| fail("update_archive_path_invalid"))?;
            if link.is_absolute() {
                return Err(fail("update_archive_path_invalid"));
            }
            let mut components = path
                .parent()
                .ok_or_else(|| fail("update_archive_path_invalid"))?
                .components()
                .collect::<Vec<_>>();
            for part in link.components() {
                match part {
                    Component::Normal(_) => components.push(part),
                    Component::CurDir => {}
                    Component::ParentDir => {
                        if components.len() <= 1 {
                            return Err(fail("update_archive_path_invalid"));
                        }
                        components.pop();
                    }
                    _ => return Err(fail("update_archive_path_invalid")),
                }
            }
        }
        let destination = root.join(&path);
        let mut ancestor = destination.parent();
        while let Some(parent) = ancestor {
            if parent == root {
                break;
            }
            if fs::symlink_metadata(parent).is_ok_and(|m| m.file_type().is_symlink()) {
                let canonical = parent
                    .canonicalize()
                    .map_err(|_| fail("update_archive_path_invalid"))?;
                if !canonical.starts_with(
                    root.canonicalize()
                        .map_err(|_| fail("update_archive_path_invalid"))?,
                ) {
                    return Err(fail("update_archive_path_invalid"));
                }
            }
            ancestor = parent.parent();
        }
        if !entry
            .unpack_in(root)
            .map_err(|_| fail("update_extract_failed"))?
        {
            return Err(fail("update_archive_path_invalid"));
        }
    }
    let name = bundle.ok_or_else(|| fail("update_extract_failed"))?;
    let bundle = root.join(name);
    if !bundle.join("Contents/Info.plist").is_file() {
        return Err(fail("update_bundle_invalid"));
    }
    Ok(bundle)
}
pub fn validate_macos_bundle(bundle: &Path, version: &str) -> Result<()> {
    let Some(trust) = compiled_trust() else {
        return Err(fail("update_disabled"));
    };
    let fingerprint = trust
        .mac_certificate_sha256
        .filter(|f| f.len() == 64 && f.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| fail("update_certificate_unconfigured"))?;
    let value = plist::Value::from_file(bundle.join("Contents/Info.plist"))
        .map_err(|_| fail("update_bundle_invalid"))?;
    let dict = value
        .as_dictionary()
        .ok_or_else(|| fail("update_bundle_invalid"))?;
    if dict
        .get("CFBundleIdentifier")
        .and_then(plist::Value::as_string)
        != Some("io.orivelle.orbit")
        || dict
            .get("CFBundleShortVersionString")
            .and_then(plist::Value::as_string)
            != Some(version)
    {
        return Err(fail("update_bundle_invalid"));
    }
    let executable = dict
        .get("CFBundleExecutable")
        .and_then(plist::Value::as_string)
        .filter(|s| !s.contains('/') && !s.contains("..") && !s.is_empty())
        .ok_or_else(|| fail("update_bundle_invalid"))?;
    let file = bundle.join("Contents/MacOS").join(executable);
    let mut header = [0u8; 12];
    File::open(file)
        .and_then(|mut f| f.read_exact(&mut header))
        .map_err(|_| fail("update_architecture_invalid"))?;
    if header[..4] != [0xcf, 0xfa, 0xed, 0xfe]
        || u32::from_le_bytes(
            header[4..8]
                .try_into()
                .map_err(|_| fail("update_architecture_invalid"))?,
        ) != 0x0100000c
    {
        return Err(fail("update_architecture_invalid"));
    }
    #[cfg(target_os = "macos")]
    {
        let verified = std::process::Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(bundle)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|_| fail("update_codesign_invalid"))?;
        if !verified.success() {
            return Err(fail("update_codesign_invalid"));
        }
        let prefix = bundle
            .parent()
            .ok_or_else(|| fail("invalid_path"))?
            .join(format!("cert-{}-", uuid()));
        let mut extract = std::ffi::OsString::from("--extract-certificates=");
        extract.push(&prefix);
        let result = std::process::Command::new("/usr/bin/codesign")
            .arg("--display")
            .arg(extract)
            .arg(bundle)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|_| fail("update_codesign_invalid"))?;
        let mut cert = prefix.into_os_string();
        cert.push("0");
        let cert = PathBuf::from(cert);
        let actual = crate::images::bounded_read(&cert, 64 * 1024).map(|b| sha256(&b));
        let _ = fs::remove_file(&cert);
        if !result.success() || actual?.to_ascii_lowercase() != fingerprint.to_ascii_lowercase() {
            return Err(fail("update_certificate_mismatch"));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = fingerprint;
        return Err(fail("platform_unsupported"));
    }
    Ok(())
}
#[cfg(target_os = "macos")]
pub fn swap_macos_bundle(current: &Path, staged: &Path) -> Result<()> {
    use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
    let parent = current.parent().ok_or_else(|| fail("invalid_path"))?;
    if staged.parent() != Some(parent) {
        return Err(fail("update_staging_device_mismatch"));
    }
    let before = fs::symlink_metadata(current).map_err(|_| fail("update_install_failed"))?;
    let staged_meta = fs::symlink_metadata(staged).map_err(|_| fail("update_install_failed"))?;
    if !before.is_dir()
        || before.file_type().is_symlink()
        || !staged_meta.is_dir()
        || staged_meta.file_type().is_symlink()
        || before.dev() != staged_meta.dev()
    {
        return Err(fail("update_install_failed"));
    }
    let current_c =
        std::ffi::CString::new(current.as_os_str().as_bytes()).map_err(|_| fail("invalid_path"))?;
    let staged_c =
        std::ffi::CString::new(staged.as_os_str().as_bytes()).map_err(|_| fail("invalid_path"))?;
    let check = fs::symlink_metadata(current).map_err(|_| fail("update_install_failed"))?;
    if check.dev() != before.dev() || check.ino() != before.ino() {
        return Err(fail("update_install_conflict"));
    }
    let result =
        unsafe { libc::renamex_np(current_c.as_ptr(), staged_c.as_ptr(), libc::RENAME_SWAP) };
    if result != 0 {
        return Err(fail("update_install_failed"));
    }
    File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|_| fail("update_install_failed"))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    const GITHUB_MANIFEST: &str =
        "https://github.com/Owner/Orbit/releases/latest/download/latest.json";
    const GITHUB_ASSET: &str =
        "https://github.com/Owner/Orbit/releases/download/v0.2.0/Orbit-macos.app.tar.gz";

    #[test]
    fn github_update_artifact_must_match_the_pinned_repository_and_version() {
        let http = Http::new().unwrap();
        let channel = UpdateChannel::new(&http, GITHUB_MANIFEST).unwrap();
        assert!(channel.artifact_url(&http, GITHUB_ASSET, "0.2.0").is_ok());
        assert!(channel.artifact_url(&http, GITHUB_ASSET, "0.2.1").is_err());
        for bad_url in [
            "https://github.com/Other/Orbit/releases/download/v0.2.0/Orbit-macos.app.tar.gz",
            "https://github.com/Owner/Orbit/releases/download/v0.2.0/Orbit-windows.exe",
            "https://github.com/Owner/Orbit/releases/download/v0.2.0%2fother/Orbit-macos.app.tar.gz",
            "https://release-assets.githubusercontent.com/github-production-release-asset/123/file",
        ] {
            assert!(channel.artifact_url(&http, bad_url, "0.2.0").is_err());
        }
        assert!(
            UpdateChannel::new(
                &http,
                "https://github.com:8443/Owner/Orbit/releases/latest/download/latest.json"
            )
            .is_err()
        );
    }

    #[test]
    fn github_update_redirects_allow_only_matching_assets_and_github_cdn() {
        let manifest = Url::parse(GITHUB_MANIFEST).unwrap();
        let tag_manifest =
            Url::parse("https://github.com/Owner/Orbit/releases/download/v0.2.0/latest.json")
                .unwrap();
        let cdn = Url::parse(
            "https://release-assets.githubusercontent.com/github-production-release-asset/123/file?signature=synthetic",
        )
        .unwrap();
        assert!(github_update_redirect(
            &manifest,
            &tag_manifest,
            "/Owner/Orbit",
            "latest.json",
            None,
        ));
        assert!(github_update_redirect(
            &tag_manifest,
            &cdn,
            "/Owner/Orbit",
            "latest.json",
            None,
        ));
        let artifact = Url::parse(GITHUB_ASSET).unwrap();
        assert!(github_update_redirect(
            &artifact,
            &cdn,
            "/Owner/Orbit",
            "Orbit-macos.app.tar.gz",
            Some("0.2.0"),
        ));
        assert!(!github_update_redirect(
            &cdn,
            &artifact,
            "/Owner/Orbit",
            "Orbit-macos.app.tar.gz",
            Some("0.2.0"),
        ));
        for bad_url in [
            "http://release-assets.githubusercontent.com/github-production-release-asset/123/file",
            "https://release-assets.githubusercontent.com:8443/github-production-release-asset/123/file",
            "https://user@release-assets.githubusercontent.com/github-production-release-asset/123/file",
            "https://release-assets.githubusercontent.com/github-production-release-asset/123/file#fragment",
            "https://release-assets.githubusercontent.com.evil.test/github-production-release-asset/123/file",
            "https://release-assets.githubusercontent.com/another-service/file",
            "https://github.com/Other/Orbit/releases/download/v0.2.0/latest.json",
            "https://github.com/Owner/Orbit/releases/download/v0.1.0/Orbit-macos.app.tar.gz",
        ] {
            let bad = Url::parse(bad_url).unwrap();
            assert!(!github_update_redirect(
                &manifest,
                &bad,
                "/Owner/Orbit",
                "latest.json",
                None,
            ));
            assert!(!github_update_redirect(
                &artifact,
                &bad,
                "/Owner/Orbit",
                "Orbit-macos.app.tar.gz",
                Some("0.2.0"),
            ));
        }
    }

    #[test]
    fn tauri_signature_box_validates_exact_bytes_version_and_global_comment() {
        let wire: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/updater-wire.json")).unwrap();
        let key = wire["public_key"].as_str().unwrap();
        let signature = wire["signature"].as_str().unwrap();
        let bytes = STANDARD
            .decode(wire["artifact_base64"].as_str().unwrap())
            .unwrap();
        assert!(verify_artifact(key, signature, "0.2.0", &bytes).is_ok());
        assert!(verify_artifact(key, signature, "0.3.0", &bytes).is_err());
        let mut changed = bytes.clone();
        changed[0] ^= 1;
        assert!(verify_artifact(key, signature, "0.2.0", &changed).is_err());
        assert!(
            verify_artifact(
                key,
                wire["unversioned_signature"].as_str().unwrap(),
                "0.2.0",
                &bytes
            )
            .is_err()
        );
        let tampered = decode_signature_box(signature)
            .unwrap()
            .replace("version:0.2.0", "version:0.3.0");
        assert!(verify_artifact(key, &STANDARD.encode(tampered), "0.3.0", &bytes).is_err());
    }
    #[test]
    fn verified_artifact_does_not_reopen_replaced_source_path() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        fs::write(&source, b"verified bytes").unwrap();
        let artifact = VerifiedArtifact {
            candidate: Candidate {
                version: "0.2.0".into(),
                target: "darwin-aarch64".into(),
                url: "https://updates.example.test/orbit".into(),
                signature: "synthetic".into(),
                file_name: None,
                hash: Some(sha256(b"verified bytes")),
                verified: true,
            },
            bytes: fs::read(&source).unwrap(),
        };
        fs::write(&source, b"replaced unsigned bytes").unwrap();
        assert_eq!(artifact.bytes(), b"verified bytes");
        assert_eq!(artifact.into_bytes(), b"verified bytes");
    }
    #[test]
    fn default_channel_is_disabled_without_request() {
        assert!(compiled_trust().is_none());
        assert_eq!(disabled().status, "disabled");
    }
    #[test]
    fn requires_signed_exact_version_not_artifact_filename() {
        assert!(verified_version("timestamp:1\tfile:Orbit-2.0.0.tar.gz", "2.0.0").is_err());
        assert!(verified_version("timestamp:1\tversion:2.0.0", "2.0.0").is_ok());
        assert!(verified_version("version:2.0.0\tversion:2.0.0", "2.0.0").is_err());
        assert!(verified_version("version:1.0.0", "2.0.0").is_err());
    }
    #[test]
    fn exclusive_update_lock_waits_for_helper() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.path().join("locks/helper-update.lock"))
            .unwrap();
        lock.lock_shared().unwrap();
        assert!(exclusive_helper_lock(&store.root).is_err());
        lock.unlock().unwrap();
        assert!(exclusive_helper_lock(&store.root).is_ok());
    }
}
