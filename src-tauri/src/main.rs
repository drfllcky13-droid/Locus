// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::Emitter;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::project_create,
            commands::project_open,
            commands::import_preview,
            commands::import_commit,
            commands::evidence_verify,
        ])
        .menu(|app| {
            let item = |id: &str, text: &str, accel: Option<&str>| {
                MenuItem::with_id(app, id, text, true, accel)
            };
            Menu::with_items(
                app,
                &[
                    &Submenu::with_items(
                        app,
                        "File",
                        true,
                        &[
                            &item("new_project", "New Project…", Some("CmdOrCtrl+N"))?,
                            &item("open_project", "Open Project…", Some("CmdOrCtrl+O"))?,
                            &PredefinedMenuItem::separator(app)?,
                            &item("import", "Import Evidence…", Some("CmdOrCtrl+I"))?,
                            &item("verify_evidence", "Verify Evidence", None)?,
                            &PredefinedMenuItem::separator(app)?,
                            &PredefinedMenuItem::quit(app, None)?,
                        ],
                    )?,
                    &Submenu::with_items(
                        app,
                        "Edit",
                        true,
                        &[
                            &PredefinedMenuItem::cut(app, None)?,
                            &PredefinedMenuItem::copy(app, None)?,
                            &PredefinedMenuItem::paste(app, None)?,
                            &PredefinedMenuItem::select_all(app, None)?,
                        ],
                    )?,
                    &Submenu::with_items(
                        app,
                        "Help",
                        true,
                        &[&PredefinedMenuItem::about(
                            app,
                            Some("About Locus"),
                            Some(AboutMetadata {
                                name: Some("Locus".into()),
                                version: Some(env!("CARGO_PKG_VERSION").into()),
                                ..Default::default()
                            }),
                        )?],
                    )?,
                ],
            )
        })
        // Menu actions are handled in the UI, which owns the dialogs.
        .on_menu_event(|app, event| {
            let _ = app.emit("menu", event.id().0.as_str());
        })
        .run(tauri::generate_context!())
        .expect("failed to start Locus");
}
