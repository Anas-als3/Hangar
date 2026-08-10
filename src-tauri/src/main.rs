// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod env_resolve;
// Plan 058: default-ON (see `Cargo.toml`'s `[features]`), so the shipped app is unchanged. Only
// the `--no-default-features` Windows cross-check drops it, to get past `aws-lc-sys`'s C build
// script and back to compiling SPEC.md §8's `#[cfg(windows)]` code.
#[cfg(feature = "github")]
mod github;
// Plan 059: the other `reqwest` user, gated on the same terms and for the same reason. Default-ON,
// so the shipped app always has it — but the CHECK itself is off until the user turns it on, which
// is a settings question, never a compile-time one (see `osv.rs`'s RULE 1).
#[cfg(feature = "osv")]
mod osv;
mod preflight;
mod process;
mod registry;
mod run;
// SPEC.md §11 "Launch line" / plan 060. A sibling of `preflight`, never part of it: this one runs
// when the window opens, which is exactly what §11 forbids the Doctor's report from doing.
mod vcs;

use std::sync::atomic::Ordering;

use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

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
            // SPEC.md §18 / plan 053: its OWN managed cell, never nested inside `AppState` —
            // no GitHub call may hold a lock `AppState.projects`/`.runtime`/`.dev_env` uses.
            // Nothing here reads the keychain or the network; that happens lazily, once per
            // session, on the first `#[tauri::command]` call a webview action makes.
            #[cfg(feature = "github")]
            app.manage(github::GithubState::new());

            // SPEC.md §8: resolve the login-shell environment ONCE at startup, in the background so
            // it can never block the window from appearing. A Run that arrives before this finishes
            // awaits the same resolution rather than racing it (see `DevEnvCell`).
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<commands::AppState>();
                let _ = state.dev_env.get().await;
            });

            Ok(())
        })
        // SPEC.md §8, quit interception path 1 of 2: the window's close button (and Alt+F4).
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if intercept_quit(window.app_handle()) {
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_projects,
            commands::add_project,
            commands::update_project,
            commands::remove_project,
            commands::get_settings,
            commands::set_settings,
            commands::get_registry_error,
            commands::run_project,
            commands::stop_project,
            commands::open_in_browser,
            commands::open_in_editor,
            commands::get_log_buffer,
            commands::clear_log_buffer,
            commands::read_package_json,
            commands::get_port_status,
            commands::free_port,
            commands::find_free_port,
            // SPEC.md §11 "Doctor" / plan 057 — an addition to the frozen §7 list, never a
            // rename/reshape. Lazy: only the panel calls it, on open and on Refresh.
            commands::get_preflight,
            // SPEC.md §11 "Launch line" / plan 060 — an addition to the frozen §7 list, never a
            // rename/reshape. It reads local git refs only: no network, and no write of any kind.
            commands::get_vcs_status,
            // SPEC.md §18 / plan 053 — additions to the frozen §7 list, never a rename/reshape.
            //
            // Plan 058: `#[cfg]` here is NOT a §7 violation. §7 freezes the SHAPE of the API —
            // names, arguments, payloads — and CLAUDE.md explicitly permits implementing subsets;
            // a build configuration that omits a command is a subset, exactly as M1 was. What §7
            // forbids is a command under a different name or a different shape, and none of that
            // happens here. `default = ["github"]`, so the shipped binary ALWAYS has all three;
            // only the Windows compile-check build omits them, and it never runs a webview.
            // (`tauri::generate_handler!` parses outer attributes per command and applies them to
            // the generated match arm — see tauri-macros' `CommandDef`.)
            #[cfg(feature = "github")]
            commands::get_github_status,
            #[cfg(feature = "github")]
            commands::set_github_token,
            #[cfg(feature = "github")]
            commands::remove_github_token,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        // SPEC.md §8, quit interception path 2 of 2. `CloseRequested` alone is NOT enough: macOS
        // Cmd+Q and the app menu's Quit never close a window, they request an exit — an app that
        // only handles the first path leaks every process tree on the most common macOS quit.
        .run(|app, event| {
            if let RunEvent::ExitRequested { api, .. } = event {
                if intercept_quit(app) {
                    api.prevent_exit();
                }
            }
        });
}

/// Returns true when the quit must be blocked because trees are still alive (or the confirm flow is
/// already running). Both interception paths funnel through here, so they can never disagree.
///
/// Deliberately cheap and synchronous: it is called on the event loop thread. Everything that could
/// block — reading the runtime map behind an async mutex, the dialog, the kills — happens in the
/// spawned task.
fn intercept_quit(app: &AppHandle) -> bool {
    let state = app.state::<commands::AppState>();

    // The cleanup already ran: this is the `app.exit(0)` that follows it, and it must pass through.
    if state.cleanup_done.load(Ordering::SeqCst) {
        return false;
    }
    // A confirm is already open, or the kills are running. Keep blocking, start nothing new.
    if state.quit_in_flight.swap(true, Ordering::SeqCst) {
        return true;
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        confirm_then_quit(app).await;
    });
    true
}

async fn confirm_then_quit(app: AppHandle) {
    let running = run::stoppable_projects(&app).await;

    // Nothing alive — quit immediately, no dialog. (Reached whenever the app is closed normally.)
    if running.is_empty() {
        finish_quit(&app);
        return;
    }

    if !confirm_quit(&app, &running).await {
        app.state::<commands::AppState>()
            .quit_in_flight
            .store(false, Ordering::SeqCst);
        return;
    }

    // §8: kill all trees, phase children included, before the app goes away.
    run::stop_all(&app).await;
    finish_quit(&app);
}

/// The dialog plugin's **async** confirm. SPEC.md §8 is explicit that a blocking dialog API must
/// never be called on the main thread; `show` takes a callback and returns immediately, and the
/// oneshot turns that back into something this task can await.
async fn confirm_quit(app: &AppHandle, running: &[(String, String)]) -> bool {
    let message = match running {
        [(_, name)] => format!("{name} is still running. Stop it and quit Hangar?"),
        many => format!(
            "{} projects are still running ({}). Stop them and quit Hangar?",
            many.len(),
            many.iter()
                .map(|(_, name)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .message(message)
        .title("Quit Hangar?")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Stop and quit".to_string(),
            "Cancel".to_string(),
        ))
        .show(move |confirmed| {
            let _ = tx.send(confirmed);
        });

    // A dropped sender (the dialog went away without answering) reads as "don't quit" — the safe
    // direction, since the alternative is killing servers the user never agreed to kill.
    rx.await.unwrap_or(false)
}

/// Latch the cleanup, then ask to exit again — this time `intercept_quit` waves it through.
fn finish_quit(app: &AppHandle) {
    app.state::<commands::AppState>()
        .cleanup_done
        .store(true, Ordering::SeqCst);
    app.exit(0);
}
