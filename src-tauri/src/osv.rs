//! SPEC.md §11 "Doctor" (amended 2026-08-11, plan 059) — dependency advisories from **osv.dev**,
//! as one more *source* of §11 preflight findings. There is no new §7 command: `get_preflight`
//! already returns findings, and this adds to that list.
//!
//! # THE TWO RULES
//!
//! 1. **Off by default, and structurally unable to run when off.** This is the first request
//!    Hangar makes without the user having connected anything, so [`dependency_findings`] takes
//!    `enabled` as its first argument and returns before it has read a file, let alone built a
//!    request. `nothing_is_read_and_nothing_is_sent_when_the_setting_is_off` in the test module is
//!    the guard: it injects a sender that counts its own calls, and asserts zero with the setting
//!    off — then asserts one with the same fixture and the setting on, so it cannot pass vacuously.
//!    The `query` sender is injected for exactly that reason; the real one is [`query_osv`].
//! 2. **What is sent is package names and versions, and nothing else.** [`request_body`] is the
//!    only place a request body is built, its shape is three fields wide, and
//!    `the_request_body_carries_names_and_versions_and_nothing_else` asserts the exact bytes. No
//!    path, no project name, no machine identifier — which is what the Settings label promises the
//!    user in those words.
//!
//! # The rest of the contract
//!
//! - **Never on the startup path.** Reachable only from `get_preflight`, which the Doctor panel
//!   calls on open and on Refresh — §11's "snapshot, not a monitor".
//! - **Offline is a state, not an error** (§18's rule, reused): every failure is a `note` finding.
//!   `Err` would become a toast per §7, and a check that could not run must never read as a clean
//!   bill of health.
//! - **Bounded twice and never retried**, exactly as `github/client.rs` does it — read that file
//!   first; this deliberately does not invent a second HTTP shape.
//! - **npm only.** `pnpm-lock.yaml` and `yarn.lock` are not JSON, and parsing them needs a YAML or
//!   bespoke parser — a new dependency, which plan 059 makes an explicit STOP. Those projects get
//!   a `note` saying they were not checked, which is honest and costs nothing.
//!
//! # Feature gate
//!
//! The whole module sits behind the `osv` cargo feature (default-on) for the same reason `github`
//! exists: `reqwest` pulls `rustls -> aws-lc-sys`, whose C build script cannot build for Windows
//! from macOS, and `--no-default-features` is what keeps `cargo check --target
//! x86_64-pc-windows-msvc` reaching Hangar's own sources. A separate feature rather than reusing
//! `github` because this is not GitHub: turning `github` off must not silently also turn off an
//! unrelated check, and neither module references the other, so the two gate independently.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::preflight::{PreflightFinding, Severity};
use crate::process::{self, LockfileKind};

/// osv.dev's batched query endpoint. No API key, no registration (measured 2026-08-11).
const API_URL: &str = "https://api.osv.dev/v1/querybatch";
/// The client's own per-request timeout — same shape and reasoning as `github/client.rs`.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// The outer bound wrapping the whole send, deliberately a little larger than [`REQUEST_TIMEOUT`]
/// so THIS is the one a test can drive and assert (`github/client.rs`, SPEC.md §18).
const OUTER_TIMEOUT: Duration = Duration::from_secs(15);

/// The most packages one request may carry. A large monorepo lockfile holds thousands of entries;
/// osv.dev's own guidance is to batch, but one request still has to stay bounded. Anything past
/// this is dropped **and said out loud** as a `note` — a silent truncation reads as "you are clean"
/// when it means "I did not look".
pub const MAX_PACKAGES: usize = 1000;

/// At most this many advisory ids are spelled out in one finding; the rest are counted. §11: a
/// finding is one line.
const MAX_IDS_SHOWN: usize = 4;

/// The budget for the WHOLE dependency pass, across every project.
///
/// [`OUTER_TIMEOUT`] bounds one request; nothing bounded the sum. Requests run one project at a
/// time, and the offline short-circuit only latches on a *failure* — so a server that answers
/// slowly but successfully (12 s × 10 projects) would hold `get_preflight` open for two minutes
/// with the panel showing a loading state the whole time. Past this budget the remaining projects
/// are not asked about, and **each one says so** rather than reporting an empty, clean-looking
/// list.
pub const TOTAL_BUDGET: Duration = Duration::from_secs(45);

/// One npm package as the lockfile records it. **These two strings are the entire payload** — see
/// [`request_body`]. Ordered so the deduped set, and therefore the request, is deterministic.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Package {
    pub name: String,
    pub version: String,
}

/// What one project's lockfile turned out to be. Every variant that is not [`Scan::Packages`]
/// becomes a `note`, never an error and never silence — except [`Scan::NoLockfile`], which matches
/// §9 step 3's own "no lockfile at all → nothing to say".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scan {
    /// No lockfile in the project folder at all.
    NoLockfile,
    /// A lockfile Hangar will not parse without a new dependency (plan 059's STOP condition).
    Unsupported { file: String },
    /// `package-lock.json` could not be read from disk, or its bytes are not JSON.
    Unreadable,
    /// Valid JSON with no `packages` map — an npm 6 `lockfileVersion: 1` file.
    ///
    /// Deliberately **separate from [`Scan::Unreadable`]**: that file is a perfectly good npm
    /// lockfile and Hangar simply reads v2/v3 only. Telling someone their lockfile "could not be
    /// read" sends them hunting for a corruption that does not exist — the limitation is ours, and
    /// the message has to say so.
    UnsupportedLockfileVersion,
    /// Deduped, sorted packages, with however many the [`MAX_PACKAGES`] cap dropped.
    Packages { packages: Vec<Package>, dropped: usize },
}

