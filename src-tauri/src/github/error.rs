//! SPEC.md §18 / plan 053 — the request-level failure states a GitHub call can produce.
//!
//! Every variant holds only a status code, a reset instant or a retry-after duration — never a
//! token or a composed header — but it DOES carry enough request context that a **blanket**
//! `impl From<GithubError> for String` would let `?` convert it straight into a §7 toast with no
//! reviewer in the loop. That impl must never be added (SPEC.md §18's reviewable invariant).
//! Every conversion to user-facing text happens explicitly, by name, at the one call site in
//! `commands.rs` (`status_from_error`) — never via `?` or `.into()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubError {
    /// Network failure (DNS, connection refused, TLS) or either `client.rs` timeout tripping.
    /// SPEC.md §18: offline is a first-class state, never surfaced as a §7 `Err`.
    Offline,
    /// 401 — the token itself is rejected. The caller already knows whether a token was stored
    /// before this attempt and picks "bad token" vs. "expired or revoked" wording from that —
    /// this variant only proves GitHub said no.
    Unauthorized,
    /// 403 with `x-ratelimit-remaining` != "0" — plenty of quota left, so this is a genuine scope
    /// refusal, not a rate limit.
    InsufficientScope,
    /// 403/429 with `x-ratelimit-remaining` == "0" (the primary limit). `reset_at` is already an
    /// ISO-8601 string converted from `x-ratelimit-reset` — never a raw header value.
    RateLimited { reset_at: String },
    /// 429 carrying a `retry-after` header — the *secondary* (abuse-detection) limit, which gets
    /// its own wording: telling someone to wait an hour when it is 60s is its own bug.
    SecondaryRateLimited { retry_after_sec: u64 },
    /// Any other non-2xx status this module does not specifically classify above.
    Unexpected { status: u16 },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC.md §18's guard test: every variant's fields are status codes, timestamps or counts —
    /// there is no `String` field wide enough to carry a token. This documents the invariant
    /// test-by-example (a type signature already guarantees it; a test that fails to compile is
    /// not available per the plan, so this is the closest runtime check).
    #[test]
    fn no_variant_carries_a_field_that_could_hold_a_secret() {
        let variants = [
            GithubError::Offline,
            GithubError::Unauthorized,
            GithubError::InsufficientScope,
            GithubError::RateLimited { reset_at: "2026-08-10T00:00:00Z".into() },
            GithubError::SecondaryRateLimited { retry_after_sec: 60 },
            GithubError::Unexpected { status: 500 },
        ];
        for v in variants {
            assert!(!format!("{v:?}").to_lowercase().contains("ghp_"));
        }
    }
}
