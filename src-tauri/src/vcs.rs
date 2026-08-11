//! SPEC.md §11 "Launch line" (added 2026-08-11, plan 060) — the local-only version-control read
//! behind the one line under the header.
//!
//! # THE TWO INVARIANTS
//!
//! **1. It reports; it never acts.** SPEC.md §3's OUT list is absolute and this module is the
//! place it would be crossed first. There is no push, no pull, no fetch, no commit, no stash here
//! — not behind a confirm, not in a menu, not "for convenience". The single command this module
//! ever runs is a `git status` read, and it carries `--no-optional-locks` so it will not even take
//! git's index lock. Adding a write here turns Hangar into a git client, which §3 says it is not.
//!
//! **2. No network, and "behind" is therefore never reported.** `git ls-remote` and `git fetch`
//! are network calls; one per project on launch puts a hung DNS lookup on the startup path. This
//! module reads the **local remote-tracking ref** only. That trade has a consequence, and the
//! consequence is stated rather than hidden:
//!
//! - **Ahead (unpushed) is exact** — `HEAD` and the tracking ref are both local facts.
//! - **Behind is NOT reported at all.** `git status --porcelain=v2 --branch` prints it in the same
//!   `# branch.ab +A -B` line, and [`parse_status_v2`] deliberately reads only `+A`. There is no
//!   field on [`VcsStatus`] capable of holding a "behind" count — a field that exists can be
//!   filled by a later refactor, a field that does not exist cannot. Hangar never fetches, so a
//!   stale "you are up to date" would be worse than silence.
//!
//! # A check that could not run is never a clean bill of health
//!
//! [`VcsState`] has three values and they are three different facts:
//! `not-a-repo` (looked, nothing to say) · `checked` (git answered) · `unavailable` (git did **not**
//! answer — missing, timed out, or exited non-zero). A clean repo is `checked` with `ahead: 0,
//! uncommitted: 0`; a failed check is `unavailable` with both `None`. They can never collapse into
//! each other, because the state is a separate field the frontend must switch on. That separation
//! is the whole point: **a check that could not run must never render as a clean bill of health.**
//! In a line that is silent when clean, "I did not look" and "there is nothing to say" would
//! otherwise draw the same zero pixels, and the user would read reassurance into a failure.
//!
//! # Why a sibling module and not `preflight.rs`
//!
//! `preflight.rs` is the Doctor panel's module, and §11 binds it to a rule this module breaks by
//! design: preflight "never runs on the startup path". The launch line *is* the startup path — it
//! is the thing you see when you sit down. Folding the two together would put one module under two
//! contradictory lifecycle rules, and every future check added to the Doctor would arrive on the
//! launch path by accident rather than by decision. They stay apart.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::env_resolve::EnvMap;
use crate::process::{self, ShellKind, SpawnSpec};

/// What Hangar was able to learn about one project's repository.
///
/// Kebab-case for the same reason `Status` is — the TypeScript mirror is a string union.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VcsState {
    /// No `.git` anywhere at or above the project folder (or the folder is gone). Hangar looked and
    /// there is genuinely nothing to say — the line stays silent. Not an error.
    NotARepo,
    /// `git status` ran and answered. `ahead`/`uncommitted` carry the answer; both `0` is the
    /// common, quiet case.
    Checked,
    /// `git status` did **not** answer: git was not on the resolved PATH, the read timed out, or it
    /// exited non-zero. **This is not "clean".** The line says so rather than staying silent.
    Unavailable,
}

