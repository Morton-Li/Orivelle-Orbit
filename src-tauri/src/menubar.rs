//! macOS-only, preloaded panel. Closing it never changes the Service lifecycle.
use crate::{
    desktop,
    menubar_geometry::{Bounds, place},
};
use orbit_core::error::{Result, fail};
use serde::Deserialize;
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::{
    App, AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

pub(crate) const LABEL: &str = "menubar";
#[derive(Default)]
struct PanelState {
    ready: AtomicBool,
    requested: AtomicBool,
    menu_open: AtomicBool,
    height: Mutex<f64>,
    blurred: Mutex<Option<Instant>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Destination {
    Home,
    Settings,
    Usage,
    Images,
    Access,
    Codex,
    Claude,
}
impl Destination {
    fn path(&self) -> &'static str {
        match self {
            Self::Home => "/",
            Self::Settings => "/settings",
            Self::Usage => "/usage",
            Self::Images => "/images",
            Self::Access => "/access",
            Self::Codex => "/access#client-codex",
            Self::Claude => "/access#client-claude",
        }
    }
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Control {
    Ready {},
    Hide {},
    Resize { height: f64 },
    Open { destination: Destination },
    Menu {},
}

pub(crate) fn setup(app: &mut App) -> tauri::Result<()> {
    app.manage(PanelState {
        height: Mutex::new(660.0),
        ..Default::default()
    });
    let window =
        WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html#/menubar".into()))
            .title("Orbit")
            .inner_size(400.0, 660.0)
            .visible(false)
            .focused(false)
            .decorations(false)
            .transparent(true)
            .incognito(true)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .visible_on_all_workspaces(true)
            .shadow(false)
            .disable_drag_drop_handler()
            .build()?;
    // A utility surface must also be usable above another app's full-screen Space.
    let pointer = window.ns_window()? as *mut objc2_app_kit::NSWindow;
    // setup and all AppKit window operations run on Tauri's main thread.
    let native = unsafe { &*pointer };
    use objc2_app_kit::NSWindowCollectionBehavior as Behavior;
    native.setCollectionBehavior(
        Behavior::CanJoinAllSpaces
            | Behavior::FullScreenAuxiliary
            | Behavior::Transient
            | Behavior::IgnoresCycle,
    );
    if let Some(tray) = app.tray_by_id(desktop::TRAY_ID) {
        tray.set_show_menu_on_left_click(false)?;
    }
    #[cfg(feature = "native-acceptance")]
    crate::native_acceptance::record(app.handle(), "panel-setup");
    Ok(())
}

fn layout(app: &AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(LABEL) else {
        return Ok(());
    };
    let Some(rect) = app
        .tray_by_id(desktop::TRAY_ID)
        .and_then(|tray| tray.rect().ok().flatten())
    else {
        return Ok(());
    };
    let point = rect.position.to_physical::<f64>(1.0);
    let Some(monitor) = app
        .monitor_from_point(point.x, point.y)?
        .or(app.primary_monitor()?)
    else {
        return Ok(());
    };
    let scale = monitor.scale_factor();
    let tray_position = rect.position.to_logical::<f64>(scale);
    let tray_size = rect.size.to_logical::<f64>(scale);
    let work = monitor.work_area();
    let requested_height = *app
        .state::<PanelState>()
        .height
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let position = place(
        Bounds {
            x: tray_position.x,
            y: tray_position.y,
            width: tray_size.width,
            height: tray_size.height,
        },
        Bounds {
            x: f64::from(work.position.x) / scale,
            y: f64::from(work.position.y) / scale,
            width: f64::from(work.size.width) / scale,
            height: f64::from(work.size.height) / scale,
        },
        requested_height,
    );
    window.set_size(LogicalSize::new(position.width, position.height))?;
    window.set_position(LogicalPosition::new(position.x, position.y))?;
    window.emit("orbit://menubar-layout", position.arrow)?;
    Ok(())
}

fn show(app: &AppHandle) -> tauri::Result<()> {
    layout(app)?;
    if let Some(window) = app.get_webview_window(LABEL) {
        window.show()?;
        window.set_focus()?;
        window.emit("orbit://menubar-opened", ())?;
    }
    #[cfg(feature = "native-acceptance")]
    crate::native_acceptance::record(app, "panel-shown");
    Ok(())
}
pub(crate) fn hide(app: &AppHandle) {
    if let Some(state) = app.try_state::<PanelState>() {
        state.requested.store(false, Ordering::SeqCst);
    }
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.emit("orbit://menubar-hidden", ());
        let _ = window.hide();
    }
    #[cfg(feature = "native-acceptance")]
    crate::native_acceptance::record(app, "panel-hidden");
}
pub(crate) fn blur(app: &AppHandle) {
    let Some(state) = app.try_state::<PanelState>() else {
        return;
    };
    if state.menu_open.load(Ordering::SeqCst) {
        return;
    }
    *state.blurred.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    hide(app);
}
pub(crate) fn toggle(app: &AppHandle) {
    #[cfg(feature = "native-acceptance")]
    crate::native_acceptance::record(app, "panel-toggle-request");
    let Some(state) = app.try_state::<PanelState>() else {
        return;
    };
    if state.requested.swap(false, Ordering::SeqCst) {
        hide(app);
        return;
    }
    // Clicking the status item first blurs the panel; do not reopen it on mouse-up.
    if state
        .blurred
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some_and(|time| time.elapsed() < Duration::from_millis(180))
    {
        return;
    }
    state.requested.store(true, Ordering::SeqCst);
    if state.ready.load(Ordering::SeqCst) && show(app).is_err() {
        state.requested.store(false, Ordering::SeqCst);
        desktop::open_main(app);
    }
}

#[tauri::command]
pub(crate) fn menubar_control(
    app: AppHandle,
    window: WebviewWindow,
    request: Control,
) -> Result<()> {
    if window.label() != LABEL {
        return Err(fail("operation_not_allowed"));
    }
    let state = app
        .try_state::<PanelState>()
        .ok_or_else(|| fail("operation_not_allowed"))?;
    let result = match request {
        Control::Ready {} => {
            state.ready.store(true, Ordering::SeqCst);
            #[cfg(feature = "native-acceptance")]
            crate::native_acceptance::record(&app, "panel-frontend-ready");
            if state.requested.load(Ordering::SeqCst) {
                show(&app)
            } else {
                Ok(())
            }
        }
        Control::Hide {} => {
            hide(&app);
            Ok(())
        }
        Control::Resize { height } => {
            if !height.is_finite() || !(100.0..=1400.0).contains(&height) {
                return Err(fail("invalid_request"));
            }
            *state.height.lock().unwrap_or_else(|e| e.into_inner()) = height;
            layout(&app)
        }
        Control::Open { destination } => {
            hide(&app);
            desktop::open_main(&app);
            app.get_webview_window("main")
                .ok_or_else(|| fail("operation_failed"))?
                .emit("orbit://open-page", destination.path())
        }
        Control::Menu {} => {
            let items = app.state::<desktop::TrayMenu>();
            let menu = tauri::menu::Menu::with_items(&app, &[&items.settings, &items.quit])
                .map_err(|_| fail("operation_failed"))?;
            state.menu_open.store(true, Ordering::SeqCst);
            let result = window.popup_menu(&menu);
            state.menu_open.store(false, Ordering::SeqCst);
            if !window.is_focused().unwrap_or(false) {
                hide(&app);
            }
            result
        }
    };
    result.map_err(|_| fail("operation_failed"))
}

#[cfg(test)]
mod tests {
    use super::Control;

    #[test]
    fn navigation_and_control_requests_have_closed_schemas() {
        assert!(
            serde_json::from_str::<Control>(r#"{"action":"open","destination":"codex"}"#).is_ok()
        );
        assert!(
            serde_json::from_str::<Control>(
                r#"{"action":"open","destination":"https://example.com"}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<Control>(r#"{"action":"ready","path":"/tmp/other"}"#).is_err()
        );
    }
}
