#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Nova als Desktop-App: ein Fenster mit der Web-App des Servers, dazu
//!
//! - Tray-Symbol mit „Nova anzeigen“, „Mit dem System starten“ und „Beenden“;
//!   Schließen legt das Fenster in den Tray (wie bei Chat-Apps üblich),
//! - native Benachrichtigungen: das Notification-Plugin ersetzt
//!   `window.Notification`, die Web-App braucht dafür keinen eigenen Code,
//! - eine Instanz: ein zweiter Start holt das laufende Fenster nach vorn,
//! - Autostart (aus, bis man ihn im Tray einschaltet), startet dann im Tray.
//!
//! Die Web-App (fremde URL) bekommt nur die drei Benachrichtigungsrechte,
//! sonst keinen Zugriff auf Tauri.

use tauri::{
    ipc::CapabilityBuilder,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt;

const DEFAULT_URL: &str = "https://85-215-175-202.sslip.io";
/// Autostart-Argument: Fenster versteckt lassen, nur der Tray erscheint.
const HIDDEN_ARG: &str = "--minimized";

fn main() {
    let target = std::env::var("NOVA_DESKTOP_URL").unwrap_or_else(|_| DEFAULT_URL.to_owned());

    tauri::Builder::default()
        // Muss das erste Plugin sein.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("Nova")
                .arg(HIDDEN_ARG)
                .build(),
        )
        .setup(move |app| {
            let url: tauri::Url = target.parse()?;
            app.add_capability(
                CapabilityBuilder::new("nova-server")
                    .remote(url.origin().ascii_serialization())
                    .window("main")
                    .permission("notification:allow-is-permission-granted")
                    .permission("notification:allow-request-permission")
                    .permission("notification:allow-notify"),
            )?;
            let hidden = std::env::args().any(|arg| arg == HIDDEN_ARG);
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("Nova")
                .inner_size(1280.0, 800.0)
                .min_inner_size(960.0, 600.0)
                .visible(!hidden)
                .build()?;
            build_tray(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to run Nova");
}

/// Hauptfenster zeigen und nach vorn holen.
fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let autostart_on = app.autolaunch().is_enabled().unwrap_or(false);
    let show = MenuItem::with_id(app, "show", "Nova anzeigen", true, None::<&str>)?;
    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        "Mit dem System starten",
        true,
        autostart_on,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Beenden", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &PredefinedMenuItem::separator(app)?,
            &autostart,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let mut tray = TrayIconBuilder::with_id("nova")
        .tooltip("Nova")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show" => show_main(app),
            "autostart" => {
                let launcher = app.autolaunch();
                let on = launcher.is_enabled().unwrap_or(false);
                let _ = if on {
                    launcher.disable()
                } else {
                    launcher.enable()
                };
                // Haken zeigt den echten Zustand, auch wenn das Umschalten scheiterte.
                let _ = autostart.set_checked(launcher.is_enabled().unwrap_or(on));
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}