/// One project's row of the launch-line snapshot. SPEC.md §7 addition (plan 060) — an addition to
/// the frozen list, never a rename or a reshape of anything in it.
///
/// **There is deliberately no `behind` field.** See THE TWO INVARIANTS above. Do not add one "for
/// completeness": Hangar does not fetch, so any number it could hold would be as old as the user's
/// last manual fetch and would read as current.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VcsStatus {
    pub project_id: String,
    pub state: VcsState,
    /// Commits on `HEAD` that the **local** remote-tracking ref does not have — "unpushed".
    /// `None` when `state != Checked`, and also when the branch has no upstream configured, is
    /// detached, or the repo has no commits yet: in all three there is nothing to count, which is
    /// not the same as counting zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ahead: Option<u32>,
    /// How many paths `git status --porcelain=v2` listed — changed, staged, unmerged or untracked.
    /// A **count**, never a name and never a diff: nothing about the content of an uncommitted
    /// change leaves this module. `None` when `state != Checked`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uncommitted: Option<u32>,
    /// Why the check could not run, in one human sentence. Only ever `Some` for `Unavailable`;
    /// it explains a failure, it never carries repository content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// ISO — one timestamp shared by every row in a single `get_vcs_status` call.
    pub checked_at: String,
}

/// One `git status` read is a one-shot lookup on the startup path, so it gets a budget closer to
/// §9 step 1's owner lookup than to §9 step 2's 10 s pull: a repository large enough to need longer
/// than this is one whose launch line is not worth stalling for. On expiry the row is
/// `Unavailable`, never a silent "clean".
const STATUS_TIMEOUT: Duration = Duration::from_secs(3);

/// SPEC.md §9 step 2's four non-interactive variables, applied to this module's read for the same
/// reason `run.rs` applies them to the pull: a git that prompts for credentials on the startup path
/// would hang this line forever, and `stdin` being null (§8) turns a prompt into a hang, not a
/// failure, for helpers that reopen the tty.
///
/// Deliberately a second copy of `run.rs`'s `git_pull_env` rather than a shared import: plan 060
/// puts `run.rs` out of scope, and widening a private helper's visibility there to save four lines
/// here is not a trade worth making. If a fifth variable is ever needed, both lists change.
fn non_interactive_git_env() -> Vec<(String, String)> {
    vec![
        ("GIT_TERMINAL_PROMPT".to_string(), "0".to_string()),
        ("GIT_ASKPASS".to_string(), "echo".to_string()),
        ("GIT_SSH_COMMAND".to_string(), "ssh -oBatchMode=yes".to_string()),
        ("GCM_INTERACTIVE".to_string(), "never".to_string()),
    ]
}

/// The one command this module runs, and the only one it ever may.
///
/// - `--no-optional-locks`: git must not take the index lock to refresh stat information. A status
///   monitor that fights a real `git` invocation for the lock is a status monitor that acts.
/// - `-c core.quotePath=true`: paths with newlines come back C-quoted on one line, so the
///   line-based entry count in [`parse_status_v2`] cannot be inflated by a filename.
/// - `--porcelain=v2 --branch`: the machine format, which is documented as stable and untranslated,
///   and whose `# branch.ab` header is what makes "ahead" readable without a second spawn.
///
/// Note for the reviewer: this string contains no push/pull/fetch/commit/stash, and there is no
/// second command string in this file.
const STATUS_COMMAND: &str =
    "git --no-optional-locks -c core.quotePath=true status --porcelain=v2 --branch";

// ---------------------------------------------------------------------------------------------
// The pure half — everything below `read_status` is testable without spawning git.
// ---------------------------------------------------------------------------------------------

/// What one successful `git status --porcelain=v2 --branch` said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedStatus {
    /// From `# branch.ab +A -B`, the `+A` half **only**. `None` when the header is absent, which is
    /// exactly the set of cases git omits it for: no upstream configured, a detached `HEAD`, and a
    /// repository with no commits yet.
    pub ahead: Option<u32>,
    /// Entry lines: `1` (ordinary change), `2` (rename/copy), `u` (unmerged), `?` (untracked).
    pub uncommitted: u32,
}

