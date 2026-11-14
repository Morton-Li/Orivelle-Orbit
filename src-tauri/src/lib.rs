mod commands;
mod desktop;
#[cfg(target_os = "macos")]
mod menubar;
#[cfg(any(target_os = "macos", test))]
mod menubar_geometry;
#[cfg(all(target_os = "macos", feature = "native-acceptance"))]
mod native_acceptance;
#[cfg(target_os = "macos")]
use menubar::menubar_control;
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
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            desktop::open_main(app);
        }))
        .manage(desktop::Lifecycle::default())
        .on_menu_event(desktop::menu_event)
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
                app.path().app_data_dir().map_err(|_| {
                    eprintln!("Orbit could not resolve the app data directory.");
                    fail("local_storage_unavailable")
                })?
            } else {
                let identifier = &app.config().identifier;
                let directory = if identifier == "io.orivelle.orbit" {
                    "gui".to_owned()
                } else {
                    format!("gui-{identifier}")
                };
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../runtime")
                    .join(directory)
            };
            let instance = orbit_core::runtime_lock::RuntimeLock::acquire(&root)
                .inspect_err(|error| eprintln!("Orbit runtime lock setup: {}", error.code))?;
            let executable =
                std::env::current_exe().map_err(|_| fail("update_recovery_invalid"))?;
            let bundle = executable
                .ancestors()
                .find(|p| p.extension().is_some_and(|e| e == "app"));
            match orbit_core::update_recovery::boot(&root, bundle, env!("CARGO_PKG_VERSION"))? {
                orbit_core::update_recovery::BootDisposition::Ready => {}
                orbit_core::update_recovery::BootDisposition::RestartRequired => {
                    // Recovery closed all DB handles and restored the old bundle.
                    // Do not expose a Service from this already mapped new binary.
                    return Err(fail("update_restart_required").into());
                }
            }
            let store = Arc::new(
                Store::open(&root)
                    .inspect_err(|error| eprintln!("Orbit data store setup: {}", error.code))?,
            );
            let paths = if authorized {
                let home = app.path().home_dir().map_err(|_| {
                    eprintln!("Orbit could not resolve the user home directory.");
                    fail("local_storage_unavailable")
                })?;
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
                #[cfg(target_os = "macos")]
                let native = NativeVault::with_claude_mapping_creator(
                    root.clone(),
                    paths
                        .helper_expected_sha256
                        .clone()
                        .ok_or_else(|| fail("helper_artifact_invalid"))?,
                );
                #[cfg(not(target_os = "macos"))]
                let native = NativeVault::default();
                Arc::new(native)
            } else {
                Arc::new(DeniedVault)
            };
            let service = Service::new(store, vault, paths)?;
            let handle = app.handle().clone();
            service.observe(Arc::new(move |state| {
                desktop::update_tray(&handle, &state);
                let _ = handle.emit("orbit://state", state);
            }))?;
            let handle = app.handle().clone();
            service.observe_authorization(Arc::new(move |event| {
                let _ = handle.emit("orbit://authorization", event);
            }))?;
            let mapping_service = service.clone();
            tauri::async_runtime::spawn(async move {
                let observer = mapping_service.clone();
                let runtime = tokio::runtime::Handle::current();
                let result = tauri::async_runtime::spawn_blocking(move || {
                    let _runtime = runtime.enter();
                    mapping_service.reconcile_claude_mappings()
                })
                .await;
                if !matches!(result, Ok(Ok(()))) {
                    let _ = observer.note_claude_mapping_unavailable();
                }
            });
            app.manage(Native {
                service,
                downloads: Default::default(),
                _instance: instance,
            });
            if let Err(error) = desktop::setup(app) {
                eprintln!("Orbit tray setup failed: {error}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            #[cfg(target_os = "macos")]
            menubar_control,
            bootstrap,
            refresh_account,
            start_login,
            cancel_login,
            logout,
            select_node,
            list_models,
            read_usage_summary,
            list_usage_records,
            probe_nodes,
            cancel_node_probes,
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
            update_settings,
            prepare_diagnostics,
            export_diagnostics,
            check_update,
            download_update,
            cancel_update_download,
            install_update
        ])
        .on_window_event(|window, event| {
            #[cfg(all(target_os = "macos", feature = "native-acceptance"))]
            match event {
                tauri::WindowEvent::Focused(focused) => native_acceptance::record(
                    window.app_handle(),
                    &format!("{}:focused:{focused}", window.label()),
                ),
                tauri::WindowEvent::Resized(_) => native_acceptance::record(
                    window.app_handle(),
                    &format!("{}:resized", window.label()),
                ),
                _ => {}
            }
            match event {
                #[cfg(target_os = "macos")]
                tauri::WindowEvent::Focused(false) if window.label() == menubar::LABEL => {
                    menubar::blur(window.app_handle());
                }
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    #[cfg(target_os = "macos")]
                    if window.label() == menubar::LABEL {
                        api.prevent_close();
                        menubar::hide(window.app_handle());
                    }
                    if window.label() == "main" {
                        api.prevent_close();
                        desktop::close_main(window.app_handle());
                    }
                }
                tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) => {
                    let service = window.state::<Native>().service.clone();
                    let handle = window.app_handle().clone();
                    let paths = paths.clone();
                    if let Ok(state) = service.bootstrap() {
                        if let Some(id) = state.active_connection_id {
                            if let Some(connection) =
                                state.connections.into_iter().find(|c| c.id == id)
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
                                                        account_id: Some(
                                                            connection.account_id.clone(),
                                                        ),
                                                        connection_id: Some(id.clone()),
                                                        connection_revision: Some(
                                                            connection.revision,
                                                        ),
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
            }
        })
        .build(tauri::generate_context!())
        .expect("Orbit startup failed");
    app.run(|app, event| match event {
        tauri::RunEvent::Ready => desktop::ready(app),
        tauri::RunEvent::ExitRequested { code, api, .. } => {
            desktop::exit_requested(app, code, &api);
        }
        tauri::RunEvent::Exit => {
            if let Some(native) = app.try_state::<Native>() {
                native.service.stop_claude_mappings();
            }
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => desktop::open_main(app),
        _ => {}
    });
}
