fn main() {
    embedded_helper();
    const COMMANDS: &[&str] = &[
        "bootstrap",
        "refresh_account",
        "start_login",
        "cancel_login",
        "logout",
        "select_node",
        "list_models",
        "read_usage_summary",
        "list_usage_records",
        "probe_nodes",
        "cancel_node_probes",
        "preview_client_config",
        "apply_client_config",
        "export_claude_import",
        "preview_restore",
        "restore_client_config",
        "select_image_inputs",
        "use_history_as_input",
        "create_image_task",
        "cancel_image_task",
        "retry_local_save",
        "save_image_as",
        "copy_image",
        "delete_history",
        "reuse_task_parameters",
        "update_settings",
        "prepare_diagnostics",
        "export_diagnostics",
        "check_update",
        "download_update",
        "cancel_update_download",
        "install_update",
    ];
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("Orbit typed IPC permission generation failed");
}
fn embedded_helper() {
    println!("cargo:rerun-if-env-changed=ORBIT_BUNDLED_HELPER_PATH");
    if std::env::var_os("CARGO_FEATURE_SYSTEM_INTEGRATION").is_none() {
        return;
    }
    use sha2::Digest;
    let source = std::path::PathBuf::from(
        std::env::var_os("ORBIT_BUNDLED_HELPER_PATH")
            .expect("system-integration requires a project-built helper artifact"),
    );
    let project = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    let metadata = std::fs::symlink_metadata(&source).expect("helper artifact is unreadable");
    assert!(
        source.is_absolute()
            && metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() > 32
            && metadata.len() <= 64 * 1024 * 1024,
        "invalid helper artifact"
    );
    let source = source.canonicalize().unwrap();
    assert!(
        source.starts_with(project),
        "helper must be built inside this project"
    );
    let bytes = std::fs::read(&source).unwrap();
    let target = std::env::var("TARGET").unwrap();
    if target == "aarch64-apple-darwin" {
        assert!(
            bytes.get(..4) == Some(&[0xcf, 0xfa, 0xed, 0xfe])
                && u32::from_le_bytes(bytes[4..8].try_into().unwrap()) == 0x0100000c
                && u32::from_le_bytes(bytes[12..16].try_into().unwrap()) == 2,
            "helper must be one arm64 Mach-O executable"
        );
    } else if target == "x86_64-pc-windows-msvc" {
        assert!(
            bytes.get(..2) == Some(b"MZ") && bytes.len() >= 64,
            "helper must be a Windows x64 PE executable"
        );
        let offset = u32::from_le_bytes(bytes[60..64].try_into().unwrap()) as usize;
        assert!(
            bytes.get(offset..offset + 4) == Some(b"PE\0\0")
                && bytes.get(offset + 4..offset + 6) == Some(&[0x64, 0x86]),
            "helper must be AMD64"
        );
    } else {
        panic!("unsupported Orbit helper target");
    }
    println!("cargo:rerun-if-changed={}", source.display());
    println!(
        "cargo:rustc-env=ORBIT_BUNDLED_HELPER_FILE={}",
        source.display()
    );
    println!(
        "cargo:rustc-env=ORBIT_BUNDLED_HELPER_SHA256={:x}",
        sha2::Sha256::digest(&bytes)
    );
}
