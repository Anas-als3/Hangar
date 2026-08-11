//! SPEC.md §18 / §11's Inbox entry (amended 2026-08-11, plan 062) — the Inbox's rows: one per
//! distinct repository, is its build red or green.
//!
//! # A check that could not run is never green
//!
//! This is the third time this rule has been written down in this repository (`vcs.rs`'s
//! `Unavailable`, the Doctor panel's "a check that could not run must never render as a clean bill
//! of health") and the second time it has had to be *fixed*. [`BuildState::Unknown`] is therefore a
//! value of its own, not an absent field and not a default: offline, rate-limited, a ref that could
//! not be resolved, a cancelled run and a conclusion GitHub invents next year all land on it, and
//! the panel renders it in muted grey with the word "unknown" — never in the green
//! [`BuildState::Passing`] wears.
//!
//! [`BuildState::NoChecks`] is a fourth thing again: GitHub answered, and this ref genuinely has no
//! checks. That is not a pass either, and it is not a failure to look.
//!
//! # Nothing in this module is an error
//!
//! Every function returns a state. SPEC.md §7 turns every `Err` into a toast and §18 makes offline
//! a first-class state, so the only `Err` in this whole feature is the command itself failing.

use serde::{Deserialize, Serialize};

/// The one fact the Inbox shows per repository. Kebab-case for the same reason `Status` and
/// `VcsState` are — the TypeScript mirror is a string union.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuildState {
    /// Every check on this ref finished, and none of them failed.
    Passing,
    /// At least one check finished and failed. The reason this feature exists: fifty identical
    /// "CI workflow run failed" notifications collapse to this one word.
    Failing,
    /// At least one check is queued or still running, and none has failed yet.
    Running,
    /// GitHub answered and there are no checks on this ref at all. **Not** a pass.
    NoChecks,
    /// Hangar could not find out. Offline, rate-limited, an unexpected status, a ref it could not
    /// determine, or a check that was cancelled or reported a conclusion this build does not know.
    /// **Never rendered as a pass** — see this module's header.
    Unknown,
}

/// One check run, reduced to the only two fields this feature reads. Deserialized straight from
/// the check-runs response; every other field GitHub sends (URLs, app metadata, output text,
/// annotations) is dropped by serde and never reaches Hangar's memory.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CheckRun {
    /// `queued` | `in_progress` | `completed` (and anything GitHub adds later, which this treats
    /// as "not completed").
    pub status: String,
    /// `null` until the run completes; then `success` | `failure` | `neutral` | `cancelled` |
    /// `timed_out` | `action_required` | `stale` | `skipped`.
    pub conclusion: Option<String>,
}

/// What one request actually saw. **`complete` is load-bearing, not bookkeeping**: the request asks
/// for a single page, and a green verdict drawn from a page that did not hold every check would be
/// the could-not-run-rendered-as-green bug wearing a different hat — the failing run could be the
/// one on page two. `false` therefore forces [`BuildState::Unknown`] unless a failure was already
/// found, since a failure seen is a failure regardless of what else was not seen.
///
/// Deliberately derived from "did the response fill the page" rather than from the API's
/// `total_count`: whether `total_count` counts before or after `filter=latest` is not something
/// this executor could verify against the live API, and a wrong reading of it would turn every
/// re-run repository permanently `unknown`. A short page is unambiguous.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecksPage {
    pub runs: Vec<CheckRun>,
    pub complete: bool,
}

/// One row of the Inbox. SPEC.md §7 addition (plan 062) — an addition to the frozen list, never a
/// rename or a reshape of anything in it.
///
/// There is deliberately **no field capable of holding a remote URL, a commit message, a run log
/// or an author**. The whole point of this row is that it is one fact wide; a field that exists can
/// be filled by a later refactor, and §18's line ("does it tell me something I would otherwise have
/// missed, or is it a worse version of a page GitHub already serves?") is what stops this becoming
/// an Actions dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoBuild {
    /// `owner/repo`, exactly as `.git/config` spelled it. Never a URL — see `repo.rs`'s header.
    pub repository: String,
    /// The ref that was checked. `None` only when Hangar could not determine one, which is an
    /// `Unknown` row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub state: BuildState,
    /// Which registered projects map to this row, in `projects.json` array order. SPEC.md §11:
    /// "the unit is the repository, never the project — two cards sharing one repo root produce
    /// one section."
    pub project_ids: Vec<String>,
    /// One secret-free human sentence, present only when `state` is `Unknown`. Built exclusively
    /// by the named converters in `commands.rs`; nothing here formats a `GithubError`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// ISO — set only when the primary rate limit is what stopped this row. §18: "rate limits are
    /// shown, never swallowed… and says when it resets."
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<String>,
    /// Set only when the secondary (abuse-detection) limit is what stopped this row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_sec: Option<u64>,
}

