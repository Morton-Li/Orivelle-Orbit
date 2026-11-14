//! Native window and tray lifecycle. Hiding a window never stops the Service.

use crate::commands::Native;
use orbit_core::dto::BootstrapView;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    App, AppHandle, Emitter, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

pub(crate) const TRAY_ID: &str = "orbit-tray";
const OPEN: &str = "orbit-open";
const SETTINGS: &str = "orbit-settings";
const QUIT: &str = "orbit-quit";

#[derive(Default)]
pub(crate) struct Lifecycle {
    tray_ready: AtomicBool,
    quit_dialog_open: AtomicBool,
    notice_open: AtomicBool,
    quitting: AtomicBool,
}

pub(crate) struct TrayMenu {
    status: MenuItem<tauri::Wry>,
    open: MenuItem<tauri::Wry>,
    pub(crate) settings: MenuItem<tauri::Wry>,
    pub(crate) quit: MenuItem<tauri::Wry>,
}

struct Labels {
    open: &'static str,
    settings: &'static str,
    quit: &'static str,
    healthy: &'static str,
    working: &'static str,
    login: &'static str,
    unavailable: &'static str,
    updating: &'static str,
    tray_failed: &'static str,
    window_failed: &'static str,
    quit_busy: &'static str,
    quit_tasks: &'static str,
    keep_running: &'static str,
    ok: &'static str,
}

const ZH: Labels = Labels {
    open: "打开 Orbit",
    settings: "设置…",
    quit: "退出 Orbit",
    healthy: "运行正常",
    working: "有图片任务进行中",
    login: "需要登录",
    unavailable: "安全存储不可用",
    updating: "正在安装更新",
    tray_failed: "菜单栏或系统托盘图标暂不可用。Orbit 会保留窗口以便重新访问，当前任务继续运行。",
    window_failed: "无法收起或恢复 Orbit 窗口，请从菜单栏或系统托盘重试。",
    quit_busy: "Orbit 正在更新，或本机数据暂不可用。退出可能中断当前操作，是否退出 Orbit？",
    quit_tasks: "仍有图片任务、未保存的结果或正在转发的 Claude 请求。退出将停止本机处理，已提交的请求仍可能完成并计费。是否退出 Orbit？",
    keep_running: "继续运行",
    ok: "知道了",
};

const EN: Labels = Labels {
    open: "Open Orbit",
    settings: "Settings…",
    quit: "Quit Orbit",
    healthy: "Running normally",
    working: "Image tasks in progress",
    login: "Sign in required",
    unavailable: "Secure storage unavailable",
    updating: "Installing update",
    tray_failed: "The menu bar or system tray icon is unavailable. Orbit will keep its window accessible while current tasks continue.",
    window_failed: "Orbit could not hide or restore its window. Please try again from the menu bar or system tray.",
    quit_busy: "Orbit is updating or its local data is unavailable. Quitting may interrupt the current operation. Quit Orbit?",
    quit_tasks: "Image tasks, unsaved results or forwarded Claude requests are still active. Quitting stops local processing. Submitted requests may still complete and incur charges. Quit Orbit?",
    keep_running: "Keep running",
    ok: "OK",
};

fn labels(locale: &str) -> &'static Labels {
    if locale == "en" { &EN } else { &ZH }
}

fn current_labels(app: &AppHandle) -> &'static Labels {
    app.try_state::<Native>()
        .and_then(|native| native.service.store.settings().ok())
        .map(|settings| labels(&settings.locale))
        .unwrap_or(&ZH)
}

