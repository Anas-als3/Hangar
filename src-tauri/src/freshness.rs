//! SPEC.md §11 "Build freshness" (added 2026-08-11, plan 063) — the one line that tells the
//! maintainer the window in front of him is not running the build he just installed.
//!
//! Hangar is developed on the machine it runs on and installed to `/Applications`, so the loop is
//! change code → `npm run install:app` → the `.app` on disk is new **but the running process is
//! not**: macOS keeps a running app's code in memory, and replacing the bundle does nothing to the
//! window already open. That has three times produced the conclusion *"the feature was not built"*
//! — the most expensive wrong conclusion available, because it sends someone debugging code that
//! is correct. This module exists so the app just says.
//!
//! # THE ONE FAILURE THIS MUST NOT HAVE: a false nag
//!
//! If the line ever shows on a freshly restarted, current app it is **worse than absent** — the
//! user learns to ignore it, and the next real one is ignored too. Every rule below is a
//! consequence of that:
//!
//! - **`Ok`, never `Err`.** A missing bundle path, an unreadable file, an app launched from
//!   somewhere unexpected: all mean *say nothing*. This runs on the startup path, where an error
//!   would be a toast on every launch.
//! - **Both facts are captured before anything can move them.** [`capture_running_build`] runs once
//!   in `main.rs`'s `setup`, while the executable we were launched from is still the executable on
//!   disk.
//! - **A tolerance, never a strict `>`** — see [`TOLERANCE`].
//! - **Text only.** Nothing here restarts, kills, downloads or checks a version against a server:
//!   SPEC.md §3 bans auto-update outright, and §8's whole guarantee is that Hangar owns its
//!   children's lifecycle, so a "Restart now" button that silently killed a running dev server
//!   would be a §6/§8 violation wearing a convenience hat. The user restarts.
//! - **No network of any kind.** Two `stat`s and a constant.
//!
//! # Which "when was this built" source, and why the alternatives lose
//!
//! Two facts are combined, and the combination is what makes the check honest:
//!
//! 1. `HANGAR_BUILD_UNIX_TIME`, stamped by `build.rs` at compile time.
//! 2. The mtime of this process's own executable, **read once at startup**.
//!
//! [`running_build_at`] takes the **later** of the two, and that is compared against the mtime of
//! the same path read again later.
//!
//! - The **running binary's mtime at check time** is rejected: `install:app` copies over that exact
//!   path, so a later read returns the *new* build's mtime as though it were ours and the check
//!   could never fire (`build.rs` carries the same note).
//! - The **compile-time constant alone** is rejected, and this is the subtle one: on any machine
//!   where the bundle was copied into place *after* it was built — every downloaded release, and
//!   every `cp -R` install here — the bundle's mtime is legitimately later than the build stamp, by
//!   the length of the bundle-and-copy step or by days. Alone, that constant would put a
//!   **permanent** "a newer build is installed" on a perfectly current app, which is exactly the
//!   plan's "on a machine where the app was not installed from a local build it is silent forever",
//!   inverted. Taking the later of the two erases that class of false nag entirely: the reference
//!   point becomes *this* install, whenever it happened.
//! - The **startup mtime alone** would be enough on this machine, but the constant still earns its
//!   place as a floor: an install that preserves timestamps (`cp -p`, `rsync -a`, an archive
//!   extraction) can hand us an mtime older than the build it contains, and the floor keeps us from
//!   claiming to be older than we are.
//!
//! # macOS only
//!
//! [`bundle_executable`] has a `#[cfg(not(target_os = "macos"))]` arm that returns `None`, so the
//! answer on Windows and Linux is always "say nothing". **That is a deliberate stub, not an
//! oversight** — plan 063 scopes this to macOS, where the `.app`-replacement behaviour that causes
//! the confusion lives. The line is silent there rather than wrong.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, UNIX_EPOCH};

use serde::Serialize;

/// The answer to "am I running an old build", as one wire value. SPEC.md §7 addition (plan 063) —
/// an addition to the frozen list, never a rename or a reshape of anything in it.
///
/// There is deliberately **no field that could carry an action**: no download URL, no version, no
/// pid, no "restart" token. The only thing the frontend can do with this is render a sentence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildFreshness {
    /// `false` is the common, quiet case — and the answer to every failure, on every platform other
    /// than macOS, and on any machine where the app was not installed from a local build.
    pub newer_build_installed: bool,
    /// ISO — when the bundle on disk was written. Only ever `Some` alongside
    /// `newer_build_installed: true`, and only so the line can say how old the waiting build is;
    /// nothing acts on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_at: Option<String>,
}

