//! SPEC.md §18 / plan 053 — the `Secret` newtype. The whole feature's safety rests on this type
//! being structurally unable to reach a toast, a log line, an error string or a panic: no
//! `Display` anywhere, and a `Debug` that emits a fixed redaction and nothing else.

/// A GitHub personal access token. Never a `String` in any signature that returns
/// `Result<_, String>` (SPEC.md §18) — every command that might carry one returns a typed
/// `GithubStatus`/`GithubError` instead (see `commands.rs`, `error.rs`).
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(token: String) -> Self {
        Self(token)
    }

    /// The ONLY way the raw bytes ever leave this type — called at exactly two places in the
    /// whole codebase: `client.rs`'s `send()` (building the `Authorization` header fresh on
    /// every request, never cached) and `keychain.rs`'s `store()`. Named for what it costs, not
    /// what it returns.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    /// Fixed redaction — never the wrapped value, never its length, never a prefix.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC.md §18's guard test: the one thing that must never happen is the token's bytes
    /// reaching a `{:?}` print. `Secret` has no `Display` impl at all — that half of the
    /// invariant is enforced by the compiler (nothing can format it with `{}`, or the crate
    /// fails to build), not by a runtime assertion.
    #[test]
    fn debug_redaction_contains_no_token_bytes() {
        let secret = Secret::new("ghp_TESTTOKEN1234567890abcdefghij".to_string());
        let debug_str = format!("{secret:?}");
        assert_eq!(debug_str, "Secret(<redacted>)");
        assert!(!debug_str.contains("TESTTOKEN"));
    }

    #[test]
    fn expose_returns_exactly_the_wrapped_token() {
        let secret = Secret::new("abc123".to_string());
        assert_eq!(secret.expose(), "abc123");
    }
}