pub(crate) fn setup(app: &mut App) -> tauri::Result<()> {
    let text = current_labels(app.handle());
    let status = MenuItem::with_id(app, "orbit-status", text.healthy, false, None::<&str>)?;
    let open = MenuItem::with_id(app, OPEN, text.open, true, None::<&str>)?;
    let settings = MenuItem::with_id(
        app,
        SETTINGS,
        text.settings,
        true,
        cfg!(target_os = "macos").then_some("CmdOrCtrl+,"),
    )?;
    let quit = MenuItem::with_id(
        app,
        QUIT,
        text.quit,
        true,
        cfg!(target_os = "macos").then_some("CmdOrCtrl+Q"),
    )?;
    #[cfg(target_os = "macos")]
    application_menu(app, &open, &settings, &quit)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&status, &separator, &open, &settings, &quit])?;
    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(tray_icon(app)?)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Orivelle Orbit")
        .menu(&menu)
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_tray_icon_event(|_tray, _event| {
            #[cfg(target_os = "macos")]
            if matches!(
                _event,
                tauri::tray::TrayIconEvent::Click {
                    button: tauri::tray::MouseButton::Left,
                    button_state: tauri::tray::MouseButtonState::Up,
                    ..
                }
            ) {
                #[cfg(feature = "native-acceptance")]
                crate::native_acceptance::record(_tray.app_handle(), "tray-left-mouse-up");
                crate::menubar::toggle(_tray.app_handle());
            }
            #[cfg(target_os = "windows")]
            match _event {
                tauri::tray::TrayIconEvent::Click {
                    button: tauri::tray::MouseButton::Left,
                    button_state: tauri::tray::MouseButtonState::Down,
                    ..
                } => crate::menubar::tray_pressed(_tray.app_handle()),
                tauri::tray::TrayIconEvent::Click {
                    button: tauri::tray::MouseButton::Left,
                    button_state: tauri::tray::MouseButtonState::Up,
                    rect,
                    ..
                } => crate::menubar::toggle_at(_tray.app_handle(), rect),
                tauri::tray::TrayIconEvent::DoubleClick {
                    button: tauri::tray::MouseButton::Left,
                    ..
                } => crate::menubar::tray_double_clicked(_tray.app_handle()),
                _ => {}
            }
        })
        .build(app)?;
    app.manage(TrayMenu {
        status,
        open,
        settings,
        quit,
    });
    app.state::<Lifecycle>()
        .tray_ready
        .store(true, Ordering::SeqCst);
    if let Err(error) = crate::menubar::setup(app) {
        // The native menu remains usable if the optional panel cannot be built.
        eprintln!("Orbit menu bar panel setup failed: {error}");
    }
    #[cfg(all(target_os = "macos", feature = "native-acceptance"))]
    crate::native_acceptance::install_menu(app)?;
    if let Ok(state) = app.state::<Native>().service.bootstrap() {
        update_tray(app.handle(), &state);
    }
    // The application manager retains the tray for the process lifetime.
    let _ = tray;
    Ok(())
}

pub(crate) fn menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    #[cfg(all(target_os = "macos", feature = "native-acceptance"))]
    if event.id.as_ref() == crate::native_acceptance::TOGGLE
        && app.config().identifier == "io.orivelle.orbit.menubar-smoke"
    {
        crate::native_acceptance::record(app, "acceptance-menu-toggle");
        crate::menubar::toggle(app);
        return;
    }
    match event.id.as_ref() {
        OPEN => open_main(app),
        SETTINGS => {
            open_main(app);
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.emit("orbit://open-settings", ());
            }
        }
        QUIT => request_quit(app),
        _ => {}
    }
}

#[cfg(target_os = "macos")]
fn application_menu(
    app: &App,
    open: &MenuItem<tauri::Wry>,
    settings: &MenuItem<tauri::Wry>,
    quit: &MenuItem<tauri::Wry>,
) -> tauri::Result<()> {
    let menu = Menu::default(app.handle())?;
    if let Some(tauri::menu::MenuItemKind::Submenu(application)) = menu.items()?.into_iter().next()
    {
        // Tauri's predefined macOS Quit invokes NSApplication.terminate directly,
        // bypassing ExitRequested. Route Cmd+Q through the same guard as the tray.
        let count = application.items()?.len();
        application.remove_at(count - 1)?;
        application.append(quit)?;
        application.insert_items(&[open, settings, &PredefinedMenuItem::separator(app)?], 2)?;
    }
    app.set_menu(menu)?;
    Ok(())
}

fn tray_icon(_app: &App) -> tauri::Result<tauri::image::Image<'static>> {
    #[cfg(target_os = "windows")]
    if let Some(icon) = _app.default_window_icon() {
        return Ok(icon.clone().to_owned());
    }
    let image = image::load_from_memory(include_bytes!("../icons/tray-template@2x.png"))
        .map_err(|error| tauri::Error::Io(std::io::Error::other(error)))?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Ok(tauri::image::Image::new_owned(
        image.into_raw(),
        width,
        height,
    ))
}

