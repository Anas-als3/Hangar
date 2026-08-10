//! SPEC.md §11 "Doctor" (added 2026-08-11, plan 057) — the preflight report: what Hangar can
//! already tell, before a Run, about whether a project would even start.
//!
//! # THE INVARIANT
//!
//! This module reads `.env` files. It parses key **NAMES**. A value is never retained, never
//! serialized, never logged, never rendered — and no type below has a field capable of holding
//! one. A field that exists can be filled by a later refactor; a field that does not exist cannot.
//! [`parse_env_keys`] is the single place a `.env` line is ever split, and the right-hand side is
//! never bound to a variable there. `preflight_report_never_serializes_an_env_value` in the test
//! module is the guard: it writes a distinctive fake value, builds the whole report, serializes it
//! and asserts the value appears nowhere — while also asserting the key NAME does, so it cannot
//! pass vacuously against an empty report.
//!
//! Everything here is a **snapshot**: computed when the panel opens and on Refresh, never on a
//! timer and never on the startup path. It **reports** — it writes nothing, installs nothing,
//! creates nothing, and it never gates, delays or reorders §9.
//!
//! Filesystem access is kept at the edges on purpose: [`parse_env_keys`], [`missing_env_keys`],
//! [`nvmrc_pin`] and [`version_satisfies_pin`] are pure and take `&str`, so the rules they encode
//! are tested without a temp dir.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::env_resolve::EnvMap;
use crate::process::{self, ShellKind, SpawnSpec};
use crate::registry::Project;

/// SPEC.md §11: `blocker · warning · note`. Kebab-case for the same reason `Status` is — the
/// TypeScript mirror is a string union, not a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Blocker,
    Warning,
    Note,
}

/// One line of the report. Exactly four fields, by §11: a stable id, a severity, a human sentence
/// and the file it came from. **Nothing else, and in particular nothing that could hold a `.env`
/// value** — see THE INVARIANT above. Do not add an `Option<String>` here "for the value, redacted";
/// the whole point is that there is nowhere for one to go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightFinding {
    /// Stable across calls for the same fact, so the panel can key on it.
    pub id: String,
    pub severity: Severity,
    /// One human line. Built only from key NAMES, filenames and version strings.
    pub message: String,
    /// Relative to the project folder (`.env`, `package-lock.json`, …), or the project's own path
    /// when the finding is about the folder itself.
    pub file: String,
}

/// One project's section of the panel. `findings` empty is the common, quiet case (§11: "a project
/// with nothing to report says so once").
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightReport {
    pub project_id: String,
    /// In check order (folder → env → node → install), never sorted by severity — §11, the same
    /// reason the grid and the Ports panel are never re-sorted.
    pub findings: Vec<PreflightFinding>,
    /// ISO — one timestamp shared by every report in a single `get_preflight` call.
    pub checked_at: String,
}

/// The example-env filenames §11 names, in the order they are checked.
pub const ENV_EXAMPLE_NAMES: [&str; 3] = [".env.example", ".env.sample", ".env.template"];

/// `node --version` is a one-shot read like §9 step 1's owner lookup; it gets the same 2 s budget.
const NODE_VERSION_TIMEOUT: Duration = Duration::from_secs(2);

// ---------------------------------------------------------------------------------------------
// Check 1 — env keys. THE INVARIANT lives in `parse_env_keys`.
// ---------------------------------------------------------------------------------------------

