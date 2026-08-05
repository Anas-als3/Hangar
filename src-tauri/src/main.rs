// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod env_resolve;
mod process;
mod registry;
mod run;

use tauri::Manager;

fn main() {
    // `main` stays a plain fn — no `#[tokio::main]`, no hand-rolled runtime (SPEC.md §4).
    tauri::Builder::default()
        // single-instance MUST be registered first (SPEC.md §4). Desktop-only, no capability entry.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // SPEC.md §4 (Storage): app_config_dir, created before the first write.
            let config_dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&config_dir)?;

            let load = registry::load_projects(&config_dir);
            let settings = registry::load_settings(&config_dir);

            app.manage(commands::AppState::new(
                config_dir,
                load.projects,
                settings,
                load.error,
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_projects,
            commands::get_settings,
            commands::set_settings,
            commands::get_registry_error,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
