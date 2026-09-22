// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::menu::{AboutMetadata, Menu, PredefinedMenuItem, Submenu};

fn main() {
    tauri::Builder::default()
        .menu(|app| {
            Menu::with_items(
                app,
                &[
                    &Submenu::with_items(
                        app,
                        "File",
                        true,
                        &[&PredefinedMenuItem::quit(app, None)?],
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
        .run(tauri::generate_context!())
        .expect("failed to start Locus");
}