/// Every key NAME declared by one `.env`-shaped file.
///
/// **The value is never bound to a variable.** The line is split at the first `=`, the left half
/// becomes a name and the right half is never named, never read and never returned. Comments,
/// blank lines and a leading `export ` are handled the way every `.env` loader handles them; a
/// line whose left half is not a POSIX-shaped variable name is skipped rather than guessed at.
///
/// `FOO=` (an empty value) still declares `FOO` — that is the whole point of an example file.
pub fn parse_env_keys(text: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();

    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line).trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // `export FOO=bar` — dotenv's own extension, and common in checked-in example files.
        let line = line
            .strip_prefix("export ")
            .or_else(|| line.strip_prefix("export\t"))
            .unwrap_or(line)
            .trim_start();

        let Some(eq) = line.find('=') else {
            continue; // not a declaration
        };
        // THE INVARIANT, in one line: only `..eq` is ever looked at. `line[eq + 1..]` is the
        // value; it is not bound here, not returned, and must never be.
        let name = line[..eq].trim();
        if is_env_name(name) {
            keys.insert(name.to_string());
        }
    }

    keys
}

/// POSIX-shaped variable name: leading letter or `_`, then letters, digits or `_`.
fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The key NAMES an example file declares that the real `.env` does not, in sorted order. Pure:
/// both sides are file contents, so the comparison is tested without a temp dir.
pub fn missing_env_keys(example: &str, actual: &str) -> Vec<String> {
    let declared = parse_env_keys(example);
    let present = parse_env_keys(actual);
    declared.difference(&present).cloned().collect()
}

// ---------------------------------------------------------------------------------------------
// Check 2 — Node version. `.nvmrc` only; see the `engines.node` note below.
// ---------------------------------------------------------------------------------------------

/// The version pin an `.nvmrc` declares, normalised without its leading `v`.
///
/// `None` for anything Hangar cannot resolve on its own: `lts/*`, `lts/hydrogen`, `node`, `iojs`,
/// `system`, an empty file, or any other alias. Resolving those needs nvm itself (and, for
/// `lts/*`, the network) — SPEC.md §11 says Hangar never invents a policy the project never had,
/// and guessing at an alias would do exactly that.
pub fn nvmrc_pin(text: &str) -> Option<String> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))?;
    let pin = line.strip_prefix('v').or_else(|| line.strip_prefix('V')).unwrap_or(line);
    is_numeric_version(pin).then(|| pin.to_string())
}

/// Whether the Node that would actually run satisfies an `.nvmrc` pin.
///
/// nvm resolves a pin by **version prefix, on dot boundaries**: `24` and `24.18` and `24.18.0` all
/// match `v24.18.0`, while `24.1` does not. Component-wise numeric comparison, not a string
/// `starts_with`, is what makes `24.1` vs `24.18` come out right.
///
/// `None` when `running` is not a plain version (nothing to compare against) — the caller reports
/// nothing rather than guessing.
pub fn version_satisfies_pin(pin: &str, running: &str) -> Option<bool> {
    let running = running.trim();
    let running = running
        .strip_prefix('v')
        .or_else(|| running.strip_prefix('V'))
        .unwrap_or(running);
    let running_parts = numeric_components(running)?;
    let pin_parts = numeric_components(pin)?;
    if pin_parts.len() > running_parts.len() {
        return Some(false);
    }
    Some(pin_parts.iter().zip(running_parts.iter()).all(|(a, b)| a == b))
}

fn is_numeric_version(text: &str) -> bool {
    numeric_components(text).is_some()
}

/// `24.18.0` → `[24, 18, 0]`. `None` for an empty string, an empty component, a non-digit
/// component (`lts/*`, `x`, `^24`) or more than three components.
fn numeric_components(text: &str) -> Option<Vec<u64>> {
    if text.is_empty() {
        return None;
    }
    let parts: Vec<&str> = text.split('.').collect();
    if parts.len() > 3 {
        return None;
    }
    parts.iter().map(|p| p.parse::<u64>().ok()).collect()
}