/// **Why a tolerance and not a strict `>`.** The two timestamps do not share a clock origin: one is
/// a number a build script wrote into the binary, the other is a filesystem mtime, and they are
/// separated by however long linking, bundling, signing and copying took. A few seconds of skew
/// must not produce a permanent nag.
///
/// **Why two minutes.** The two error directions are not symmetric — too small is a false nag,
/// which teaches the user to ignore the line forever; too large only delays a report the user can
/// still get by restarting. So it is sized to comfortably swallow the skew (whole-second mtime
/// granularity, the gap between process launch and the startup capture, a small NTP step) and
/// nothing more. It cannot swallow a real report: `install:app` is a full `tauri build` — a vite
/// build, a release compile, a bundle and a copy — so a *new* build can never land within two
/// minutes of the moment the previous one started running.
pub const TOLERANCE: Duration = Duration::from_secs(120);

// ---------------------------------------------------------------------------------------------
// The pure half — a function of two timestamps and a tolerance, tested without a filesystem.
// ---------------------------------------------------------------------------------------------

/// The whole decision, in one place: is the build on disk newer than the build we are running, by
/// **more than** the tolerance?
///
/// `None` on either side means the fact was not established — an unreadable file, a path outside a
/// `.app`, a non-macOS platform — and an unestablished fact is never evidence of staleness. Both
/// arms return `false`, which is "say nothing".
///
/// `saturating_sub` is what makes "installed older" fall out for free: it clamps at zero rather
/// than wrapping a `u64` into an enormous positive difference.
pub fn is_newer_build_installed(
    running_build_at: Option<u64>,
    installed_at: Option<u64>,
    tolerance: Duration,
) -> bool {
    let (Some(running), Some(installed)) = (running_build_at, installed_at) else {
        return false;
    };
    installed.saturating_sub(running) > tolerance.as_secs()
}

/// Whether a path looks like `…/<Name>.app/Contents/MacOS/<binary>`.
///
/// Pure, so the guard is testable without a bundle. It exists for two reasons, and the second is
/// the important one:
///
/// - "Launched from somewhere unexpected" is a *say nothing* case (plan 063), and this is what
///   makes that decidable.
/// - Under `tauri dev` the executable is `target/debug/hangar`, which **every** `cargo build`
///   rewrites underneath the running app. Without this guard, a normal development session would
///   raise the line constantly — a nag machine, and the exact way a real warning gets trained out
///   of a user.
pub fn is_inside_app_bundle(exe: &Path) -> bool {
    let tail: Vec<&str> = exe
        .components()
        .rev()
        .take(4)
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    // Reversed: [binary, "MacOS", "Contents", "<Name>.app"]. A non-UTF-8 component drops out of the
    // `filter_map` and shortens the vector, which fails the length check — silent, like every other
    // thing this module cannot establish.
    tail.len() == 4 && tail[1] == "MacOS" && tail[2] == "Contents" && tail[3].ends_with(".app")
}

// ---------------------------------------------------------------------------------------------
// The filesystem half
// ---------------------------------------------------------------------------------------------

/// Captured once at startup and never again — see [`capture_running_build`].
static RUNNING_BUILD_AT: OnceLock<Option<u64>> = OnceLock::new();

/// The compile-time stamp from `build.rs`. `option_env!` rather than `env!` so a build without the
/// script's output is a missing fact, not a compile error.
fn compile_time_build_at() -> Option<u64> {
    option_env!("HANGAR_BUILD_UNIX_TIME").and_then(|s| s.parse::<u64>().ok())
}

/// The executable this process was launched from, but only when it sits inside a `.app`.
///
/// `current_exe()` is not canonicalised into a "resolved once" handle here on purpose: we want the
/// **path**, so that stat-ing it later reads whatever file now lives there — that replacement is
/// the entire signal.
#[cfg(target_os = "macos")]
fn bundle_executable() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    if !is_inside_app_bundle(&exe) {
        return None;
    }
    Some(exe)
}

