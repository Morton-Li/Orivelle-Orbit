#[cfg(all(
    not(test),
    not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64")
    ))
))]
compile_error!("Orbit credential helper is delivered only on macOS arm64 and Windows x64.");
use orbit_core::{
    dto::ClientKind,
    error::{Result, fail},
    identity::{HelperBinding, bound_helper_token, validate_uuid},
    store::Store,
    vault::{DeniedVault, NativeVault, Vault},
};
use std::{fs::OpenOptions, io::Write, path::PathBuf};
fn arguments(args: impl IntoIterator<Item = String>) -> Result<(ClientKind, String)> {
    let values = args.into_iter().collect::<Vec<_>>();
    if values.len() != 2 {
        return Err(fail("invalid_helper_arguments"));
    }
    let client = match values[0].as_str() {
        "codex" => ClientKind::Codex,
        "claude" => ClientKind::Claude,
        _ => return Err(fail("invalid_helper_arguments")),
    };
    validate_uuid(&values[1])?;
    Ok((client, values[1].clone()))
}
fn run() -> Result<()> {
    let (client, id) = arguments(std::env::args().skip(1))?;
    // No path/origin/account/token argument is accepted. The fixed deployed helper
    // resides in <private application-data>/bin, independent of app bundle versions.
    let exe = std::env::current_exe()
        .map_err(|_| fail("binding_disabled"))?
        .canonicalize()
        .map_err(|_| fail("binding_disabled"))?;
    let bin = exe.parent().ok_or_else(|| fail("binding_disabled"))?;
    if bin.file_name().and_then(|n| n.to_str()) != Some("bin") {
        return Err(fail("binding_disabled"));
    }
    let root: PathBuf = bin
        .parent()
        .ok_or_else(|| fail("binding_disabled"))?
        .to_path_buf();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("locks/helper-update.lock"))
        .map_err(|_| fail("binding_disabled"))?;
    lock.try_lock_shared().map_err(|_| fail("update_busy"))?;
    let store = Store::open_readonly(&root)?;
    let binding = store
        .get::<HelperBinding>("binding", &id)?
        .ok_or_else(|| fail("binding_disabled"))?;
    let installation = store
        .installation(&binding.origin)?
        .ok_or_else(|| fail("binding_disabled"))?;
    if installation.id != binding.installation_id {
        return Err(fail("binding_disabled"));
    }
    let vault: Box<dyn Vault> = if cfg!(feature = "system-integration") {
        Box::new(NativeVault::default())
    } else {
        Box::new(DeniedVault)
    };
    let token = bound_helper_token(&binding, client, &id, &store, vault.as_ref(), chrono_now())?;
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(token.as_bytes())
        .and_then(|_| stdout.write_all(b"\n"))
        .and_then(|_| stdout.flush())
        .map_err(|_| fail("helper_output_failed"))?;
    Ok(())
}
fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(i64::MAX)
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Orbit helper: {}", error.code);
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_only_client_and_opaque_binding() {
        let id = orbit_core::identity::uuid();
        assert_eq!(
            arguments(["codex".into(), id.clone()]).unwrap(),
            (ClientKind::Codex, id)
        );
        assert!(arguments(["codex".into(), "token".into()]).is_err());
        assert!(
            arguments([
                "claude".into(),
                orbit_core::identity::uuid(),
                "--secret".into()
            ])
            .is_err()
        );
        assert!(arguments(["active".into(), orbit_core::identity::uuid()]).is_err());
    }
}
