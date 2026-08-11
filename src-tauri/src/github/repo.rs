//! SPEC.md §18 / plan 062 — which GitHub repository a project folder belongs to, read from
//! `.git/config`.
//!
//! # THIS MODULE NEVER SHELLS OUT TO GIT
//!
//! Detection is a file read, deliberately, and the reason is a bug this repo has already paid for
//! once: `run_lookup` sets no `cwd`, so listing a project's remotes via a git subprocess resolves
//! against **Hangar's own** working directory and reports every registered project as the same
//! repository. There is no subprocess of any kind in this file — no §8 spawn helper, no standard
//! library process API, no git invocation — and the test at the bottom asserts it against this
//! file's own source text. Adding one reintroduces that bug in a form no test on this machine
//! would notice, because Hangar's own checkout happens to be a GitHub repository too.
//!
//! # Nothing here is ever an error
//!
//! Every failure — no `.git`, no `origin`, a non-GitHub host, an unreadable file — yields
//! [`None`]. SPEC.md §11's Inbox entry: "a project that is not on GitHub, has no remote, or cannot
//! be seen with the current token is simply **absent**, with no toast, no banner and no `system`
//! log line."
//!
//! # The remote URL never leaves this module
//!
//! A `.git/config` remote URL can carry a credential (`https://ghp_…@github.com/o/r.git`), so the
//! URL is a **secret-shaped** value: [`github_slug`] returns only the two path segments, and no
//! function here puts a URL into a returned value, a detail string or a `Debug` impl. If you ever
//! need to report *why* a remote was rejected, report the reason without the URL.

use std::path::{Path, PathBuf};

/// One repository coordinate to ask GitHub about. Owner and repo come from `.git/config`; the ref
/// comes from `.git/HEAD`, and is `None` when HEAD could not be read or names something this
/// module refuses to put in a URL — which makes the row **unknown**, never green.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoTarget {
    pub owner: String,
    pub repo: String,
    /// The branch currently checked out locally, or the detached-HEAD SHA. Deliberately the
    /// **local** ref rather than the literal `HEAD` or the repository's default branch: a branch
    /// name is documented as a valid `{ref}` for the check-runs endpoint, it costs no second API
    /// call to learn, and it answers the question the user actually has — is the build red for
    /// what I am working on.
    pub git_ref: Option<String>,
}

impl RepoTarget {
    /// `owner/repo`, the only rendering of this coordinate that is ever serialized.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    /// SPEC.md §11: "the unit is the repository, never the project — two cards sharing one repo
    /// root produce one row." Owner and repo compare case-insensitively (GitHub's own rule); the
    /// ref compares exactly, so two *separate clones* of one repository sitting on two different
    /// branches stay two rows. Merging those would print one branch's build under the other's
    /// name, which is the same class of lie as a check that could not run rendering as green.
    pub fn same_row(&self, other: &RepoTarget) -> bool {
        self.owner.eq_ignore_ascii_case(&other.owner)
            && self.repo.eq_ignore_ascii_case(&other.repo)
            && self.git_ref == other.git_ref
    }
}

/// The nearest `.git` **directory** at or above `dir`.
///
/// Walking upward is what makes a monorepo package path (a project registered at `repo/apps/web`)
/// resolve to the repository it lives in — the same walk `vcs::looks_like_a_repo` does.
///
/// A `.git` **file** stops the walk and yields `None`. That is what a linked worktree and a
/// submodule look like, and plan 062 makes both *absent*: continuing upward from a worktree would
/// find some unrelated parent repository, and following the `gitdir:` pointer is a second file
/// format for a case Hangar has no row to show anyway.
pub fn find_git_dir(dir: &Path) -> Option<PathBuf> {
    let mut current = Some(dir);
    while let Some(path) = current {
        let candidate = path.join(".git");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if candidate.exists() {
            return None; // a `.git` file — worktree or submodule
        }
        current = path.parent();
    }
    None
}

/// True for `[remote "origin"]`. The section name is case-insensitive in git's config format; the
/// subsection (`origin`) is case-sensitive.
fn section_is_origin_remote(header: &str) -> bool {
    let inner = header.trim_start_matches('[').split(']').next().unwrap_or("").trim();
    let Some((name, subsection)) = inner.split_once(char::is_whitespace) else {
        return false;
    };
    name.eq_ignore_ascii_case("remote") && subsection.trim().trim_matches('"') == "origin"
}

