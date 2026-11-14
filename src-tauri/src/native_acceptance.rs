//! Opt-in local acceptance evidence. Never enabled by a normal build or release.
use std::{
    fs::OpenOptions,
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};

pub(crate) const TOGGLE: &str = "orbit-acceptance-toggle";

pub(crate) fn install_menu(app: &tauri::App) -> tauri::Result<()> {
    if app.config().identifier != "io.orivelle.orbit.menubar-smoke" {
        return Ok(());
    }
    let Some(menu) = app.menu() else {
        return Ok(());
    };
    if let Some(tauri::menu::MenuItemKind::Submenu(application)) = menu.items()?.into_iter().next()
    {
        let toggle = tauri::menu::MenuItem::with_id(
            app,
            TOGGLE,
            "展开或收起快捷面板（验收）",
            true,
            Some("CmdOrCtrl+Shift+P"),
        )?;
        application.insert(&toggle, 2)?;
    }
    Ok(())
}

pub(crate) fn record_settled(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || record(&handle, "panel-settled"));
    });
}

pub(crate) fn record(app: &AppHandle, event: &str) {
    // Tracing is restricted to the disposable, credential-free acceptance app.
    if app.config().identifier != "io.orivelle.orbit.menubar-smoke" {
        return;
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../runtime/menubar-native-events.jsonl");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 256 * 1024) {
        return;
    }
    let windows: Vec<_> = ["main", "menubar"]
        .iter()
        .filter_map(|label| {
            let window = app.get_webview_window(label)?;
            let position = window.outer_position().ok();
            let size = window.outer_size().ok();
            let pointer = window.ns_window().ok()? as *mut objc2_app_kit::NSWindow;
            // Acceptance callbacks run on AppKit's main thread and only inspect
            // windows belonging to this disposable app.
            let native = unsafe { &*pointer };
            Some(serde_json::json!({
                "label":label,"visible":window.is_visible().ok(),"focused":window.is_focused().ok(),
                "fullscreen":window.is_fullscreen().ok(),"scale":window.scale_factor().ok(),
                "on_active_space":native.isOnActiveSpace(),"level":native.level(),
                "collection_behavior":native.collectionBehavior().bits(),
                "native_class":native.class().name().to_string_lossy(),
                "style_mask":native.styleMask().bits(),
                "occlusion_state":native.occlusionState().bits(),
                "x":position.map(|p|p.x),"y":position.map(|p|p.y),
                "width":size.map(|s|s.width),"height":size.map(|s|s.height)
            }))
        })
        .collect();
    let tray = app
        .tray_by_id(crate::desktop::TRAY_ID)
        .and_then(|t| t.rect().ok().flatten())
        .map(|r| {
            let p = r.position.to_physical::<f64>(1.0);
            let s = r.size.to_physical::<f64>(1.0);
            serde_json::json!({"x":p.x,"y":p.y,"width":s.width,"height":s.height})
        });
    let monitors: Vec<_> = app.available_monitors().unwrap_or_default().into_iter().map(|m| {
        serde_json::json!({"x":m.position().x,"y":m.position().y,"width":m.size().width,"height":m.size().height,"scale":m.scale_factor()})
    }).collect();
    let evidence = serde_json::json!({
        "time_ms":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
        "event":event,"windows":windows,"tray":tray,"monitors":monitors,
        "app_active":tauri_nspanel::objc2::MainThreadMarker::new().map(|marker|
            objc2_app_kit::NSApplication::sharedApplication(marker).isActive())
    });
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{evidence}");
    }
}