pub(crate) fn update_tray(app: &AppHandle, state: &BootstrapView) {
    let text = labels(&state.settings.locale);
    let status = if state.maintenance_active {
        text.updating
    } else if state
        .tasks
        .iter()
        .any(|task| ["queued", "sending", "waiting", "saving"].contains(&task.state.as_str()))
    {
        text.working
    } else if state.session.status == "vault_unavailable" {
        text.unavailable
    } else if ["authenticated", "refreshing"].contains(&state.session.status.as_str()) {
        text.healthy
    } else {
        text.login
    };
    // Service observers can run while domain locks are held. Do not wait for
    // native menu operations on those worker threads.
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(menu) = handle.try_state::<TrayMenu>() else {
            return;
        };
        let _ = menu.status.set_text(status);
        let _ = menu.open.set_text(text.open);
        let _ = menu.settings.set_text(text.settings);
        let _ = menu.quit.set_text(text.quit);
        if let Some(tray) = handle.tray_by_id(TRAY_ID) {
            let _ = tray.set_tooltip(Some(format!("Orbit · {status}")));
        }
    });
}

pub(crate) fn ready(app: &AppHandle) {
    if !app.state::<Lifecycle>().tray_ready.load(Ordering::SeqCst) {
        notice(app, current_labels(app).tray_failed);
    }
}

fn show_main(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(target_os = "macos")]
        app.set_dock_visibility(true)?;
        window.show()?;
        window.unminimize()?;
        window.set_focus()?;
    }
    Ok(())
}

pub(crate) fn open_main(app: &AppHandle) {
    crate::menubar::hide(app);
    if show_main(app).is_err() {
        notice(app, current_labels(app).window_failed);
    }
}

pub(crate) fn close_main(app: &AppHandle) {
    let Some(native) = app.try_state::<Native>() else {
        return;
    };
    let Ok(settings) = native.service.store.settings() else {
        notice(app, current_labels(app).window_failed);
        return;
    };
    if settings.close_behavior == "quit" {
        request_quit(app);
        return;
    }
    // platform_default is the legacy value and follows the new tray default.
    if !app.state::<Lifecycle>().tray_ready.load(Ordering::SeqCst) {
        notice(app, labels(&settings.locale).tray_failed);
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        if window.hide().is_err() {
            notice(app, labels(&settings.locale).window_failed);
            return;
        }
        #[cfg(target_os = "macos")]
        if app.set_dock_visibility(false).is_err() {
            // Restore the window if the transition cannot finish.
            notice(app, labels(&settings.locale).window_failed);
        }
    }
}

fn notice(app: &AppHandle, message: &'static str) {
    let lifecycle = app.state::<Lifecycle>();
    if lifecycle.notice_open.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = show_main(app);
    let handle = app.clone();
    let mut dialog = app
        .dialog()
        .message(message)
        .title("Orivelle Orbit")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCustom(
            current_labels(app).ok.into(),
        ));
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.parent(&window);
    }
    dialog.show(move |_| {
        handle
            .state::<Lifecycle>()
            .notice_open
            .store(false, Ordering::SeqCst);
    });
}

pub(crate) fn request_quit(app: &AppHandle) {
    let lifecycle = app.state::<Lifecycle>();
    if lifecycle.quitting.load(Ordering::SeqCst)
        || lifecycle.quit_dialog_open.swap(true, Ordering::SeqCst)
    {
        return;
    }
    let Some(native) = app.try_state::<Native>() else {
        app.exit(0);
        return;
    };
    let text = current_labels(app);
    let message = if native.service.ensure_mutations_available().is_err() {
        text.quit_busy
    } else if native.service.active_tasks().unwrap_or(true) {
        text.quit_tasks
    } else {
        finish_quit(app);
        return;
    };
    let _ = show_main(app);
    let handle = app.clone();
    let mut dialog = app
        .dialog()
        .message(message)
        .title(text.quit)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            text.quit.into(),
            text.keep_running.into(),
        ));
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.parent(&window);
    }
    dialog.show(move |confirmed| {
        handle
            .state::<Lifecycle>()
            .quit_dialog_open
            .store(false, Ordering::SeqCst);
        if confirmed {
            finish_quit(&handle);
        }
    });
}

fn finish_quit(app: &AppHandle) {
    app.state::<Lifecycle>()
        .quitting
        .store(true, Ordering::SeqCst);
    app.exit(0);
}

pub(crate) fn exit_requested(app: &AppHandle, code: Option<i32>, api: &tauri::ExitRequestApi) {
    // Programmatic exits, including updater restarts and secondary instances,
    // must pass through. User Quit / Cmd+Q follows the same guard as the tray.
    if code.is_none() && !app.state::<Lifecycle>().quitting.load(Ordering::SeqCst) {
        api.prevent_exit();
        request_quit(app);
    }
}