/// The Node that would actually run: `node --version` through the **one** §8 spawn helper, with
/// the cached §8 dev environment — the same PATH every project child gets. Read once per
/// `get_preflight` call, never once per project: nvm's shell hook does not run under `sh -c`, so
/// the answer cannot vary with the project's `cwd`.
///
/// `None` on any failure (not on PATH, non-zero exit, timeout, unparseable output). A failure is a
/// finding for the caller to word, never an `Err` and never a panic.
pub async fn running_node_version(env: &EnvMap) -> Option<String> {
    let spec = SpawnSpec {
        command: "node --version".to_string(),
        env: env.clone(),
        // Read-only one-shot: no process group, no Job Object (§8). `kill_on_drop` so a hung
        // `node` cannot outlive the 2 s timeout — this is tokio's reaper for this helper alone,
        // never the §8 kill path.
        long_lived: false,
        kill_on_drop: true,
        shell: ShellKind::Default,
        ..SpawnSpec::default()
    };
    let spawned = process::spawn(&spec).ok()?;
    let output = tokio::time::timeout(NODE_VERSION_TIMEOUT, spawned.child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    // Lossy decode, never a hard failure on odd bytes (§8's log-pipeline rule).
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}

// ---------------------------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------------------------

/// The whole report for one project. Never returns `Err` and never panics: a missing folder, an
/// unreadable `.env` and an unhashable lockfile are all *findings* (§11 — a toast per project on
/// open would be intolerable).
///
/// `node_version` is the one `running_node_version` read the caller made for this whole call.
pub fn build_report(
    project: &Project,
    node_version: Option<&str>,
    checked_at: &str,
) -> PreflightReport {
    let dir = Path::new(&project.path);
    let mut findings = Vec::new();

    // Check 4 — the folder is gone (SPEC.md §12; the card already warns, the panel restates it so
    // one place lists everything). Every other check reads files inside this folder, so this is
    // both first and terminal: continuing would emit three more findings all saying the same thing.
    if !dir.exists() {
        findings.push(PreflightFinding {
            id: "path-missing".to_string(),
            severity: Severity::Blocker,
            message: "This project's folder no longer exists, so Run is disabled.".to_string(),
            file: project.path.clone(),
        });
        return PreflightReport {
            project_id: project.id.clone(),
            findings,
            checked_at: checked_at.to_string(),
        };
    }

    findings.extend(env_findings(dir));
    findings.extend(node_findings(dir, node_version));
    findings.extend(install_findings(dir, project.last_lockfile_hash.as_deref()));

    PreflightReport {
        project_id: project.id.clone(),
        findings,
        checked_at: checked_at.to_string(),
    }
}

/// Check 1. Runs **only** for the example files that exist — SPEC.md §11: a project with no
/// example env file is never asked about env keys.
fn env_findings(dir: &Path) -> Vec<PreflightFinding> {
    let mut findings = Vec::new();

    for name in ENV_EXAMPLE_NAMES {
        let example_path = dir.join(name);
        if !example_path.is_file() {
            continue;
        }
        // The decoded text is a local that dies with this iteration; only key NAMES escape it.
        let Some(example) = read_lossy(&example_path) else {
            findings.push(PreflightFinding {
                id: format!("env-example-unreadable:{name}"),
                severity: Severity::Note,
                message: format!("{name} could not be read, so its keys were not checked."),
                file: name.to_string(),
            });
            continue;
        };
        let declared = parse_env_keys(&example);
        if declared.is_empty() {
            continue; // documents nothing — nothing to compare
        }

        let env_path = dir.join(".env");
        if !env_path.exists() {
            // One finding, not one per key: "there is no .env at all" is a different fact from
            // "one key drifted", and N lines all repeating it would be the noise §11 forbids.
            findings.push(PreflightFinding {
                id: format!("env-file-missing:{name}"),
                severity: Severity::Warning,
                message: format!(
                    "There is no .env file, and {name} declares {} key{}.",
                    declared.len(),
                    if declared.len() == 1 { "" } else { "s" }
                ),
                file: ".env".to_string(),
            });
            continue;
        }
        let Some(actual) = read_lossy(&env_path) else {
            findings.push(PreflightFinding {
                id: format!("env-unreadable:{name}"),
                severity: Severity::Note,
                message: format!(".env could not be read, so its keys were not compared with {name}."),
                file: ".env".to_string(),
            });
            continue;
        };

        for key in missing_env_keys(&example, &actual) {
            findings.push(PreflightFinding {
                id: format!("env-key-missing:{name}:{key}"),
                severity: Severity::Warning,
                message: format!("{key} is declared in {name} but is not set in .env."),
                file: ".env".to_string(),
            });
        }
    }

    findings
}

/// Check 2. `.nvmrc` only.
///
/// **`engines.node` is deliberately NOT checked** (plan 057's STOP condition). It is a node-semver
/// *range* (`^22.22.2 || ^24.15.0 || >=26.0.0`), and evaluating one correctly needs a node-semver
/// implementation — the `semver` crate implements Cargo's dialect, which differs on exactly the
/// operators that matter. An approximate range check that is wrong is worse than no check, so this
/// reports nothing about `engines.node` rather than guessing.
fn node_findings(dir: &Path, node_version: Option<&str>) -> Vec<PreflightFinding> {
    let nvmrc_path = dir.join(".nvmrc");
    if !nvmrc_path.is_file() {
        return Vec::new(); // no pin → the check does not run at all
    }
    let Some(text) = read_lossy(&nvmrc_path) else {
        return vec![PreflightFinding {
            id: "nvmrc-unreadable".to_string(),
            severity: Severity::Note,
            message: ".nvmrc could not be read, so the Node version was not checked.".to_string(),
            file: ".nvmrc".to_string(),
        }];
    };
    let Some(pin) = nvmrc_pin(&text) else {
        return Vec::new(); // an alias Hangar cannot resolve — say nothing rather than guess
    };

    let Some(running) = node_version else {
        return vec![PreflightFinding {
            id: "node-not-found".to_string(),
            severity: Severity::Warning,
            message: format!(
                ".nvmrc asks for Node {pin}, but Hangar could not run `node` on the PATH it \
                 resolved for this machine."
            ),
            file: ".nvmrc".to_string(),
        }];
    };

    match version_satisfies_pin(&pin, running) {
        Some(false) => vec![PreflightFinding {
            id: "node-version-mismatch".to_string(),
            severity: Severity::Warning,
            message: format!(".nvmrc asks for Node {pin}, but Hangar would run {running}."),
            file: ".nvmrc".to_string(),
        }],
        // Satisfied, or a running version Hangar could not parse — either way, nothing to say.
        _ => Vec::new(),
    }
}

/// Check 3. Calls SPEC.md §9 step 3's own functions — `process::find_lockfile`,
/// `process::hash_lockfile`, `process::needs_install` — so this is the same decision the Run will
/// make, shown earlier. It is deliberately NOT a second implementation of that three-way OR.
fn install_findings(dir: &Path, last_hash: Option<&str>) -> Vec<PreflightFinding> {
    let Some((kind, lockfile)) = process::find_lockfile(dir) else {
        return Vec::new(); // §9 step 3: no lockfile at all → no hashing, no install, nothing to say
    };
    let name = lockfile
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "the lockfile".to_string());

    let Ok(current) = process::hash_lockfile(&lockfile) else {
        return vec![PreflightFinding {
            id: "lockfile-unreadable".to_string(),
            severity: Severity::Note,
            message: format!("{name} could not be read, so Hangar cannot tell whether the next Run will install."),
            file: name,
        }];
    };

    if process::needs_install(last_hash, &current, dir.join("node_modules").exists()) {
        return vec![PreflightFinding {
            id: "install-needed".to_string(),
            severity: Severity::Note,
            message: format!(
                "The next Run will install dependencies first (`{}`).",
                kind.install_command()
            ),
            file: name,
        }];
    }

    Vec::new()
}

