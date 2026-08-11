//! SPEC.md §8 — startup resolution of the user's login-shell environment (`<shell> -ilc 'env'`),
//! so GUI-launched Hangar finds nvm/fnm/volta-managed `npm` on macOS/Linux.
//!
//! A GUI app inherits launchd's minimal environment, not the terminal's. `-ilc` (interactive AND
//! login) is required: a non-interactive login zsh reads `~/.zprofile` but skips `~/.zshrc`, which
//! is exactly where nvm/fnm/volta put node on the most common macOS setup.
//!
//! On Windows this is a no-op — the inherited environment is already the user's.

use std::collections::HashMap;
#[cfg(unix)]
use std::time::Duration;

use tokio::sync::OnceCell;

/// Overlay applied on top of the inherited environment of every spawned child (SPEC.md §8).
/// Empty means "inherit unchanged".
pub type EnvMap = HashMap<String, String>;

/// SPEC.md §8: 5 second budget. On timeout we fall back to the inherited environment — resolution
/// must never block startup.
#[cfg(unix)]
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

/// Variables that describe the *shell's own* situation rather than the user's toolchain. Carrying
/// them into a child would lie about where it is running (we set `cwd` explicitly).
const SKIP_VARS: [&str; 4] = ["PWD", "OLDPWD", "SHLVL", "_"];

/// The resolved dev environment plus any `system` log lines explaining a failure. SPEC.md §8:
/// "On failure: log a `system` line and fall back to the inherited environment."
#[derive(Debug, Clone, Default)]
pub struct DevEnvironment {
    pub vars: EnvMap,
    /// Non-empty only when resolution failed; surfaced into the project's log at Run time so the
    /// user can see *why* their PATH is the bare launchd one.
    pub notes: Vec<String>,
}

impl DevEnvironment {
    /// The PATH a child will actually search, for the "tool not found" log line (SPEC.md §8).
    pub fn effective_path(&self) -> String {
        self.vars
            .get("PATH")
            .cloned()
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_default()
    }
}

/// Resolved exactly once per app run. `get()` awaits an in-flight resolution instead of racing it,
/// so a Run clicked one second after launch cannot silently get the un-resolved environment.
#[derive(Default)]
pub struct DevEnvCell {
    cell: OnceCell<DevEnvironment>,
}

impl DevEnvCell {
    pub async fn get(&self) -> &DevEnvironment {
        self.cell.get_or_init(resolve).await
    }
}

#[cfg(windows)]
async fn resolve() -> DevEnvironment {
    // No-op on Windows: a GUI-launched process already inherits the user's environment.
    DevEnvironment::default()
}

#[cfg(unix)]
async fn resolve() -> DevEnvironment {
    use crate::process::{self, ShellKind, SpawnSpec};

    let shell = login_shell();

    let spec = SpawnSpec {
        command: "env".to_string(),
        shell: ShellKind::LoginInteractive(shell.clone()),
        // Not a project child: nothing will ever need to tree-kill it, so no process group.
        long_lived: false,
        // tokio's own reaper for THIS helper only — if the interactive shell hangs past the 5 s
        // budget, dropping the future must not leave a stray shell behind. This is not the §8 kill
        // path (that is plan 003) and it never touches a project's tree.
        kill_on_drop: true,
        ..SpawnSpec::default()
    };

    let spawned = match process::spawn(&spec) {
        Ok(spawned) => spawned,
        Err(e) => {
            return failed(format!(
                "could not start {shell} to read your shell environment: {e} — \
                 falling back to the environment Hangar was launched with"
            ))
        }
    };

    match tokio::time::timeout(RESOLVE_TIMEOUT, spawned.child.wait_with_output()).await {
        Ok(Ok(output)) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            let vars = parse_env_output(&text);
            if vars.is_empty() {
                failed(format!(
                    "{shell} -ilc 'env' produced no variables — falling back to the environment \
                     Hangar was launched with"
                ))
            } else {
                DevEnvironment {
                    vars,
                    notes: Vec::new(),
                }
            }
        }
        Ok(Ok(output)) => failed(format!(
            "{shell} -ilc 'env' exited with {} — falling back to the environment Hangar was \
             launched with",
            output.status
        )),
        Ok(Err(e)) => failed(format!(
            "could not read the environment from {shell}: {e} — falling back to the environment \
             Hangar was launched with"
        )),
        Err(_) => failed(format!(
            "{shell} -ilc 'env' did not finish within 5 s — falling back to the environment Hangar \
             was launched with"
        )),
    }
}

#[cfg(unix)]
fn failed(note: String) -> DevEnvironment {
    eprintln!("hangar: {note}");
    DevEnvironment {
        vars: EnvMap::new(),
        notes: vec![note],
    }
}