/// The `url` of `[remote "origin"]`, or `None` when there is no origin (a repo that was `git
/// init`-ed and never pushed) or no `url` under it.
///
/// Returns a borrowed slice on purpose: the caller feeds it straight to [`github_slug`] and drops
/// it. See this module's header — the URL may carry a credential and is never stored.
pub fn parse_origin_url(config: &str) -> Option<&str> {
    let mut in_origin = false;
    for raw in config.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            in_origin = section_is_origin_remote(line);
            continue;
        }
        if !in_origin {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("url") {
            return Some(value.trim());
        }
    }
    None
}

/// Splits any remote URL into `(host-authority, path)`. Handles both shapes git writes:
/// `scheme://[userinfo@]host[:port]/path` and the scp-like `[user@]host:path`.
fn split_authority_and_path(url: &str) -> Option<(&str, &str)> {
    if let Some((_scheme, rest)) = url.split_once("://") {
        rest.split_once('/')
    } else {
        url.split_once(':')
    }
}

/// A path segment GitHub itself would accept for an owner or a repository name. This is also the
/// **URL-injection guard**: everything that reaches [`super::client::check_runs`]'s path is
/// checked here first, so a `.git/config` cannot steer a request at another endpoint.
fn is_valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// `(owner, repo)` when — and only when — the remote points at `github.com` itself.
///
/// GitHub Enterprise hosts, GitLab, a self-hosted forge and a lookalike domain
/// (`github.com.example.net`) all yield `None`: SPEC.md §18 permits exactly one provider and one
/// host, and a host that merely *contains* "github.com" is not that host.
pub fn github_slug(url: &str) -> Option<(String, String)> {
    let (authority, path) = split_authority_and_path(url)?;
    // Discard any `user[:password]@` prefix WITHOUT reading it — see this module's header.
    let host_and_port = authority.rsplit_once('@').map(|(_, host)| host).unwrap_or(authority);
    let host = host_and_port.split(':').next().unwrap_or("");
    if !host.eq_ignore_ascii_case("github.com") {
        return None;
    }

    let path = path.trim_start_matches('/').trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut segments = path.split('/');
    let owner = segments.next()?;
    let repo = segments.next()?;
    if segments.next().is_some() || !is_valid_segment(owner) || !is_valid_segment(repo) {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

/// The ref named by `.git/HEAD`: the branch when attached, the SHA when detached.
///
/// `None` for anything else — an empty file, a `ref:` pointing outside `refs/heads/`, junk. That
/// becomes an **unknown** row rather than a skipped one.
pub fn parse_head(head: &str) -> Option<String> {
    let line = head.lines().next()?.trim();
    if let Some(target) = line.strip_prefix("ref:") {
        let branch = target.trim().strip_prefix("refs/heads/")?;
        return (!branch.is_empty()).then(|| branch.to_string());
    }
    // Detached HEAD — the file holds a raw object id.
    let looks_like_an_object_id =
        line.len() >= 7 && line.chars().all(|c| c.is_ascii_hexdigit());
    looks_like_an_object_id.then(|| line.to_string())
}

/// Whether a ref may be pasted into a request path. The second half of the URL-injection guard
/// (see [`is_valid_segment`]): `%`, `?`, `#`, whitespace, control characters and `..` are all
/// refused, while `/` is allowed because `feature/thing` is an ordinary branch name and the
/// check-runs endpoint matches a slashed ref greedily.
///
/// A refused ref is not an error and not a skip — it is an unknown row.
pub fn is_url_safe_ref(git_ref: &str) -> bool {
    !git_ref.is_empty()
        && !git_ref.starts_with('/')
        && !git_ref.starts_with('-')
        && !git_ref.contains("..")
        && git_ref.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
}

/// The whole detection, as one blocking file read. Never returns `Err` and never panics — every
/// failure is `None`, i.e. "this project has no row" (SPEC.md §11).
pub fn detect(dir: &Path) -> Option<RepoTarget> {
    let git_dir = find_git_dir(dir)?;
    let config = std::fs::read_to_string(git_dir.join("config")).ok()?;
    let (owner, repo) = github_slug(parse_origin_url(&config)?)?;
    let git_ref = std::fs::read_to_string(git_dir.join("HEAD"))
        .ok()
        .as_deref()
        .and_then(parse_head)
        .filter(|r| is_url_safe_ref(r));
    Some(RepoTarget { owner, repo, git_ref })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hangar-repo-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_repo(root: &Path, config: &str, head: &str) {
        let git = root.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("config"), config).unwrap();
        std::fs::write(git.join("HEAD"), head).unwrap();
    }

    const ORIGIN_HTTPS: &str = "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = https://github.com/anas/hangar.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n";

    /// Plan 062 step 1's first case: an HTTPS remote, with and without the `.git` suffix, and with
    /// the userinfo form git writes after a credential-helper login.
    #[test]
    fn https_remotes_resolve_to_owner_and_repo() {
        assert_eq!(
            github_slug("https://github.com/anas/hangar.git"),
            Some(("anas".into(), "hangar".into()))
        );
        assert_eq!(
            github_slug("https://github.com/anas/hangar"),
            Some(("anas".into(), "hangar".into()))
        );
        assert_eq!(
            github_slug("http://github.com/anas/hangar/"),
            Some(("anas".into(), "hangar".into()))
        );
    }

    /// A remote URL can carry a credential. The slug must come back with the userinfo discarded —
    /// and, because only the two segments are returned, there is no return path a token could take.
    #[test]
    fn a_credential_in_the_remote_url_is_discarded_and_never_returned() {
        let slug = github_slug("https://ghp_EXAMPLETOKEN123@github.com/anas/hangar.git");
        assert_eq!(slug, Some(("anas".into(), "hangar".into())));
        let (owner, repo) = slug.unwrap();
        assert!(!owner.contains("ghp_") && !repo.contains("ghp_"));
    }

    /// Plan 062 step 1's second case: SSH remotes, both the scp-like and the `ssh://` forms.
    #[test]
    fn ssh_remotes_resolve_to_owner_and_repo() {
        assert_eq!(
            github_slug("git@github.com:anas/hangar.git"),
            Some(("anas".into(), "hangar".into()))
        );
        assert_eq!(
            github_slug("ssh://git@github.com/anas/hangar.git"),
            Some(("anas".into(), "hangar".into()))
        );
        assert_eq!(
            github_slug("git://github.com/anas/hangar.git"),
            Some(("anas".into(), "hangar".into()))
        );
    }

    /// Plan 062 step 1's fourth case: a non-GitHub host is *no repo*, never an error — and a
    /// lookalike domain must not slip through a substring check.
    #[test]
    fn non_github_hosts_including_lookalikes_are_no_repo() {
        assert_eq!(github_slug("git@gitlab.com:anas/hangar.git"), None);
        assert_eq!(github_slug("https://github.example.com/anas/hangar.git"), None);
        assert_eq!(github_slug("https://github.com.evil.net/anas/hangar.git"), None);
        assert_eq!(github_slug("https://ghe.corp.internal/anas/hangar.git"), None);
        // Not two path segments.
        assert_eq!(github_slug("https://github.com/anas"), None);
        assert_eq!(github_slug("https://github.com/anas/hangar/tree/main"), None);
        // A segment that could steer the request path somewhere else.
        assert_eq!(github_slug("https://github.com/../hangar"), None);
        assert_eq!(github_slug("https://github.com/anas/hangar?x=1"), None);
    }

    /// Plan 062 step 1's third case: a config with remotes but no `origin` is *no repo*.
    #[test]
    fn a_missing_origin_is_no_repo() {
        let config = "[remote \"upstream\"]\n\turl = https://github.com/other/hangar.git\n";
        assert_eq!(parse_origin_url(config), None);
        // …and a section that only *looks* like origin does not count.
        assert_eq!(parse_origin_url("[remote \"origin-backup\"]\n\turl = x\n"), None);
    }

    /// `url` is read only from inside the origin section, comments are skipped, and the section
    /// name is matched case-insensitively the way git does.
    #[test]
    fn the_origin_url_is_read_from_its_own_section_only() {
        let config = "# a comment\n[core]\n\turl = https://github.com/wrong/wrong.git\n\
                      [REMOTE \"origin\"]\n\t; another comment\n\tURL=git@github.com:anas/hangar.git\n";
        assert_eq!(parse_origin_url(config), Some("git@github.com:anas/hangar.git"));
    }

    #[test]
    fn head_yields_the_branch_when_attached_and_the_sha_when_detached() {
        assert_eq!(parse_head("ref: refs/heads/main\n"), Some("main".to_string()));
        assert_eq!(
            parse_head("ref: refs/heads/feature/inbox\n"),
            Some("feature/inbox".to_string())
        );
        assert_eq!(
            parse_head("bb0748b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7\n"),
            Some("bb0748b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7".to_string())
        );
        assert_eq!(parse_head("ref: refs/tags/v1\n"), None);
        assert_eq!(parse_head(""), None);
    }

    /// The URL-injection guard. A branch name is allowed to contain `/`; nothing may contain the
    /// characters that would change which endpoint is called.
    #[test]
    fn only_url_safe_refs_are_ever_put_in_a_request_path() {
        assert!(is_url_safe_ref("main"));
        assert!(is_url_safe_ref("feature/inbox-build-status"));
        assert!(is_url_safe_ref("v1.2.3"));
        assert!(!is_url_safe_ref("../../../user"));
        assert!(!is_url_safe_ref("main?per_page=1"));
        assert!(!is_url_safe_ref("main#frag"));
        assert!(!is_url_safe_ref("main%2F"));
        assert!(!is_url_safe_ref("has space"));
        assert!(!is_url_safe_ref(""));
    }

    /// End to end against real files: an HTTPS origin and an attached HEAD.
    #[test]
    fn detect_reads_owner_repo_and_branch_from_the_git_directory() {
        let dir = scratch("detect-https");
        write_repo(&dir, ORIGIN_HTTPS, "ref: refs/heads/main\n");
        assert_eq!(
            detect(&dir),
            Some(RepoTarget {
                owner: "anas".into(),
                repo: "hangar".into(),
                git_ref: Some("main".into())
            })
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The walk-up: a monorepo package resolves to the repository it lives in.
    #[test]
    fn a_package_inside_a_repository_resolves_to_that_repository() {
        let dir = scratch("detect-monorepo");
        write_repo(&dir, ORIGIN_HTTPS, "ref: refs/heads/main\n");
        let package = dir.join("apps").join("web");
        std::fs::create_dir_all(&package).unwrap();
        assert_eq!(detect(&package).map(|t| t.slug()), Some("anas/hangar".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Plan 062 step 1's fifth case: a `.git` FILE (a linked worktree or a submodule) is *no
    /// repo*, and the walk must not continue past it into an unrelated parent repository.
    #[test]
    fn a_git_file_is_no_repo_and_does_not_fall_through_to_the_parent() {
        let dir = scratch("detect-worktree");
        write_repo(&dir, ORIGIN_HTTPS, "ref: refs/heads/main\n");
        let worktree = dir.join("wt");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(worktree.join(".git"), "gitdir: /somewhere/.git/worktrees/wt\n").unwrap();
        assert_eq!(detect(&worktree), None);
        assert_eq!(find_git_dir(&worktree), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A folder with no `.git` anywhere above it, and a folder that is simply gone, are both *no
    /// repo* — never an error.
    #[test]
    fn no_git_and_a_missing_folder_are_both_no_repo() {
        let dir = scratch("detect-plain");
        assert_eq!(detect(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(detect(&dir), None);
        assert_eq!(detect(Path::new("/definitely/not/here/hangar")), None);
    }

    /// A repository whose HEAD cannot be read still has a row — with **no ref**, which the caller
    /// turns into `unknown`. Dropping it would be indistinguishable from a project that is fine.
    #[test]
    fn an_unreadable_head_leaves_the_repo_detected_but_without_a_ref() {
        let dir = scratch("detect-no-head");
        write_repo(&dir, ORIGIN_HTTPS, "");
        let target = detect(&dir).expect("origin still resolves");
        assert_eq!(target.slug(), "anas/hangar");
        assert_eq!(target.git_ref, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC.md §11: two cards sharing one repo root are one row; two clones on different branches
    /// are not, because they are two different facts.
    #[test]
    fn rows_merge_on_owner_repo_and_ref_only() {
        let a = RepoTarget { owner: "anas".into(), repo: "hangar".into(), git_ref: Some("main".into()) };
        let b = RepoTarget { owner: "Anas".into(), repo: "Hangar".into(), git_ref: Some("main".into()) };
        let c = RepoTarget { owner: "anas".into(), repo: "hangar".into(), git_ref: Some("dev".into()) };
        assert!(a.same_row(&b));
        assert!(!a.same_row(&c));
    }

    /// The trap this module exists to avoid, asserted on the source text itself: detection reads
    /// `.git/config`, and the moment it spawns a git subprocess instead — which `run_lookup` would
    /// run with **no `cwd`**, against Hangar's own directory — every registered project reports the
    /// same repository. Plan 062's own grep gate is the other half of this check, which is why the
    /// phrase it searches for appears nowhere in this file, comments included.
    ///
    /// Each needle is assembled at run time so this test's own source text cannot satisfy the
    /// search it performs (the same device `vcs.rs` uses for its subcommand test).
    #[test]
    fn this_module_never_spawns_a_subprocess() {
        let source = include_str!("repo.rs");
        for parts in [["process", "::spawn"], ["Command", "::new"], ["std", "::process"]] {
            let needle = parts.concat();
            assert!(
                !source.contains(&needle),
                "detection must read .git/config, never spawn `{needle}`"
            );
        }
    }
}