/// Read a file as lossy UTF-8 (§8's decoding rule — never fail on odd bytes). `None` means the
/// read itself failed, which is a *finding* for the caller, never an error.
fn read_lossy(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique scratch directory under the OS temp dir — same idiom as `registry.rs`'s `scratch`,
    /// which exists to avoid adding a `tempfile` dependency.
    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hangar-preflight-test-{tag}-{}-{:?}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn project_at(path: &str) -> Project {
        Project {
            id: "p1".into(),
            name: "Scratch".into(),
            path: path.to_string(),
            command: "npm run dev".into(),
            port: 3000,
            url: None,
            update_on_run: true,
            ready_timeout_sec: 60,
            last_lockfile_hash: None,
            last_run_at: None,
            notes: None,
            stack: None,
            folder_id: None,
            folder_name: None,
            open_browser_on_ready: None,
        }
    }

    /// THE INVARIANT's guard, written before the feature it guards.
    ///
    /// Deliberately two-sided. The negative half alone (the value is absent) passes vacuously
    /// against an empty report, which is exactly the state a broken or unimplemented builder is
    /// in; the positive half (the key NAME is present) is what makes it a real red.
    ///
    /// **Both files carry a distinctive value, and that is not decoration.** The reported names
    /// come from the EXAMPLE file (`declared`), while `.env`'s names are only ever subtracted, so
    /// a guard whose example file had empty values would survive a parser broken to retain values
    /// — verified by mutation, which is exactly how this test got its second value. `.env`'s own
    /// value is still asserted absent, so a future shape that reports `.env`-only keys is covered
    /// in advance.
    #[test]
    fn preflight_report_never_serializes_an_env_value() {
        let dir = scratch("no-value-leak");
        std::fs::write(
            dir.join(".env.example"),
            // Example files routinely carry placeholder values — sometimes real ones, pasted by
            // accident. They are `.env`-shaped files and the rule covers them identically.
            "ANTHROPIC_API_KEY=sk-example-qqq-distinctive-placeholder-qqq\n\
             DATABASE_URL=postgres://qqq-distinctive-placeholder-qqq@localhost/db\n\
             PORT=3000\n",
        )
        .unwrap();
        std::fs::write(
            dir.join(".env"),
            "DATABASE_URL=postgres://zzz-distinctive-secret-zzz@localhost/db\n\
             PORT=3000\n\
             UNUSED_TOKEN=sk-live-zzz-distinctive-secret-zzz\n",
        )
        .unwrap();
        // A `.nvmrc` and a lockfile too, so every check that can produce a finding has run by the
        // time the JSON is inspected — the guard covers the WHOLE report, not just check 1.
        std::fs::write(dir.join(".nvmrc"), "18.20.4\n").unwrap();
        std::fs::write(dir.join("package-lock.json"), "{}").unwrap();

        let project = project_at(&dir.to_string_lossy());
        let report = build_report(&project, Some("v24.18.0"), "2026-08-11T09:00:00Z");
        let json = serde_json::to_string(&report).unwrap();

        assert!(
            json.contains("ANTHROPIC_API_KEY"),
            "the key NAME must be reported, or this guard passes vacuously: {json}"
        );
        assert!(
            !json.contains("zzz-distinctive-secret-zzz"),
            "a .env VALUE reached the serialized report — THE INVARIANT is broken: {json}"
        );
        assert!(
            !json.contains("qqq-distinctive-placeholder-qqq"),
            "an example-file VALUE reached the serialized report — THE INVARIANT is broken: {json}"
        );
        assert!(
            !json.contains("postgres://") && !json.contains("sk-live") && !json.contains("sk-example"),
            "a value reached the serialized report — THE INVARIANT is broken: {json}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_key_declared_in_the_example_and_absent_from_env_is_found() {
        let dir = scratch("missing-key");
        std::fs::write(dir.join(".env.example"), "ANTHROPIC_API_KEY=\nPORT=\n").unwrap();
        std::fs::write(dir.join(".env"), "PORT=3000\n").unwrap();

        let report = build_report(
            &project_at(&dir.to_string_lossy()),
            Some("v24.18.0"),
            "2026-08-11T09:00:00Z",
        );

        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        let finding = &report.findings[0];
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(finding.id, "env-key-missing:.env.example:ANTHROPIC_API_KEY");
        assert!(finding.message.contains("ANTHROPIC_API_KEY"));
        assert_eq!(finding.file, ".env");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_example_file_means_the_env_check_does_not_run_at_all() {
        let dir = scratch("no-example");
        // A .env full of keys, and no example to compare it against: SPEC.md §11 — Hangar must not
        // invent a policy this project never had.
        std::fs::write(dir.join(".env"), "ONLY_HERE=1\n").unwrap();

        let report = build_report(
            &project_at(&dir.to_string_lossy()),
            Some("v24.18.0"),
            "2026-08-11T09:00:00Z",
        );
        assert!(report.findings.is_empty(), "{:?}", report.findings);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn comments_blank_lines_export_and_empty_values_all_parse_to_the_right_key_set() {
        let keys = parse_env_keys(
            "# FOO=bar\n\
             \n\
             REAL_KEY=value\n\
             export EXPORTED=value\n\
             EMPTY=\n\
             # a plain comment\n\
             \tSPACED = value\n\
             not a declaration\n\
             1BAD=value\n\
             WITH_EQUALS=a=b=c\n",
        );

        let expected: BTreeSet<String> = ["REAL_KEY", "EXPORTED", "EMPTY", "SPACED", "WITH_EQUALS"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(keys, expected);
        assert!(!keys.contains("FOO"), "a commented-out key is not declared");
        assert!(!keys.contains("1BAD"), "not a POSIX-shaped name");
    }

    #[test]
    fn missing_env_keys_compares_names_only_and_ignores_extras() {
        let missing = missing_env_keys(
            "A=\nB=\nC=\n",
            "B=whatever\nZ=extra\n",
        );
        assert_eq!(missing, vec!["A".to_string(), "C".to_string()]);
    }

    #[test]
    fn a_project_path_that_does_not_exist_yields_one_blocker_and_no_panic() {
        let missing = std::env::temp_dir().join("hangar-preflight-test-definitely-not-here");
        let _ = std::fs::remove_dir_all(&missing);

        let report = build_report(
            &project_at(&missing.to_string_lossy()),
            Some("v24.18.0"),
            "2026-08-11T09:00:00Z",
        );

        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].id, "path-missing");
        assert_eq!(report.findings[0].severity, Severity::Blocker);
    }

    #[test]
    fn a_clean_project_reports_nothing() {
        let dir = scratch("clean");
        std::fs::write(dir.join(".env.example"), "PORT=\n").unwrap();
        std::fs::write(dir.join(".env"), "PORT=3000\n").unwrap();
        std::fs::write(dir.join(".nvmrc"), "24.18.0\n").unwrap();
        // A lockfile whose hash is already stored, with node_modules present — §9 step 3's
        // "none of the above" branch, so `needs_install` is false.
        let lockfile = dir.join("package-lock.json");
        std::fs::write(&lockfile, "{}").unwrap();
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        let hash = process::hash_lockfile(&lockfile).unwrap();

        let mut project = project_at(&dir.to_string_lossy());
        project.last_lockfile_hash = Some(hash);

        let report = build_report(&project, Some("v24.18.0"), "2026-08-11T09:00:00Z");
        assert!(report.findings.is_empty(), "{:?}", report.findings);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_env_file_is_one_finding_not_one_per_declared_key() {
        let dir = scratch("no-env-at-all");
        std::fs::write(dir.join(".env.example"), "A=\nB=\nC=\n").unwrap();

        let report = build_report(
            &project_at(&dir.to_string_lossy()),
            Some("v24.18.0"),
            "2026-08-11T09:00:00Z",
        );

        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        assert_eq!(report.findings[0].id, "env-file-missing:.env.example");
        assert_eq!(report.findings[0].severity, Severity::Warning);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nvmrc_pins_are_read_but_aliases_are_not_guessed_at() {
        assert_eq!(nvmrc_pin("24.18.0\n").as_deref(), Some("24.18.0"));
        assert_eq!(nvmrc_pin("v22\n").as_deref(), Some("22"));
        assert_eq!(nvmrc_pin("# a comment\n20.11\n").as_deref(), Some("20.11"));
        for alias in ["lts/*\n", "lts/hydrogen\n", "node\n", "system\n", "^24.1.0\n", "\n", ""] {
            assert_eq!(nvmrc_pin(alias), None, "{alias:?} must not resolve to a pin");
        }
    }

    #[test]
    fn a_pin_matches_by_dot_boundary_prefix_the_way_nvm_resolves_it() {
        assert_eq!(version_satisfies_pin("24.18.0", "v24.18.0"), Some(true));
        assert_eq!(version_satisfies_pin("24", "v24.18.0"), Some(true));
        assert_eq!(version_satisfies_pin("24.18", "v24.18.0"), Some(true));
        // The case a string `starts_with` would get wrong.
        assert_eq!(version_satisfies_pin("24.1", "v24.18.0"), Some(false));
        assert_eq!(version_satisfies_pin("22", "v24.18.0"), Some(false));
        assert_eq!(version_satisfies_pin("24.18.0", "v24.18"), Some(false));
        // Nothing to compare against — the caller reports nothing rather than guessing.
        assert_eq!(version_satisfies_pin("24", "not-a-version"), None);
    }

    #[test]
    fn a_node_version_mismatch_is_reported_and_a_match_is_silent() {
        let dir = scratch("nvmrc");
        std::fs::write(dir.join(".nvmrc"), "22.11.0\n").unwrap();
        let project = project_at(&dir.to_string_lossy());

        let mismatch = build_report(&project, Some("v24.18.0"), "2026-08-11T09:00:00Z");
        assert_eq!(mismatch.findings.len(), 1, "{:?}", mismatch.findings);
        assert_eq!(mismatch.findings[0].id, "node-version-mismatch");
        assert_eq!(mismatch.findings[0].severity, Severity::Warning);

        let matched = build_report(&project, Some("v22.11.0"), "2026-08-11T09:00:00Z");
        assert!(matched.findings.is_empty(), "{:?}", matched.findings);

        // `node` not on the resolved PATH at all, with a pin present.
        let absent = build_report(&project, None, "2026-08-11T09:00:00Z");
        assert_eq!(absent.findings.len(), 1, "{:?}", absent.findings);
        assert_eq!(absent.findings[0].id, "node-not-found");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_nvmrc_means_the_node_check_does_not_run_at_all() {
        let dir = scratch("no-nvmrc");
        let report = build_report(
            &project_at(&dir.to_string_lossy()),
            Some("v24.18.0"),
            "2026-08-11T09:00:00Z",
        );
        assert!(report.findings.is_empty(), "{:?}", report.findings);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_needed_is_reported_and_no_lockfile_is_silent() {
        let dir = scratch("install");
        // No lockfile at all — §9 step 3 skips hashing and installing, so preflight says nothing.
        let quiet = build_report(
            &project_at(&dir.to_string_lossy()),
            Some("v24.18.0"),
            "2026-08-11T09:00:00Z",
        );
        assert!(quiet.findings.is_empty(), "{:?}", quiet.findings);

        // A lockfile with no stored hash — §9 step 3 branch (a), so the next Run installs.
        std::fs::write(dir.join("pnpm-lock.yaml"), "lockfileVersion: 9\n").unwrap();
        let report = build_report(
            &project_at(&dir.to_string_lossy()),
            Some("v24.18.0"),
            "2026-08-11T09:00:00Z",
        );
        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        assert_eq!(report.findings[0].id, "install-needed");
        assert_eq!(report.findings[0].severity, Severity::Note);
        assert!(report.findings[0].message.contains("pnpm install"));
        assert_eq!(report.findings[0].file, "pnpm-lock.yaml");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