/// Why a `package-lock.json` produced no package list. Two cases that read identically in code and
/// must never read identically to a user — see [`Scan::UnsupportedLockfileVersion`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockfileProblem {
    /// The bytes are not JSON at all.
    NotJson,
    /// Valid JSON, no `packages` map: npm 6's `lockfileVersion: 1`.
    NoPackagesMap,
}

// ---------------------------------------------------------------------------------------------
// The lockfile — pure, and the part plan 059 warned would be wrong first
// ---------------------------------------------------------------------------------------------

/// Every registry package a `package-lock.json` v2/v3 records, deduped and sorted.
///
/// The `packages` map is keyed by **path**, not by name: `"node_modules/foo"`,
/// `"node_modules/a/node_modules/b"`, `"node_modules/@scope/pkg"`. The name is therefore the
/// segment after the **last** `node_modules` — a naive `split('/')[1]` returns `a` for the nested
/// case and is silent about it — and a scope contributes two segments, not one. The root entry
/// (key `""`) is the user's own project and has no registry identity, so it is skipped.
///
/// Skipped as well: any key with no `node_modules` segment (a workspace member's own entry, which
/// lives on disk rather than on the registry), any `"link": true` entry (a symlink to one of
/// those), and any entry with no `version`. An aliased install records its real registry name in
/// `name`, which wins over the path when present — `npm i foo@npm:bar` must be asked about `bar`.
///
/// A dependency that did not come from a registry is skipped — see [`came_from_a_registry`].
///
/// `Err` distinguishes "not JSON" from "npm 6 lockfile", because those are one `note` each and the
/// wrong one accuses a valid file of being broken.
pub fn parse_package_lock(text: &str) -> Result<Vec<Package>, LockfileProblem> {
    let root: serde_json::Value =
        serde_json::from_str(text).map_err(|_| LockfileProblem::NotJson)?;
    let packages = root
        .get("packages")
        .and_then(|v| v.as_object())
        .ok_or(LockfileProblem::NoPackagesMap)?;

    let mut deduped = BTreeSet::new();
    for (key, entry) in packages {
        if key.is_empty() {
            continue; // the root project itself
        }
        let Some(entry) = entry.as_object() else {
            continue;
        };
        if entry.get("link").and_then(|v| v.as_bool()).unwrap_or(false) {
            continue; // a symlink to a workspace member — local, not a registry package
        }
        if !came_from_a_registry(entry) {
            continue;
        }
        let Some(version) = entry.get("version").and_then(|v| v.as_str()) else {
            continue;
        };
        // The key must live under a `node_modules` segment BEFORE `name` is consulted. A workspace
        // member's own entry (`"packages/linked"`) carries a perfectly good `name` and `version`
        // and is still not a registry package — checking `name` first lets exactly that through,
        // which is what `the_name_is_the_segment_after_the_last_node_modules` caught.
        let Some(from_path) = name_from_key(key) else {
            continue;
        };
        // Only now: an aliased install (`npm i foo@npm:bar`) records its real registry name here,
        // and `bar` is the name osv.dev must be asked about, not the directory it was installed to.
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or(from_path);
        if name.is_empty() || version.is_empty() {
            continue;
        }
        deduped.insert(Package { name, version: version.to_string() });
    }

    Ok(deduped.into_iter().collect())
}

/// Whether an entry came from a package registry, rather than from git, a local path or a link.
///
/// Skipping the others is a **correctness** fix first and a disclosure fix second. osv.dev is
/// asked by npm name and version, so a `git+ssh://…/acme/internal-thing` dependency either matches
/// nothing — pure batch noise — or matches a *public* package that merely shares its name, which
/// would report an advisory against code that is not in this project. That those same names are
/// the most sensitive strings in the lockfile is the second reason, pointing the same way.
///
/// Deliberately conservative in one direction: **an absent `resolved` is kept.** Bundled
/// dependencies omit it, and dropping real packages to be tidy would manufacture exactly the
/// quiet, clean-looking result this module exists to prevent. Only an explicitly non-registry
/// `resolved` is skipped.
///
/// A private *registry* (`https://npm.example-corp.com/…`) is deliberately NOT filtered: it cannot
/// be told apart from a public registry or a corporate mirror of one, and filtering by host would
/// silently skip every package for anyone behind a proxy. The Settings label says so instead.
fn came_from_a_registry(entry: &serde_json::Map<String, serde_json::Value>) -> bool {
    let Some(resolved) = entry.get("resolved").and_then(|v| v.as_str()) else {
        return true;
    };
    let resolved = resolved.trim();
    !["git+", "git:", "git@", "file:", "link:", "github:", "bitbucket:", "gitlab:"]
        .iter()
        .any(|prefix| resolved.starts_with(prefix))
}

/// The package name a `packages` key encodes. Splits on `/` and walks to the **last**
/// `node_modules` segment rather than doing string surgery, so both the nested case and a scope
/// (two segments, one name) come out right.
fn name_from_key(key: &str) -> Option<String> {
    let segments: Vec<&str> = key.split('/').collect();
    let last = segments.iter().rposition(|s| *s == "node_modules")?;
    let first = *segments.get(last + 1)?;
    if let Some(scope) = first.strip_prefix('@') {
        if scope.is_empty() {
            return None;
        }
        let second = segments.get(last + 2)?;
        return Some(format!("{first}/{second}"));
    }
    Some(first.to_string())
}