/// Parses porcelain v2. Pure, so every case the plan names — no upstream, detached `HEAD`, a repo
/// with zero commits — is tested against captured output rather than a fixture repository.
///
/// **The `-B` (behind) half of `# branch.ab` is never bound to a variable here.** That is the same
/// device `preflight.rs` uses to keep `.env` values out of its report: the value is not read, so
/// there is nothing for a later refactor to start returning.
pub fn parse_status_v2(stdout: &str) -> ParsedStatus {
    let mut ahead = None;
    let mut uncommitted = 0u32;

    for line in stdout.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(rest) = line.strip_prefix("# branch.ab ") {
            // `+A -B`. Only the `+A` token is looked at; the `-B` token is skipped without being
            // read — see this function's doc comment.
            ahead = rest
                .split_whitespace()
                .next()
                .and_then(|token| token.strip_prefix('+'))
                .and_then(|count| count.parse::<u32>().ok());
            continue;
        }
        if line.starts_with('#') {
            continue; // any other header (`branch.oid`, `branch.head`, `branch.upstream`, …)
        }
        // Entry lines. The prefix is a single character followed by a space; anything else (a blank
        // trailing line, an `!` ignored entry we never ask for) is not counted.
        if matches!(line.as_bytes().first(), Some(b'1' | b'2' | b'u' | b'?'))
            && line.as_bytes().get(1) == Some(&b' ')
        {
            uncommitted += 1;
        }
    }

    ParsedStatus { ahead, uncommitted }
}

/// Whether a `.git` entry exists at `dir` or any ancestor — the cheap, spawn-free repo test.
///
/// A **file** counts as well as a directory: that is what a linked worktree and a submodule look
/// like. Walking upward is what makes a monorepo package path (a project registered at
/// `repo/apps/web`) read as the repository it is in, instead of being written off as "not a repo"
/// while it quietly carries thirty unpushed commits.
///
/// This runs first, so a non-repo costs **zero** spawns; and because it runs first, a non-zero exit
/// from `git status` in a directory that does have a `.git` can be reported as a failure to check
/// rather than being guessed at as "not a repo".
pub fn looks_like_a_repo(dir: &Path) -> bool {
    let mut current = Some(dir);
    while let Some(path) = current {
        if path.join(".git").exists() {
            return true;
        }
        current = path.parent();
    }
    false
}

/// Turns one completed `git status` run into a state. Pure, and the guard against this plan's
/// predecessor bug: a non-zero exit or unparseable output becomes `Unavailable`, which is a
/// **different value** from a clean `Checked`, not an empty one that renders the same way.
pub fn interpret_status(exit_code: Option<i32>, stdout: &str) -> (VcsState, Option<ParsedStatus>, Option<String>) {
    match exit_code {
        Some(0) => (VcsState::Checked, Some(parse_status_v2(stdout)), None),
        Some(code) if process::is_tool_not_found_exit(code) => (
            VcsState::Unavailable,
            None,
            Some("git is not on the PATH Hangar resolved for this machine.".to_string()),
        ),
        Some(code) => (
            VcsState::Unavailable,
            None,
            Some(format!("git status exited {code}, so this project was not checked.")),
        ),
        None => (
            VcsState::Unavailable,
            None,
            Some("git status was terminated, so this project was not checked.".to_string()),
        ),
    }
}

/// Assembles a row from a state. Kept separate from [`read_status`] so the wire shape's own rule —
/// `ahead`/`uncommitted` are `Some` only when `state == Checked` — is enforced in one place and
/// testable without git.
pub fn build_status(
    project_id: &str,
    state: VcsState,
    parsed: Option<ParsedStatus>,
    detail: Option<String>,
    checked_at: &str,
) -> VcsStatus {
    let (ahead, uncommitted) = match (state, parsed) {
        (VcsState::Checked, Some(p)) => (p.ahead, Some(p.uncommitted)),
        // `Unavailable` and `NotARepo` carry no counts at all. A `Some(0)` here would be the exact
        // "a failed check renders as clean" bug this module exists to make impossible.
        _ => (None, None),
    };
    VcsStatus {
        project_id: project_id.to_string(),
        state,
        ahead,
        uncommitted,
        detail: if state == VcsState::Unavailable { detail } else { None },
        checked_at: checked_at.to_string(),
    }
}