/// **A deliberate stub, not an oversight.** Plan 063 scopes this feature to macOS: the confusion it
/// exists to end comes from macOS keeping a running `.app`'s code in memory while `install:app`
/// replaces the bundle underneath it. Windows and Linux therefore answer "say nothing" — never a
/// guess, never an error. Implementing them means deciding what "the installed bundle" even is
/// there, which is a decision, not an omission.
#[cfg(not(target_os = "macos"))]
fn bundle_executable() -> Option<PathBuf> {
    None
}

/// One `stat`, of a path the kernel already resolved in order to run us. `None` on any failure.
fn modified_unix_secs(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// When the build we are *running* was made or installed — the later of the compile-time stamp and
/// the executable's mtime. See this module's header for why it is the later of the two and not
/// either one alone.
///
/// `None` when the executable's mtime could not be read at all: the constant is a floor, never a
/// standalone reference point, so without a startup mtime this module stays silent forever.
fn running_build_at() -> Option<u64> {
    let installed_at_startup = bundle_executable().as_deref().and_then(modified_unix_secs)?;
    Some(installed_at_startup.max(compile_time_build_at().unwrap_or(0)))
}

/// Call **once**, as early as possible in `main.rs`'s `setup` — before anything can replace the
/// bundle underneath us. Idempotent: a second call is ignored rather than overwriting the first
/// reading with one taken after an install.
pub fn capture_running_build() {
    let _ = RUNNING_BUILD_AT.set(running_build_at());
}

/// The check itself. Never returns an error and never panics; every unknown is `false`.
///
/// Cheap enough to run whenever the user looks at the window: one `stat` of the running
/// executable's own path. Nothing is spawned, nothing is read from the network, nothing is written.
pub fn check() -> BuildFreshness {
    // `None` if `capture_running_build` never ran (which cannot happen through `main.rs`) or if the
    // startup read failed — both are "say nothing".
    let running = RUNNING_BUILD_AT.get().copied().flatten();
    let installed = bundle_executable().as_deref().and_then(modified_unix_secs);

    if !is_newer_build_installed(running, installed, TOLERANCE) {
        return BuildFreshness::default();
    }
    BuildFreshness {
        newer_build_installed: true,
        installed_at: installed
            .map(|secs| crate::run::iso8601_utc(UNIX_EPOCH + Duration::from_secs(secs))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: Duration = Duration::from_secs(120);
    /// An arbitrary "when this build was made". Absolute values are irrelevant — only the gap is.
    const RUNNING: u64 = 1_775_000_000;

    /// Plan 063 case 1: installed newer by well past the tolerance → stale.
    ///
    /// The real report this was built from: a window running since 06:54, a build installed at
    /// 09:11.
    #[test]
    fn a_build_installed_well_after_this_one_is_stale() {
        let two_hours_seventeen = 2 * 3600 + 17 * 60;
        assert!(is_newer_build_installed(
            Some(RUNNING),
            Some(RUNNING + two_hours_seventeen),
            TOL
        ));
    }

    /// Plan 063 case 2, and **the mutation-tested one**: installed newer by less than the
    /// tolerance → **not** stale.
    ///
    /// This is the false-nag guard. Delete the tolerance — compare with a strict `>` — and this
    /// test goes red, which is the point: a one-second difference between a compile-time constant
    /// and a filesystem mtime would otherwise be a permanent nag on a perfectly current app, and a
    /// line the user has learned to ignore is worse than no line at all.
    #[test]
    fn a_difference_inside_the_tolerance_is_not_stale() {
        for skew in [1, 5, 30, 119, 120] {
            assert!(
                !is_newer_build_installed(Some(RUNNING), Some(RUNNING + skew), TOL),
                "{skew} s of skew must not nag — the tolerance is what makes this true"
            );
        }
        // …and the far side of the same boundary still reports, so the guard above cannot pass
        // vacuously against a function that simply always returns false.
        assert!(is_newer_build_installed(Some(RUNNING), Some(RUNNING + 121), TOL));
    }

    /// Plan 063 case 3: installed older → not stale. (`saturating_sub`, not a wrapping `u64`.)
    #[test]
    fn an_older_bundle_on_disk_is_not_stale() {
        assert!(!is_newer_build_installed(Some(RUNNING), Some(RUNNING - 86_400), TOL));
        assert!(!is_newer_build_installed(Some(RUNNING), Some(0), TOL));
        assert!(!is_newer_build_installed(Some(RUNNING), Some(RUNNING), TOL));
    }

    /// Plan 063 case 4: either value missing → not stale. A fact that could not be established is
    /// never evidence — an unreadable bundle, a path outside a `.app`, Windows and Linux.
    #[test]
    fn a_missing_timestamp_is_never_evidence_of_staleness() {
        assert!(!is_newer_build_installed(None, Some(RUNNING + 999_999), TOL));
        assert!(!is_newer_build_installed(Some(RUNNING), None, TOL));
        assert!(!is_newer_build_installed(None, None, TOL));
    }

    /// The `tauri dev` guard: `target/debug/hangar` is rewritten by every `cargo build`, so it must
    /// never be treated as an installed bundle.
    #[test]
    fn only_a_real_app_bundle_executable_counts() {
        assert!(is_inside_app_bundle(Path::new(
            "/Applications/Hangar.app/Contents/MacOS/hangar"
        )));
        assert!(is_inside_app_bundle(Path::new(
            "/Users/anas/Projects/Hangar/src-tauri/target/release/bundle/macos/Hangar.app/Contents/MacOS/hangar"
        )));

        assert!(!is_inside_app_bundle(Path::new("/Users/anas/Projects/Hangar/src-tauri/target/debug/hangar")));
        assert!(!is_inside_app_bundle(Path::new("/usr/local/bin/hangar")));
        assert!(!is_inside_app_bundle(Path::new("/Applications/Hangar.app/Contents/Resources/hangar")));
        assert!(!is_inside_app_bundle(Path::new("/Applications/Hangar/Contents/MacOS/hangar")));
        assert!(!is_inside_app_bundle(Path::new("hangar")));
    }

    /// The `build.rs` wiring is real. Without this, deleting the `cargo:rustc-env` line would
    /// silently turn the floor described in this module's header into `None` — no compile error, no
    /// test failure, and a class of false nag quietly back in play.
    #[test]
    fn the_compile_time_stamp_is_actually_baked_in() {
        let stamp = compile_time_build_at().expect("build.rs must emit HANGAR_BUILD_UNIX_TIME");
        // Sanity only, not a freshness assertion: after 2020 and parsed as seconds, not millis.
        assert!(stamp > 1_577_836_800, "got {stamp}");
        assert!(stamp < 100_000_000_000, "looks like milliseconds: {stamp}");
    }

    /// The wire shape: silence must serialize as one `false`, carrying nothing else. In particular
    /// `installedAt` is absent, so a frontend cannot render "a build was installed at …" for a
    /// current app by reading a field that happened to be populated.
    #[test]
    fn the_quiet_answer_carries_nothing_but_false() {
        let quiet = BuildFreshness::default();
        assert!(!quiet.newer_build_installed);
        assert_eq!(quiet.installed_at, None);

        let json = serde_json::to_string(&quiet).unwrap();
        assert_eq!(json, r#"{"newerBuildInstalled":false}"#);

        let stale = BuildFreshness {
            newer_build_installed: true,
            installed_at: Some("2026-08-11T09:11:00Z".to_string()),
        };
        let json = serde_json::to_string(&stale).unwrap();
        assert!(json.contains(r#""newerBuildInstalled":true"#), "got {json}");
        assert!(json.contains(r#""installedAt":"2026-08-11T09:11:00Z""#), "got {json}");
    }

    /// SPEC.md §3, pinned: this module reports, it never acts. There is no restart, no kill, no
    /// download and no version server here — and the `BuildFreshness` shape has no field capable of
    /// carrying one, which is the same device `vcs.rs` uses to keep a "behind" count out of its
    /// report. A field that does not exist cannot be filled by a later refactor.
    #[test]
    fn the_report_has_no_field_that_could_carry_an_action() {
        let stale = BuildFreshness {
            newer_build_installed: true,
            installed_at: Some("2026-08-11T09:11:00Z".to_string()),
        };
        let json = serde_json::to_string(&stale).unwrap();
        for forbidden in ["url", "download", "restart", "pid", "version", "command"] {
            assert!(
                !json.contains(forbidden),
                "SPEC.md §3: this report informs and nothing else — found {forbidden:?} in {json}"
            );
        }
    }
}