/// One project's lockfile, read and parsed. Blocking I/O on purpose — [`dependency_findings`]
/// runs every call of this on the blocking pool in one hop, and only after its guard.
pub fn scan_lockfile(dir: &Path) -> Scan {
    let Some((kind, path)) = process::find_lockfile(dir) else {
        return Scan::NoLockfile;
    };
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "the lockfile".to_string());

    if kind != LockfileKind::Npm {
        return Scan::Unsupported { file };
    }
    // Lossy decode, never a hard failure on odd bytes (§8's decoding rule).
    let Ok(bytes) = std::fs::read(&path) else {
        return Scan::Unreadable;
    };
    let mut packages = match parse_package_lock(&String::from_utf8_lossy(&bytes)) {
        Ok(packages) => packages,
        Err(LockfileProblem::NotJson) => return Scan::Unreadable,
        Err(LockfileProblem::NoPackagesMap) => return Scan::UnsupportedLockfileVersion,
    };

    let dropped = packages.len().saturating_sub(MAX_PACKAGES);
    packages.truncate(MAX_PACKAGES);
    Scan::Packages { packages, dropped }
}

// ---------------------------------------------------------------------------------------------
// The request body — RULE 2 lives here
// ---------------------------------------------------------------------------------------------

#[derive(serde::Serialize)]
struct QueryPackage<'a> {
    name: &'a str,
    ecosystem: &'a str,
}

#[derive(serde::Serialize)]
struct Query<'a> {
    package: QueryPackage<'a>,
    version: &'a str,
}

#[derive(serde::Serialize)]
struct Batch<'a> {
    queries: Vec<Query<'a>>,
}