/// SPEC.md §8: `$SHELL`, falling back to `/bin/zsh` on macOS and `/bin/bash` elsewhere.
#[cfg(unix)]
fn login_shell() -> String {
    match std::env::var("SHELL") {
        Ok(s) if !s.trim().is_empty() => s,
        _ => default_shell().to_string(),
    }
}

#[cfg(unix)]
const fn default_shell() -> &'static str {
    if cfg!(target_os = "macos") {
        "/bin/zsh"
    } else {
        "/bin/bash"
    }
}

/// Parse the `KEY=VALUE` output of `env`.
///
/// A line that does not start with a valid variable name followed by `=` is a continuation of the
/// previous value — `env` prints multi-line values verbatim. `=` inside a value is kept, because
/// only the FIRST `=` separates the name from the value.
pub fn parse_env_output(text: &str) -> EnvMap {
    let mut vars = EnvMap::new();
    let mut current: Option<(String, String)> = None;

    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        match split_assignment(line) {
            Some((key, value)) => {
                if let Some((k, v)) = current.take() {
                    vars.insert(k, v);
                }
                current = Some((key.to_string(), value.to_string()));
            }
            None if line.is_empty() => {
                // Blank lines are ignored — the common case is the trailing newline of `env`'s
                // output, and appending it would silently corrupt the last value. (A blank line
                // *inside* a multi-line value is lost; accepted, it has no effect on tooling.)
            }
            None => {
                if let Some((_, value)) = current.as_mut() {
                    value.push('\n');
                    value.push_str(line);
                }
                // else: noise before the first assignment (an interactive shell's banner) — dropped.
            }
        }
    }
    if let Some((k, v)) = current.take() {
        vars.insert(k, v);
    }

    vars.retain(|k, _| !SKIP_VARS.contains(&k.as_str()));
    vars
}

/// `Some((key, value))` only for a line beginning with a POSIX-shaped variable name and `=`.
fn split_assignment(line: &str) -> Option<(&str, &str)> {
    let eq = line.find('=')?;
    let (key, rest) = line.split_at(eq);
    let mut chars = key.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((key, &rest[1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_key_value_lines() {
        let vars = parse_env_output("PATH=/usr/bin\nHOME=/Users/dev\n");
        assert_eq!(vars.get("PATH").map(String::as_str), Some("/usr/bin"));
        assert_eq!(vars.get("HOME").map(String::as_str), Some("/Users/dev"));
        assert_eq!(vars.len(), 2);
    }

    #[test]
    fn keeps_equals_signs_inside_values() {
        let vars = parse_env_output("LS_COLORS=di=1;34:ln=35\nFOO=a=b=c\n");
        assert_eq!(
            vars.get("LS_COLORS").map(String::as_str),
            Some("di=1;34:ln=35")
        );
        assert_eq!(vars.get("FOO").map(String::as_str), Some("a=b=c"));
    }

    #[test]
    fn joins_multi_line_values_and_ignores_banner_noise() {
        // `env` prints a multi-line value verbatim; its continuation lines are not assignments.
        let vars = parse_env_output(
            "Welcome to your shell!\nNVM_DIR=/Users/dev/.nvm\nSCRIPT=line one\nline two\nlast line\nTERM=xterm\n",
        );
        assert_eq!(
            vars.get("NVM_DIR").map(String::as_str),
            Some("/Users/dev/.nvm")
        );
        assert_eq!(
            vars.get("SCRIPT").map(String::as_str),
            Some("line one\nline two\nlast line")
        );
        assert_eq!(vars.get("TERM").map(String::as_str), Some("xterm"));
        assert!(!vars.contains_key("Welcome to your shell!"));
    }

    #[test]
    fn blank_lines_and_shell_bookkeeping_vars_are_dropped() {
        let vars =
            parse_env_output("\nPATH=/usr/bin\n\nPWD=/tmp\nSHLVL=1\n_=/usr/bin/env\nOLDPWD=/\n\n");
        assert_eq!(vars.len(), 1, "only PATH survives, got {vars:?}");
        assert!(vars.contains_key("PATH"));
    }

    #[test]
    fn empty_values_are_preserved() {
        let vars = parse_env_output("EMPTY=\nPATH=/bin\n");
        assert_eq!(vars.get("EMPTY").map(String::as_str), Some(""));
    }

    #[test]
    fn effective_path_prefers_the_resolved_value() {
        let mut vars = EnvMap::new();
        vars.insert("PATH".into(), "/opt/homebrew/bin:/usr/bin".into());
        let env = DevEnvironment {
            vars,
            notes: Vec::new(),
        };
        assert_eq!(env.effective_path(), "/opt/homebrew/bin:/usr/bin");
    }
}