// ---------------------------------------------------------------------------------------------
// The spawning half
// ---------------------------------------------------------------------------------------------

/// One project's row. Never returns `Err` and never panics — every failure is a `state`, because
/// §7 turns an `Err` into a toast and a toast per project on launch would be intolerable.
///
/// Uses the **one** §8 spawn helper (`process::spawn`); no `Command` is constructed here.
pub async fn read_status(
    project_id: &str,
    dir: &Path,
    env: &EnvMap,
    checked_at: &str,
) -> VcsStatus {
    if !looks_like_a_repo(dir) {
        return build_status(project_id, VcsState::NotARepo, None, None, checked_at);
    }

    let spec = SpawnSpec {
        command: STATUS_COMMAND.to_string(),
        cwd: Some(dir.to_path_buf()),
        env: env.clone(),
        extra_env: non_interactive_git_env(),
        // Read-only one-shot: no process group, no Job Object (§8), same as `check_git_repo` and
        // the preflight `node --version` read. `kill_on_drop` so a wedged git cannot outlive the
        // timeout below — tokio's reaper for this helper alone, never the §8 kill path.
        long_lived: false,
        kill_on_drop: true,
        shell: ShellKind::Default,
    };

    let spawned = match process::spawn(&spec) {
        Ok(spawned) => spawned,
        Err(e) => {
            return build_status(
                project_id,
                VcsState::Unavailable,
                None,
                Some(format!("git could not be started ({e}), so this project was not checked.")),
                checked_at,
            )
        }
    };

    let output = match tokio::time::timeout(STATUS_TIMEOUT, spawned.child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return build_status(
                project_id,
                VcsState::Unavailable,
                None,
                Some(format!("git status could not be read ({e}), so this project was not checked.")),
                checked_at,
            )
        }
        Err(_elapsed) => {
            return build_status(
                project_id,
                VcsState::Unavailable,
                None,
                Some(format!(
                    "git status did not answer within {} s, so this project was not checked.",
                    STATUS_TIMEOUT.as_secs()
                )),
                checked_at,
            )
        }
    };

    // Lossy decode, never a hard failure on odd bytes (§8's log-pipeline rule).
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (state, parsed, detail) = interpret_status(output.status.code(), &stdout);
    build_status(project_id, state, parsed, detail, checked_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique scratch directory under the OS temp dir — same idiom as `preflight.rs`'s `scratch`,
    /// which exists to avoid adding a `tempfile` dependency. **No git command is ever run against
    /// these**: `looks_like_a_repo` is a filesystem test, so a `.git` directory is enough.
    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hangar-vcs-test-{tag}-{}-{:?}",
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

    const AHEAD_30: &str = "\
# branch.oid 1f0c2b3a4d5e6f708192a3b4c5d6e7f809a1b2c3
# branch.head main
# branch.upstream origin/main
# branch.ab +30 -0
";

    #[test]
    fn thirty_unpushed_commits_are_counted_exactly() {
        let parsed = parse_status_v2(AHEAD_30);
        assert_eq!(parsed.ahead, Some(30));
        assert_eq!(parsed.uncommitted, 0);
    }

    #[test]
    fn entry_lines_of_every_kind_are_counted_and_headers_are_not() {
        let stdout = "\
# branch.oid 1f0c2b3a4d5e6f708192a3b4c5d6e7f809a1b2c3
# branch.head main
# branch.upstream origin/main
# branch.ab +0 -0
1 .M N... 100644 100644 100644 aaaa bbbb src/App.tsx
1 M. N... 100644 100644 100644 cccc dddd SPEC.md
2 R. N... 100644 100644 100644 eeee ffff R100 new/path\told/path
u UU N... 100644 100644 100644 100644 1111 2222 3333 conflicted.txt
? untracked.txt
";
        let parsed = parse_status_v2(stdout);
        assert_eq!(parsed.uncommitted, 5, "one per entry line, none per header");
        assert_eq!(parsed.ahead, Some(0));
    }

    /// The plan's four "yields nothing, not an error" cases, all as captured output.
    #[test]
    fn no_upstream_detached_head_and_an_empty_repo_all_report_no_ahead_count() {
        // No upstream configured: git omits `# branch.ab` entirely.
        let no_upstream = "# branch.oid 1f0c2b3a\n# branch.head main\n";
        assert_eq!(parse_status_v2(no_upstream).ahead, None);
        assert_eq!(parse_status_v2(no_upstream).uncommitted, 0);

        // Detached HEAD.
        let detached = "# branch.oid 1f0c2b3a\n# branch.head (detached)\n";
        assert_eq!(parse_status_v2(detached).ahead, None);
        assert_eq!(parse_status_v2(detached).uncommitted, 0);

        // A repository with no commits yet.
        let initial = "# branch.oid (initial)\n# branch.head main\n";
        assert_eq!(parse_status_v2(initial).ahead, None);
        assert_eq!(parse_status_v2(initial).uncommitted, 0);

        // Empty output at all (not a shape git produces, but it must not panic or invent a count).
        assert_eq!(parse_status_v2("").ahead, None);
        assert_eq!(parse_status_v2("").uncommitted, 0);
    }

    /// THE SECOND INVARIANT's guard: the behind count is in the same header line and must not come
    /// out anywhere. Two-sided — the positive half stops it passing vacuously against a parser that
    /// returns nothing at all.
    #[test]
    fn the_behind_count_never_leaves_this_module() {
        let stdout = "\
# branch.oid 1f0c2b3a
# branch.head main
# branch.upstream origin/main
# branch.ab +7 -917
";
        let parsed = parse_status_v2(stdout);
        assert_eq!(parsed.ahead, Some(7), "the ahead count must be read, or this guard is vacuous");

        let status = build_status("p1", VcsState::Checked, Some(parsed), None, "2026-08-11T09:00:00Z");
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"ahead\":7"), "got {json}");
        assert!(
            !json.contains("917") && !json.contains("behind"),
            "a 'behind' count reached the serialized row — Hangar does not fetch and must never \
             imply it knows: {json}"
        );
    }

    /// The predecessor bug this plan was warned about, pinned: a check that did not run must not
    /// serialize into the same shape as a check that ran and found nothing.
    #[test]
    fn a_failed_check_is_a_different_row_from_a_clean_one() {
        let clean = build_status(
            "p1",
            VcsState::Checked,
            Some(ParsedStatus { ahead: Some(0), uncommitted: 0 }),
            None,
            "2026-08-11T09:00:00Z",
        );
        let (state, parsed, detail) = interpret_status(Some(128), "");
        let failed = build_status("p1", state, parsed, detail, "2026-08-11T09:00:00Z");

        assert_ne!(clean, failed);
        assert_eq!(clean.state, VcsState::Checked);
        assert_eq!(clean.ahead, Some(0));
        assert_eq!(clean.uncommitted, Some(0));

        assert_eq!(failed.state, VcsState::Unavailable);
        assert_eq!(failed.ahead, None, "a failed check must never report a count");
        assert_eq!(failed.uncommitted, None, "a failed check must never report a count");
        assert!(failed.detail.is_some(), "a failed check must say why");

        // And the two are distinguishable on the wire, which is the only place the frontend can
        // tell them apart.
        let clean_json = serde_json::to_string(&clean).unwrap();
        let failed_json = serde_json::to_string(&failed).unwrap();
        assert!(clean_json.contains("\"state\":\"checked\""), "got {clean_json}");
        assert!(failed_json.contains("\"state\":\"unavailable\""), "got {failed_json}");
    }

    #[test]
    fn git_missing_and_a_terminated_read_are_both_unavailable_never_clean() {
        for code in [Some(127), Some(126), Some(9009), None] {
            let (state, parsed, detail) = interpret_status(code, "");
            assert_eq!(state, VcsState::Unavailable, "exit {code:?}");
            assert!(parsed.is_none(), "exit {code:?}");
            assert!(detail.is_some(), "exit {code:?} must carry a reason");
        }
    }

    #[test]
    fn a_successful_exit_is_parsed_even_when_the_repo_is_clean() {
        let (state, parsed, detail) = interpret_status(Some(0), AHEAD_30);
        assert_eq!(state, VcsState::Checked);
        assert_eq!(parsed.unwrap().ahead, Some(30));
        assert!(detail.is_none());
    }

    #[test]
    fn a_folder_with_no_git_anywhere_above_it_is_not_a_repo_and_costs_no_spawn() {
        let dir = scratch("plain");
        assert!(!looks_like_a_repo(&dir));

        let status = build_status("p1", VcsState::NotARepo, None, None, "2026-08-11T09:00:00Z");
        assert_eq!(status.ahead, None);
        assert_eq!(status.uncommitted, None);
        assert_eq!(status.detail, None, "not-a-repo is a fact, not a failure — it says nothing");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_git_dir_or_git_file_at_or_above_the_project_path_reads_as_a_repo() {
        // A plain repository root.
        let root = scratch("repo-root");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        assert!(looks_like_a_repo(&root));

        // A monorepo package, several levels down — the case that would otherwise be written off
        // as "not a repo" while carrying unpushed commits.
        let package = root.join("apps").join("web");
        std::fs::create_dir_all(&package).unwrap();
        assert!(looks_like_a_repo(&package));

        // A linked worktree / submodule, where `.git` is a FILE.
        let worktree = scratch("worktree");
        std::fs::write(worktree.join(".git"), "gitdir: /elsewhere/.git/worktrees/x\n").unwrap();
        assert!(looks_like_a_repo(&worktree));

        // A folder that is gone entirely — §12 already warns on the card; here it is simply not a
        // repo, and silent.
        let missing = std::env::temp_dir().join("hangar-vcs-test-definitely-not-here");
        let _ = std::fs::remove_dir_all(&missing);
        assert!(!looks_like_a_repo(&missing));

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&worktree);
    }

    /// SPEC.md §3, pinned: this module reports, it never acts.
    ///
    /// A **whitelist**, deliberately, not a list of forbidden verbs. Two reasons. It is stronger —
    /// a blacklist passes for the next verb nobody thought of. And a blacklist would have to spell
    /// those verbs as string literals in this file, where they would turn up in the maintainer's
    /// `git grep -nE "\"(...)\"" src-tauri/src` audit and make a signal that should mean exactly
    /// one thing ("a git write reached the codebase") mean two.
    #[test]
    fn the_only_git_subcommand_this_module_runs_is_status() {
        let mut tokens = STATUS_COMMAND.split_whitespace();
        assert_eq!(tokens.next(), Some("git"), "got {STATUS_COMMAND:?}");

        // The first token that is neither a flag nor a `-c` flag's value IS the subcommand.
        let mut subcommand = None;
        let mut skip_value = false;
        for token in tokens {
            if skip_value {
                skip_value = false;
                continue;
            }
            if token == "-c" {
                skip_value = true;
                continue;
            }
            if token.starts_with('-') {
                continue;
            }
            subcommand = Some(token);
            break;
        }
        assert_eq!(
            subcommand,
            Some("status"),
            "SPEC.md §3: the only git subcommand this module may run is a read — got \
             {STATUS_COMMAND:?}"
        );
        assert!(
            STATUS_COMMAND.contains("--no-optional-locks"),
            "the read must not take git's index lock — got {STATUS_COMMAND:?}"
        );
    }
}