impl RepoBuild {
    /// A row with a state and nothing else — the base every constructor below fills in.
    pub fn new(repository: String, branch: Option<String>, project_ids: Vec<String>, state: BuildState) -> Self {
        RepoBuild {
            repository,
            branch,
            state,
            project_ids,
            detail: None,
            reset_at: None,
            retry_after_sec: None,
        }
    }

    /// The one constructor for "Hangar could not find out". Takes the sentence rather than
    /// building it, so `commands.rs` stays the only place a `GithubError` becomes text (§18).
    pub fn unknown(
        repository: String,
        branch: Option<String>,
        project_ids: Vec<String>,
        detail: String,
    ) -> Self {
        RepoBuild {
            detail: Some(detail),
            ..RepoBuild::new(repository, branch, project_ids, BuildState::Unknown)
        }
    }
}

/// Folds a ref's check runs into one state. Pure, so every branch below is tested without a
/// network call.
///
/// **Worst wins, and "could not tell" beats "green".** The ordering is deliberate:
///
/// 1. no runs at all → `NoChecks` (GitHub answered; there is nothing to pass)
/// 2. any completed failure → `Failing` — even from an incomplete page, because a failure seen is
///    a failure however much was not seen
/// 3. any run still queued or in progress → `Running`
/// 4. any completed run whose conclusion is not a pass and not a failure — `cancelled`, `stale`,
///    a missing conclusion, or a value GitHub adds after this was written → `Unknown`
/// 5. a page that did not hold every check → `Unknown`
/// 6. otherwise → `Passing`
///
/// Steps 4 and 5 are the guard. `success`, `neutral` and `skipped` are the **only** three
/// conclusions that may contribute to a green row, and they are listed explicitly rather than
/// reached by an `else`: an `else` is exactly how a cancelled run — a check that did not finish —
/// would come to render as a passing build.
pub fn summarize(page: &ChecksPage) -> BuildState {
    if page.runs.is_empty() {
        // An empty page cannot have been truncated, but a `complete: false` here would still mean
        // "Hangar saw nothing and cannot promise that is all there is" — never "nothing is wrong".
        return if page.complete { BuildState::NoChecks } else { BuildState::Unknown };
    }
    let mut any_incomplete = false;
    let mut any_inconclusive = false;
    for run in &page.runs {
        if !run.status.eq_ignore_ascii_case("completed") {
            any_incomplete = true;
            continue;
        }
        match run.conclusion.as_deref() {
            Some("failure") | Some("timed_out") | Some("action_required") | Some("startup_failure") => {
                return BuildState::Failing
            }
            Some("success") | Some("neutral") | Some("skipped") => {}
            _ => any_inconclusive = true,
        }
    }
    if any_incomplete {
        BuildState::Running
    } else if any_inconclusive || !page.complete {
        BuildState::Unknown
    } else {
        BuildState::Passing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(status: &str, conclusion: Option<&str>) -> CheckRun {
        CheckRun { status: status.to_string(), conclusion: conclusion.map(str::to_string) }
    }

    /// A response that held every check on the ref — the ordinary case.
    fn whole(runs: &[CheckRun]) -> ChecksPage {
        ChecksPage { runs: runs.to_vec(), complete: true }
    }

    #[test]
    fn every_check_succeeding_is_the_only_route_to_passing() {
        assert_eq!(
            summarize(&whole(&[run("completed", Some("success")), run("completed", Some("success"))])),
            BuildState::Passing
        );
        // `neutral` and `skipped` are non-failures by GitHub's own definition.
        assert_eq!(
            summarize(&whole(&[
                run("completed", Some("success")),
                run("completed", Some("skipped")),
                run("completed", Some("neutral")),
            ])),
            BuildState::Passing
        );
    }

    /// The measured case this whole plan was rewritten around: fifty "CI workflow run failed"
    /// notifications are one repository whose build is red.
    #[test]
    fn one_failure_makes_the_row_failing_however_many_others_passed() {
        assert_eq!(
            summarize(&whole(&[
                run("completed", Some("success")),
                run("completed", Some("failure")),
                run("completed", Some("success")),
            ])),
            BuildState::Failing
        );
        for conclusion in ["timed_out", "action_required", "startup_failure"] {
            assert_eq!(summarize(&whole(&[run("completed", Some(conclusion))])), BuildState::Failing);
        }
    }

    #[test]
    fn a_failure_outranks_a_run_still_in_flight() {
        assert_eq!(
            summarize(&whole(&[run("in_progress", None), run("completed", Some("failure"))])),
            BuildState::Failing
        );
        assert_eq!(
            summarize(&whole(&[run("queued", None), run("completed", Some("success"))])),
            BuildState::Running
        );
    }

    /// GitHub answered and there is nothing on this ref. That is its own state — a repository with
    /// no CI has not passed anything.
    #[test]
    fn no_checks_at_all_is_its_own_state_and_is_not_passing() {
        assert_eq!(summarize(&whole(&[])), BuildState::NoChecks);
        assert_ne!(summarize(&whole(&[])), BuildState::Passing);
    }

    /// **THE GUARD** (plan 062's done criterion; SPEC.md §11's "a check that could not run must
    /// never render as a clean bill of health"). A completed run whose conclusion is not one of the
    /// three passing values — cancelled, stale, absent, or something GitHub adds later — is
    /// `Unknown`, and `Unknown` is not `Passing`.
    ///
    /// Mutation-tested: replacing the explicit `Some("success") | Some("neutral") | Some("skipped")`
    /// arm's fallthrough with a catch-all that treats anything non-failing as a pass makes every
    /// assertion below fail with `left: Passing, right: Unknown`.
    #[test]
    fn a_check_that_could_not_run_is_unknown_and_never_passing() {
        for conclusion in [Some("cancelled"), Some("stale"), None, Some("some_future_conclusion")] {
            let state = summarize(&whole(&[run("completed", conclusion)]));
            assert_eq!(state, BuildState::Unknown, "conclusion {conclusion:?} must be unknown");
            assert_ne!(state, BuildState::Passing, "conclusion {conclusion:?} must never be green");
        }
        // …and one inconclusive run poisons an otherwise green ref, rather than being outvoted.
        let mixed = summarize(&whole(&[
            run("completed", Some("success")),
            run("completed", Some("cancelled")),
        ]));
        assert_eq!(mixed, BuildState::Unknown);
        assert_ne!(mixed, BuildState::Passing);
    }

    /// **THE SAME GUARD, ON THE PAGE.** A response that filled its page may have left the failing
    /// run behind, so an all-green partial page is `unknown` — never `passing`. A failure that WAS
    /// seen still wins, because a failure seen is a failure however much was not seen.
    ///
    /// Mutation-tested: deleting `|| !page.complete` from `summarize`'s penultimate branch makes
    /// the first assertion below fail with `left: Passing, right: Unknown`.
    #[test]
    fn a_page_that_did_not_hold_every_check_is_unknown_never_passing() {
        let partial = |runs: &[CheckRun]| ChecksPage { runs: runs.to_vec(), complete: false };
        let all_green = summarize(&partial(&[run("completed", Some("success"))]));
        assert_eq!(all_green, BuildState::Unknown);
        assert_ne!(all_green, BuildState::Passing);
        assert_eq!(summarize(&partial(&[run("completed", Some("failure"))])), BuildState::Failing);
        assert_eq!(summarize(&partial(&[])), BuildState::Unknown);
    }

    /// The same guard on the row rather than the fold: a row Hangar could not check carries
    /// `Unknown` and a sentence, and can never be confused with a green one by a frontend that
    /// only switches on `state`.
    #[test]
    fn an_unchecked_row_is_unknown_and_carries_no_passing_signal() {
        let row = RepoBuild::unknown(
            "anas/hangar".into(),
            Some("main".into()),
            vec!["abc123".into()],
            "Hangar could not reach GitHub.".into(),
        );
        assert_eq!(row.state, BuildState::Unknown);
        assert_ne!(row.state, BuildState::Passing);
        assert!(row.detail.is_some());
        // The wire value the frontend switches on is a distinct string, not an absent field.
        let wire = serde_json::to_value(&row).unwrap();
        assert_eq!(wire["state"], "unknown");
        assert_ne!(wire["state"], "passing");
    }

    /// Serde drops everything else GitHub sends. Asserted because "one fact wide" is a property of
    /// this feature, not a coincidence of the current response shape.
    #[test]
    fn only_status_and_conclusion_are_read_from_a_check_run() {
        let body = serde_json::json!({
            "status": "completed",
            "conclusion": "failure",
            "html_url": "https://github.com/anas/hangar/runs/1",
            "output": { "title": "boom", "summary": "a stack trace" },
            "app": { "name": "GitHub Actions" }
        });
        let parsed: CheckRun = serde_json::from_value(body).unwrap();
        assert_eq!(parsed, run("completed", Some("failure")));
    }
}