/// The ONE place a request body is built. **Three fields wide: name, ecosystem, version.** There
/// is no field here for a path, a project name, a machine id or anything else, which is what makes
/// the Settings label's promise checkable rather than aspirational.
pub fn request_body(packages: &[Package]) -> String {
    let batch = Batch {
        queries: packages
            .iter()
            .map(|p| Query {
                package: QueryPackage { name: &p.name, ecosystem: "npm" },
                version: &p.version,
            })
            .collect(),
    };
    // Infallible for this shape (plain strings); an empty body would simply read as offline.
    serde_json::to_string(&batch).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// The client — one send, bounded twice, no retries (`github/client.rs`'s shape)
// ---------------------------------------------------------------------------------------------

/// osv.dev's `querybatch` reply, as observed live on 2026-08-11:
/// `{"results":[{"vulns":[{"id":"GHSA-…","modified":"…"}]},{}]}` — `results` is parallel to
/// `queries`, and a **clean package's entry is `{}`**, with `vulns` absent rather than empty. Hence
/// `#[serde(default)]`: reading it as a required field would fail on every clean package, i.e. on
/// the common case. `modified` is deliberately not deserialized — nothing renders it.
#[derive(serde::Deserialize)]
struct BatchResponse {
    #[serde(default)]
    results: Vec<BatchResult>,
}

#[derive(serde::Deserialize)]
struct BatchResult {
    #[serde(default)]
    vulns: Vec<VulnId>,
}

#[derive(serde::Deserialize)]
struct VulnId {
    id: String,
}

/// The outer half of the two timeouts, parametrised on the duration so a test can drive it with a
/// very short bound instead of a real network call — same idiom, and the same reason, as
/// `github/client.rs`'s `bounded_with`.
async fn bounded_with<F, T>(duration: Duration, fut: F) -> Option<T>
where
    F: std::future::Future<Output = Option<T>>,
{
    tokio::time::timeout(duration, fut).await.ok().flatten()
}

/// The real sender: **one** POST, bounded twice, no retries. `None` for every failure — offline,
/// timeout, non-2xx, unparseable body, or a reply whose length does not match the query — because
/// all of them mean the same thing to the user: no answer, so nothing was checked.
///
/// Returns the advisory ids per package, parallel to `packages`.
pub async fn query_osv(packages: Vec<Package>) -> Option<Vec<Vec<String>>> {
    let client = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build().ok()?;
    let body = request_body(&packages);

    let response = bounded_with(OUTER_TIMEOUT, async {
        client
            .post(API_URL)
            .header(reqwest::header::USER_AGENT, "Hangar")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .ok()
    })
    .await?;

    if !response.status().is_success() {
        return None;
    }
    let parsed: BatchResponse = response.json().await.ok()?;
    if parsed.results.len() != packages.len() {
        return None; // not the contract this was written against — say nothing rather than guess
    }
    Some(
        parsed
            .results
            .into_iter()
            .map(|r| r.vulns.into_iter().map(|v| v.id).collect())
            .collect(),
    )
}

// ---------------------------------------------------------------------------------------------
// The findings — RULE 1's guard is the first statement below
// ---------------------------------------------------------------------------------------------

/// One list of extra findings per entry in `project_dirs`, in the same order, for the caller to
/// append to that project's report.
///
/// `query` is injected so the off-by-default guard can be proved by a test that counts sends
/// rather than by reading this function. Production passes [`query_osv`].
///
/// One request per project, run in order, with **two ways to stop early**: the first `None`
/// latches `offline`, and `budget` caps the whole pass however well the server is answering. Both
/// exist so a slow or dead network cannot hold `get_preflight` open for minutes — and in both
/// cases every remaining project **says it was not checked**.
///
/// **No path out of this function returns an empty list for a project it did not actually check.**
/// That is the one bug the whole module is written to prevent: an empty finding list renders
/// exactly like "checked and clean" (§11 is silent when clean), so "I could not look" must always
/// produce a `note`.
pub async fn dependency_findings<F, Fut>(
    enabled: bool,
    project_dirs: Vec<PathBuf>,
    budget: Duration,
    query: F,
) -> Vec<Vec<PreflightFinding>>
where
    F: Fn(Vec<Package>) -> Fut,
    Fut: std::future::Future<Output = Option<Vec<Vec<String>>>>,
{
    let count = project_dirs.len();
    // THE GUARD (RULE 1). Nothing below this line runs when the user has not opted in — not the
    // lockfile read, not the request. Deleting it is what the off-by-default test catches.
    if !enabled {
        return vec![Vec::new(); count];
    }

    // The lockfile reads are blocking I/O and a project may sit on a stalled network mount — the
    // same reasoning `get_preflight` applies to `build_report`, so they ride one blocking hop too.
    let Ok(scans) =
        tauri::async_runtime::spawn_blocking(move || -> Vec<Scan> {
            project_dirs.iter().map(|dir| scan_lockfile(dir)).collect()
        })
        .await
    else {
        // The blocking hop itself failed, so NOTHING was read and nothing was asked. Returning
        // empty lists here would render as "checked, nothing to report" for every project — the
        // clean bill of health this module must never hand out. One note each instead.
        //
        // The branch CONDITION is a `JoinError`, which a test cannot provoke without putting test
        // scaffolding in production code; its BODY is the named function below, which is tested.
        return unavailable_for_every_project(count);
    };

    let started = std::time::Instant::now();
    let mut per_project = Vec::with_capacity(count);
    let mut offline = false;

    for scan in scans {
        let mut findings = Vec::new();
        let packages = match scan {
            Scan::NoLockfile => {
                per_project.push(findings);
                continue;
            }
            Scan::Unsupported { file } => {
                findings.push(PreflightFinding {
                    id: "dependency-check-unsupported".to_string(),
                    severity: Severity::Note,
                    message: format!(
                        "Dependency advisories are only read from package-lock.json, so {file} was \
                         not checked."
                    ),
                    file,
                });
                per_project.push(findings);
                continue;
            }
            Scan::Unreadable => {
                findings.push(PreflightFinding {
                    id: "dependency-lockfile-unreadable".to_string(),
                    severity: Severity::Note,
                    message: "package-lock.json could not be read, so dependencies were not \
                              checked."
                        .to_string(),
                    file: "package-lock.json".to_string(),
                });
                per_project.push(findings);
                continue;
            }
            // The file is FINE — the limit is Hangar's. Say that, and say which versions are read.
            Scan::UnsupportedLockfileVersion => {
                findings.push(PreflightFinding {
                    id: "dependency-lockfile-version-unsupported".to_string(),
                    severity: Severity::Note,
                    message: "package-lock.json is an npm 6 lockfile (version 1); Hangar reads \
                              lockfile versions 2 and 3, so dependencies were not checked."
                        .to_string(),
                    file: "package-lock.json".to_string(),
                });
                per_project.push(findings);
                continue;
            }
            Scan::Packages { packages, dropped } => {
                if dropped > 0 {
                    // Said before the advisories, because it qualifies every one of them.
                    findings.push(PreflightFinding {
                        id: "dependency-check-truncated".to_string(),
                        severity: Severity::Note,
                        message: format!(
                            "Only the first {MAX_PACKAGES} packages in package-lock.json were \
                             checked; {dropped} more were not."
                        ),
                        file: "package-lock.json".to_string(),
                    });
                }
                packages
            }
        };

        if packages.is_empty() {
            per_project.push(findings);
            continue;
        }
        if offline {
            findings.push(unavailable());
            per_project.push(findings);
            continue;
        }
        // The pass has run out of time. Every project from here on is honestly unchecked, and says
        // so in its own words — "ran out of time" is a different fact from "no answer".
        if started.elapsed() >= budget {
            findings.push(out_of_time());
            per_project.push(findings);
            continue;
        }

        match query(packages.clone()).await {
            None => {
                offline = true;
                findings.push(unavailable());
            }
            Some(results) => {
                for (package, ids) in packages.iter().zip(results) {
                    if ids.is_empty() {
                        continue; // §11 "silent when clean"
                    }
                    findings.push(advisory_finding(package, &ids));
                }
            }
        }
        per_project.push(findings);
    }

    per_project
}

/// The one wording for "no answer". Covers unreachable, timed out, throttled and malformed alike:
/// they differ to a developer and not at all to the person reading the panel, and every one of
/// them means the same dangerous thing — **this is not a clean bill of health**.
fn unavailable() -> PreflightFinding {
    PreflightFinding {
        id: "dependency-check-unavailable".to_string(),
        severity: Severity::Note,
        message: "Hangar could not get an answer from osv.dev, so dependencies were not checked."
            .to_string(),
        file: "package-lock.json".to_string(),
    }
}

/// The whole pass could not start. **Never an empty list**: `Vec::new()` for a project renders
/// identically to "checked, nothing to report" (§11 is silent when clean), so a pass that read
/// nothing must say so once per project.
fn unavailable_for_every_project(count: usize) -> Vec<Vec<PreflightFinding>> {
    vec![vec![unavailable()]; count]
}

/// The other half of "it was not checked": the pass hit [`TOTAL_BUDGET`] before reaching this
/// project. Worded distinctly from [`unavailable`] because the causes differ — one is a network
/// that will not answer, the other is too many projects for one panel open — and a user who sees
/// this can act on it (open Doctor again, or check fewer projects).
fn out_of_time() -> PreflightFinding {
    PreflightFinding {
        id: "dependency-check-out-of-time".to_string(),
        severity: Severity::Note,
        message: "The dependency check ran out of time before reaching this project, so its \
                  dependencies were not checked."
            .to_string(),
        file: "package-lock.json".to_string(),
    }
}

/// **`warning`, never `blocker`** — §11's `blocker` means "will not start", and a CVE in a
/// transitive dev dependency does not stop the project starting.
fn advisory_finding(package: &Package, ids: &[String]) -> PreflightFinding {
    let shown: Vec<&str> = ids.iter().take(MAX_IDS_SHOWN).map(String::as_str).collect();
    let rest = ids.len().saturating_sub(shown.len());
    let listed = shown.join(", ");
    let listed = if rest > 0 { format!("{listed} and {rest} more") } else { listed };

    PreflightFinding {
        id: format!("dependency-advisory:{}@{}", package.name, package.version),
        severity: Severity::Warning,
        message: format!(
            "{} {} has {} known {}: {listed}.",
            package.name,
            package.version,
            ids.len(),
            if ids.len() == 1 { "advisory" } else { "advisories" }
        ),
        file: "package-lock.json".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Same scratch idiom as `preflight.rs` and `registry.rs` — a unique temp dir instead of a
    /// `tempfile` dependency. Never the user's own project folders or the app-data dir.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hangar-osv-test-{tag}-{}-{:?}",
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

    /// A `lockfileVersion: 3` file with one ordinary dependency, written into `dir`.
    fn write_npm_lockfile(dir: &Path) {
        std::fs::write(
            dir.join("package-lock.json"),
            r#"{
              "name": "scratch",
              "lockfileVersion": 3,
              "packages": {
                "": { "name": "scratch", "version": "1.0.0" },
                "node_modules/lodash": { "version": "4.17.15" }
              }
            }"#,
        )
        .unwrap();
    }

    // -----------------------------------------------------------------------------------------
    // RULE 1 — off by default, proved by counting sends
    // -----------------------------------------------------------------------------------------

    /// **The guard for plan 059's first hard rule.** Deliberately two-sided: the negative half
    /// alone (nothing was sent) passes vacuously against a fixture that would never have produced a
    /// request anyway, so the SAME directory and the SAME sender are run again with the setting on
    /// and asserted to send exactly once and to report the advisory. Delete the `if !enabled`
    /// guard in `dependency_findings` and the first assertion fails.
    #[test]
    fn nothing_is_read_and_nothing_is_sent_when_the_setting_is_off() {
        let dir = scratch("off-by-default");
        write_npm_lockfile(&dir);

        let sends = AtomicUsize::new(0);
        let sender = |packages: Vec<Package>| {
            sends.fetch_add(1, Ordering::SeqCst);
            async move { Some(vec![vec!["GHSA-test-0000".to_string()]; packages.len()]) }
        };

        let off = tauri::async_runtime::block_on(dependency_findings(
            false,
            vec![dir.clone()],
            TOTAL_BUDGET,
            &sender,
        ));
        assert_eq!(
            sends.load(Ordering::SeqCst),
            0,
            "a request was built with checkDependencies OFF — plan 059's first rule is broken"
        );
        assert_eq!(off.len(), 1, "one entry per project even when the check is off");
        assert!(off[0].is_empty(), "no findings when the check is off: {:?}", off[0]);

        // Not vacuous: the same fixture, the same sender, the setting ON.
        let on = tauri::async_runtime::block_on(dependency_findings(
            true,
            vec![dir.clone()],
            TOTAL_BUDGET,
            &sender,
        ));
        assert_eq!(sends.load(Ordering::SeqCst), 1, "exactly one request, and no retries");
        assert_eq!(on[0].len(), 1, "{:?}", on[0]);
        assert_eq!(on[0][0].severity, Severity::Warning);
        assert_eq!(on[0][0].id, "dependency-advisory:lodash@4.17.15");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------------------------
    // RULE 2 — exactly what is sent
    // -----------------------------------------------------------------------------------------

    /// The Settings label promises "package names and versions … nothing else". This is that
    /// promise as bytes: the whole body, asserted literally.
    #[test]
    fn the_request_body_carries_names_and_versions_and_nothing_else() {
        let body = request_body(&[
            Package { name: "lodash".into(), version: "4.17.15".into() },
            Package { name: "@scope/pkg".into(), version: "0.1.0".into() },
        ]);
        assert_eq!(
            body,
            r#"{"queries":[{"package":{"name":"lodash","ecosystem":"npm"},"version":"4.17.15"},{"package":{"name":"@scope/pkg","ecosystem":"npm"},"version":"0.1.0"}]}"#
        );
    }

    // -----------------------------------------------------------------------------------------
    // The lockfile parser — the part plan 059 said would be wrong first
    // -----------------------------------------------------------------------------------------

    /// The nested case a naive `split('/')[1]` gets wrong (it would answer `a`), plus a scope
    /// (two path segments, one name), the root `""` entry, a workspace link, a member's own
    /// non-`node_modules` entry, an alias, and a versionless entry.
    #[test]
    fn the_name_is_the_segment_after_the_last_node_modules() {
        let packages = parse_package_lock(
            r#"{
              "lockfileVersion": 3,
              "packages": {
                "": { "name": "the-root-project", "version": "9.9.9" },
                "node_modules/a": { "version": "1.0.0" },
                "node_modules/a/node_modules/b": { "version": "2.0.0" },
                "node_modules/@scope/pkg": { "version": "3.0.0" },
                "node_modules/a/node_modules/@deep/nested": { "version": "4.0.0" },
                "node_modules/aliased": { "name": "real-registry-name", "version": "5.0.0" },
                "node_modules/linked": { "resolved": "packages/linked", "link": true },
                "packages/linked": { "name": "linked", "version": "6.0.0" },
                "node_modules/no-version": { "resolved": "https://example.invalid/x.tgz" }
              }
            }"#,
        )
        .expect("a v3 lockfile parses");

        let got: Vec<(String, String)> =
            packages.iter().map(|p| (p.name.clone(), p.version.clone())).collect();
        assert_eq!(
            got,
            vec![
                ("@deep/nested".to_string(), "4.0.0".to_string()),
                ("@scope/pkg".to_string(), "3.0.0".to_string()),
                ("a".to_string(), "1.0.0".to_string()),
                ("b".to_string(), "2.0.0".to_string()),
                ("real-registry-name".to_string(), "5.0.0".to_string()),
            ],
            "the nested entry must be `b`, never `a`"
        );
        assert!(
            !packages.iter().any(|p| p.name == "the-root-project"),
            "the root entry has no registry identity and must be skipped"
        );
        assert!(
            !packages.iter().any(|p| p.name == "linked"),
            "a workspace member is local, not a registry package"
        );
    }

    /// The same name at two depths is one query, not two.
    #[test]
    fn identical_packages_at_different_paths_are_deduped() {
        let packages = parse_package_lock(
            r#"{"packages":{
              "node_modules/a": {"version": "1.0.0"},
              "node_modules/x/node_modules/a": {"version": "1.0.0"},
              "node_modules/y/node_modules/a": {"version": "2.0.0"}
            }}"#,
        )
        .unwrap();
        assert_eq!(packages.len(), 2, "{packages:?}");
    }

    /// The two "no package list" cases are **distinct**, because the messages they produce accuse
    /// very different things: one says the file is broken, the other says Hangar is limited. An
    /// npm 6 lockfile is a perfectly valid file.
    #[test]
    fn a_broken_lockfile_and_an_npm_6_lockfile_are_told_apart() {
        assert_eq!(parse_package_lock("not json at all"), Err(LockfileProblem::NotJson));
        assert_eq!(
            parse_package_lock(r#"{"lockfileVersion":1,"dependencies":{"a":{"version":"1.0.0"}}}"#),
            Err(LockfileProblem::NoPackagesMap)
        );
        // Parseable and genuinely empty is a third, different answer.
        assert_eq!(parse_package_lock(r#"{"packages":{}}"#), Ok(Vec::new()));
    }

    /// An npm 6 lockfile must not be described as unreadable — that sends someone hunting for a
    /// corruption that does not exist. The message names the real limit instead.
    #[test]
    fn an_npm_6_lockfile_blames_hangars_limit_not_the_users_file() {
        let dir = scratch("lockfile-v1");
        std::fs::write(
            dir.join("package-lock.json"),
            r#"{"lockfileVersion":1,"dependencies":{"lodash":{"version":"4.17.15"}}}"#,
        )
        .unwrap();

        let sends = AtomicUsize::new(0);
        let findings = tauri::async_runtime::block_on(dependency_findings(
            true,
            vec![dir.clone()],
            TOTAL_BUDGET,
            |_packages: Vec<Package>| {
                sends.fetch_add(1, Ordering::SeqCst);
                async { None }
            },
        ));

        assert_eq!(sends.load(Ordering::SeqCst), 0, "there is nothing to send");
        assert_eq!(findings[0].len(), 1, "{:?}", findings[0]);
        let message = &findings[0][0].message;
        assert_eq!(findings[0][0].id, "dependency-lockfile-version-unsupported");
        assert!(
            !message.contains("could not be read"),
            "a valid npm 6 lockfile must not be called unreadable: {message}"
        );
        assert!(message.contains("versions 2 and 3"), "{message}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Plan 059 review finding 4: a git / local-path / link dependency has a `version`, no `link`
    /// flag and a `node_modules` key, so it would otherwise be sent. It must not be — asking
    /// osv.dev about `internal-thing@1.0.0` either matches nothing or, worse, matches a PUBLIC
    /// package that merely shares the name, reporting an advisory against code that is not here.
    /// An **absent** `resolved` is still kept: dropping real packages to be tidy is how a check
    /// quietly stops looking.
    #[test]
    fn dependencies_that_did_not_come_from_a_registry_are_never_sent() {
        let packages = parse_package_lock(
            r#"{"packages":{
              "node_modules/from-registry": {"version":"1.0.0","resolved":"https://registry.npmjs.org/x/-/x-1.0.0.tgz"},
              "node_modules/from-private-registry": {"version":"1.0.0","resolved":"https://npm.example-corp.com/x/-/x-1.0.0.tgz"},
              "node_modules/no-resolved-field": {"version":"1.0.0"},
              "node_modules/internal-thing": {"version":"1.0.0","resolved":"git+ssh://git@github.com/acme/internal-thing.git#abc123"},
              "node_modules/from-github-shorthand": {"version":"2.0.0","resolved":"github:acme/other"},
              "node_modules/from-a-local-path": {"version":"3.0.0","resolved":"file:../sibling"}
            }}"#,
        )
        .unwrap();

        let names: Vec<&str> = packages.iter().map(|p| p.name.as_str()).collect();
        assert!(!names.contains(&"internal-thing"), "a git dependency's name must not be sent");
        assert!(!names.contains(&"from-github-shorthand"), "{names:?}");
        assert!(!names.contains(&"from-a-local-path"), "{names:?}");
        assert!(names.contains(&"from-registry"), "{names:?}");
        assert!(
            names.contains(&"no-resolved-field"),
            "an absent `resolved` must be KEPT — dropping real packages is how coverage dies: {names:?}"
        );
        // Deliberate, and disclosed in the Settings label: a private registry cannot be told apart
        // from a public one, and filtering by host would skip everything for anyone behind a proxy.
        assert!(names.contains(&"from-private-registry"), "{names:?}");
    }

    // -----------------------------------------------------------------------------------------
    // Scanning and the findings
    // -----------------------------------------------------------------------------------------

    /// Plan 059's STOP condition, as behaviour: pnpm and yarn need a YAML/bespoke parser — a new
    /// dependency — so they report that they were not checked rather than reporting nothing.
    #[test]
    fn a_pnpm_project_says_it_was_not_checked_instead_of_reporting_nothing() {
        let dir = scratch("pnpm");
        std::fs::write(dir.join("pnpm-lock.yaml"), "lockfileVersion: 9\n").unwrap();

        let sends = AtomicUsize::new(0);
        let findings = tauri::async_runtime::block_on(dependency_findings(
            true,
            vec![dir.clone()],
            TOTAL_BUDGET,
            |_packages: Vec<Package>| {
                sends.fetch_add(1, Ordering::SeqCst);
                async { None }
            },
        ));

        assert_eq!(sends.load(Ordering::SeqCst), 0, "nothing to send for a pnpm project");
        assert_eq!(findings[0].len(), 1, "{:?}", findings[0]);
        assert_eq!(findings[0][0].id, "dependency-check-unsupported");
        assert_eq!(findings[0][0].severity, Severity::Note);
        assert_eq!(findings[0][0].file, "pnpm-lock.yaml");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// No lockfile at all is silence, exactly as §9 step 3 treats it.
    #[test]
    fn a_project_with_no_lockfile_says_nothing_and_sends_nothing() {
        let dir = scratch("no-lockfile");
        let sends = AtomicUsize::new(0);
        let findings = tauri::async_runtime::block_on(dependency_findings(
            true,
            vec![dir.clone()],
            TOTAL_BUDGET,
            |_packages: Vec<Package>| {
                sends.fetch_add(1, Ordering::SeqCst);
                async { None }
            },
        ));
        assert_eq!(sends.load(Ordering::SeqCst), 0);
        assert!(findings[0].is_empty(), "{:?}", findings[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// No answer is a `note`, never an `Err` (which §7 would turn into a toast) and never silence
    /// — and the first failure short-circuits the rest instead of paying the timeout per project.
    #[test]
    fn no_answer_is_a_note_on_every_project_and_costs_exactly_one_request() {
        let one = scratch("offline-1");
        let two = scratch("offline-2");
        write_npm_lockfile(&one);
        write_npm_lockfile(&two);

        let sends = AtomicUsize::new(0);
        let findings = tauri::async_runtime::block_on(dependency_findings(
            true,
            vec![one.clone(), two.clone()],
            TOTAL_BUDGET,
            |_packages: Vec<Package>| {
                sends.fetch_add(1, Ordering::SeqCst);
                async { None }
            },
        ));

        assert_eq!(sends.load(Ordering::SeqCst), 1, "the first failure must short-circuit");
        for project in &findings {
            assert_eq!(project.len(), 1, "{project:?}");
            assert_eq!(project[0].id, "dependency-check-unavailable");
            assert_eq!(project[0].severity, Severity::Note);
        }

        let _ = std::fs::remove_dir_all(&one);
        let _ = std::fs::remove_dir_all(&two);
    }

    /// Plan 059 review finding 1: when the pass cannot run at all, every project must SAY so.
    /// An empty finding list renders exactly like "checked and clean" (§11 is silent when clean),
    /// which would hand out a clean bill of health for work that never happened. The `JoinError`
    /// that triggers this cannot be provoked from a test without test scaffolding in production
    /// code, so the branch's body is a named function and this covers it directly.
    #[test]
    fn a_pass_that_could_not_run_never_returns_an_empty_clean_looking_list() {
        let fallback = unavailable_for_every_project(3);

        assert_eq!(fallback.len(), 3, "one entry per project");
        for project in &fallback {
            assert!(
                !project.is_empty(),
                "an empty list is indistinguishable from 'checked and clean'"
            );
            assert_eq!(project.len(), 1);
            assert_eq!(project[0].id, "dependency-check-unavailable");
            assert_eq!(project[0].severity, Severity::Note);
        }
        assert!(unavailable_for_every_project(0).is_empty(), "no projects, no findings");
    }

    /// Plan 059 review finding 2: the offline short-circuit only latches on a *failure*, so a
    /// server that answers slowly but **successfully** was unbounded across projects. The budget
    /// caps the whole pass — and, critically, the projects it did not reach say so instead of
    /// coming back with an empty list that renders exactly like "checked and clean".
    #[test]
    fn a_slow_but_working_server_still_cannot_run_past_the_total_budget() {
        let dirs: Vec<PathBuf> = (0..4)
            .map(|i| {
                let dir = scratch(&format!("budget-{i}"));
                write_npm_lockfile(&dir);
                dir
            })
            .collect();

        let sends = AtomicUsize::new(0);
        let findings = tauri::async_runtime::block_on(dependency_findings(
            true,
            dirs.clone(),
            // A budget smaller than one "request", so the first project is asked and the rest are
            // not. Real time, but 30ms of it.
            Duration::from_millis(20),
            |packages: Vec<Package>| {
                sends.fetch_add(1, Ordering::SeqCst);
                async move {
                    // Answers successfully, just slowly — the case `offline` never catches.
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    Some(vec![Vec::new(); packages.len()])
                }
            },
        ));

        assert_eq!(
            sends.load(Ordering::SeqCst),
            1,
            "the budget must stop the pass, not let every project pay the full request time"
        );
        assert!(findings[0].is_empty(), "the project that WAS checked is clean: {:?}", findings[0]);
        for project in &findings[1..] {
            assert_eq!(project.len(), 1, "an unreached project must not look clean: {project:?}");
            assert_eq!(project[0].id, "dependency-check-out-of-time");
            assert_eq!(project[0].severity, Severity::Note);
        }

        for dir in &dirs {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// A clean answer is silence — §11's "silent when clean", which is the honest current output
    /// of this whole feature.
    #[test]
    fn a_clean_answer_reports_nothing_at_all() {
        let dir = scratch("clean");
        write_npm_lockfile(&dir);

        let findings = tauri::async_runtime::block_on(dependency_findings(
            true,
            vec![dir.clone()],
            TOTAL_BUDGET,
            // The real shape of a clean reply: `{}` per package, i.e. no ids.
            |packages: Vec<Package>| async move { Some(vec![Vec::new(); packages.len()]) },
        ));
        assert!(findings[0].is_empty(), "{:?}", findings[0]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The cap is said out loud. A silent truncation reads as "you are clean" when it means "I did
    /// not look".
    #[test]
    fn the_cap_drops_packages_and_says_so() {
        let dir = scratch("capped");
        let entries: Vec<String> = (0..MAX_PACKAGES + 3)
            .map(|i| format!(r#""node_modules/pkg{i}": {{"version": "1.0.{i}"}}"#))
            .collect();
        std::fs::write(
            dir.join("package-lock.json"),
            format!(r#"{{"lockfileVersion":3,"packages":{{{}}}}}"#, entries.join(",")),
        )
        .unwrap();

        let sent = std::sync::Mutex::new(0usize);
        let findings = tauri::async_runtime::block_on(dependency_findings(
            true,
            vec![dir.clone()],
            TOTAL_BUDGET,
            |packages: Vec<Package>| {
                *sent.lock().unwrap() = packages.len();
                async move { Some(vec![Vec::new(); packages.len()]) }
            },
        ));

        assert_eq!(*sent.lock().unwrap(), MAX_PACKAGES, "one request stays bounded");
        assert_eq!(findings[0].len(), 1, "{:?}", findings[0]);
        assert_eq!(findings[0][0].id, "dependency-check-truncated");
        assert!(findings[0][0].message.contains('3'), "{}", findings[0][0].message);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Severity is `warning`, and the ids are spelled out — up to [`MAX_IDS_SHOWN`], then counted,
    /// because §11 says a finding is one line.
    #[test]
    fn an_advisory_is_a_warning_that_names_the_package_version_and_ids() {
        let package = Package { name: "lodash".into(), version: "4.17.15".into() };
        let ids: Vec<String> = (0..6).map(|i| format!("GHSA-{i}")).collect();
        let finding = advisory_finding(&package, &ids);

        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(finding.id, "dependency-advisory:lodash@4.17.15");
        assert_eq!(finding.file, "package-lock.json");
        assert_eq!(
            finding.message,
            "lodash 4.17.15 has 6 known advisories: GHSA-0, GHSA-1, GHSA-2, GHSA-3 and 2 more."
        );

        let single = advisory_finding(&package, &["GHSA-only".to_string()]);
        assert_eq!(single.message, "lodash 4.17.15 has 1 known advisory: GHSA-only.");
    }

    /// The live reply shape, observed on 2026-08-11: a clean package's entry is `{}` with `vulns`
    /// absent. Reading it as a required field would fail on the common case.
    #[test]
    fn a_clean_entry_is_an_empty_object_not_an_empty_vulns_array() {
        let parsed: BatchResponse = serde_json::from_str(
            r#"{"results":[{"vulns":[{"id":"GHSA-29mw-wpgm-hmr9","modified":"2025-09-29T21:12:31.102523Z"}]},{}]}"#,
        )
        .expect("the observed reply shape must deserialize");
        assert_eq!(parsed.results.len(), 2);
        assert_eq!(parsed.results[0].vulns.len(), 1);
        assert_eq!(parsed.results[0].vulns[0].id, "GHSA-29mw-wpgm-hmr9");
        assert!(parsed.results[1].vulns.is_empty(), "`{{}}` must read as no advisories");
    }

    /// The outer half of the two bounds actually cuts a hung future off — `github/client.rs`'s
    /// test, for this module's own wrapper. `block_on` uses Tauri's runtime; SPEC.md §4 forbids
    /// creating one of our own, tests included. No real network call is involved.
    #[test]
    fn the_outer_timeout_fires_and_reads_as_no_answer() {
        let result: Option<()> = tauri::async_runtime::block_on(bounded_with(
            Duration::from_millis(5),
            async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Some(())
            },
        ));
        assert_eq!(result, None);
    }
}
