mod commands;
#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64")
)))]
compile_error!("Orbit desktop supports only macOS arm64 and Windows x64.");
use commands::*;
use orbit_core::{
    error::fail,
    service::Service,
    store::Store,
    system::SystemPaths,
    vault::{DeniedVault, NativeVault, Vault},
};
use std::{path::PathBuf, sync::Arc};
use tauri::{Emitter, Manager};
#[cfg(feature = "system-integration")]
const BUNDLED_HELPER: &[u8] = include_bytes!(env!("ORBIT_BUNDLED_HELPER_FILE"));

pub fn run() {
    let app = tauri::Builder::default()
        .register_uri_scheme_protocol("asset", |context, request| {
            let bytes = if context.webview_label() == "main"
                && request.method() == tauri::http::Method::GET
                && request.uri().query().is_none()
            {
                context
                    .app_handle()
                    .try_state::<Native>()
                    .and_then(|n| n.service.read_preview_asset(request.uri().path()).ok())
            } else {
                None
            };
            tauri::http::Response::builder()
                .status(if bytes.is_some() { 200 } else { 404 })
                .header("Content-Type", "image/png")
                .header("Cache-Control", "no-store, max-age=0")
                .header("X-Content-Type-Options", "nosniff")
                .body(bytes.unwrap_or_default())
                .unwrap()
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let authorized = cfg!(feature = "system-integration");
            let root = if authorized {
                app.path()
                    .app_data_dir()
                    .map_err(|_| fail("local_storage_unavailable"))?
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/gui")
            };
            let store = Arc::new(Store::open(&root)?);
            let paths = if authorized {
                let home = app
                    .path()
                    .home_dir()
                    .map_err(|_| fail("local_storage_unavailable"))?;
                SystemPaths::native(&root, &home)?
            } else {
                SystemPaths::isolated(&root)?
            };
            #[cfg(feature = "system-integration")]
            let paths = {
                let mut paths = paths;
                paths.helper_expected_sha256 = Some(env!("ORBIT_BUNDLED_HELPER_SHA256").into());
                let status = if orbit_core::helper_artifact::deploy_embedded(
                    &store,
                    BUNDLED_HELPER,
                    env!("ORBIT_BUNDLED_HELPER_SHA256"),
                    env!("CARGO_PKG_VERSION"),
                )
                .is_ok()
                {
                    "ready"
                } else {
                    "blocked"
                };
                store.set_meta("helper_deployment_status", status)?;
                paths
            };
            let vault: Arc<dyn Vault> = if authorized {
                Arc::new(NativeVault::default())
            } else {
                Arc::new(DeniedVault)
            };
            let service = Service::new(store, vault, paths)?;
            {
                let _lock = orbit_core::updates::exclusive_helper_lock(&root)?;
            }
            let handle = app.handle().clone();
            service.observe(Arc::new(move |state| {
                let _ = handle.emit("orbit://state", state);
            }))?;
            app.manage(Native { service });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            refresh_account,
            start_login,
            logout,
            select_connection,
            select_node,
            delete_connection,
            revoke_inference_key,
            list_models,
            preview_client_config,
            apply_client_config,
            export_claude_import,
            preview_restore,
            restore_client_config,
            select_image_inputs,
            use_history_as_input,
            create_image_task,
            cancel_image_task,
            retry_local_save,
            save_image_as,
            copy_image,
            delete_history,
            reuse_task_parameters,
            update_settings,
            prepare_diagnostics,
            export_diagnostics,
            preview_forget_installation,
            forget_installation,
            check_update,
            download_update,
            install_update
        ])
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                let native = window.state::<Native>();
                if let Ok(settings) = native.service.store.settings() {
                    if settings.close_behavior == "keep_running"
                        || settings.close_behavior == "platform_default"
                            && cfg!(target_os = "macos")
                    {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            }
            tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) => {
                let service = window.state::<Native>().service.clone();
                let handle = window.app_handle().clone();
                let paths = paths.clone();
                if let Ok(state) = service.bootstrap() {
                    if let Some(id) = state.active_connection_id {
                        if let Some(connection) = state.connections.into_iter().find(|c| c.id == id)
                        {
                            tauri::async_runtime::spawn_blocking(move || {
                                let current = service.connection_view(&id);
                                if current.as_ref().is_ok_and(|c| {
                                    c.revision == connection.revision
                                        && c.account_id == connection.account_id
                                }) {
                                    match service.import_inputs(&id, &paths) {
                                        Ok(inputs) => {
                                            let _ = handle.emit(
                                                "orbit://image-inputs",
                                                InputsEvent {
                                                    account_id: connection.account_id,
                                                    connection_id: id,
                                                    connection_revision: connection.revision,
                                                    inputs,
                                                },
                                            );
                                        }
                                        Err(error) => {
                                            let _ = handle.emit(
                                                "orbit://error",
                                                ScopedErrorEvent {
                                                    error,
                                                    account_id: Some(connection.account_id.clone()),
                                                    connection_id: Some(id.clone()),
                                                    connection_revision: Some(connection.revision),
                                                },
                                            );
                                        }
                                    }
                                }
                            });
                        }
                    }
                }
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("Orbit startup failed");
    app.run(|app, event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app, event);
    });
}
