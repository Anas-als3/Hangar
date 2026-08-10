//! SPEC.md §18 / plan 053 — the GitHub integration. `GithubState` is its own managed cell,
//! deliberately never nested inside `AppState.projects`/`.runtime`/`.dev_env` (SPEC.md §18: "no
//! GitHub call may hold any lock those use") — see `main.rs`'s separate `app.manage(...)` call.
//!
//! Nothing in this module runs before the grid renders: every function here is reachable only
//! from the three `#[tauri::command]`s in `commands.rs`, each invoked solely by an explicit
//! webview call — never from `main.rs`'s `setup`, never from `run.rs`'s startup path.

pub mod client;
pub mod error;
pub mod keychain;
pub mod secret;

use tokio::sync::Mutex;

use secret::Secret;

/// Resolved lazily, once per session, on first GitHub use (SPEC.md §18) — never touched at
/// startup. `resolved` latches after the first keychain read so a session never re-triggers the
/// OS permission prompt on every status check; whether a cached token is still *valid* is
/// re-checked on every `get_github_status` call instead, because connectivity/expiry/rate-limits
/// are inherently time-varying in a way "is there a keychain entry" is not.
#[derive(Default)]
pub struct SessionCache {
    pub resolved: bool,
    pub secret: Option<Secret>,
    pub keychain_denied: bool,
}

pub struct GithubState {
    pub session: Mutex<SessionCache>,
}

impl GithubState {
    pub fn new() -> Self {
        Self { session: Mutex::new(SessionCache::default()) }
    }
}

impl Default for GithubState {
    fn default() -> Self {
        Self::new()
    }
}
