//! SPEC.md §18 / plan 053 — the ONE place a GitHub request is built and sent: `send()` below.
//! Every request is bounded TWICE (reqwest's own `.timeout()` AND the outer `tokio::time::timeout`
//! in `bounded_with`) and there are NO retries anywhere in this module.

use std::time::Duration;

use super::error::GithubError;
use super::secret::Secret;

const API_BASE: &str = "https://api.github.com";
/// The client's own per-request timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// The outer bound wrapping the whole send — deliberately a little larger than
/// `REQUEST_TIMEOUT` so THIS is the one that actually fires under normal conditions, and the one
/// a test can assert (SPEC.md §18).
const OUTER_TIMEOUT: Duration = Duration::from_secs(15);

/// What `validate()` proves: the token works, and (when GitHub's response says so) which scopes
/// it carries. Never a secret — just a username and a list of scope names.
pub struct ValidatedUser {
    pub login: String,
    pub scopes: Vec<String>,
}

/// The outer half of the two timeouts (SPEC.md §18). A free function parametrised on the
/// duration, not a `send()`-internal literal, so the guard test below can drive it with a very
/// short bound instead of waiting on the real `OUTER_TIMEOUT` or a real network call.
async fn bounded_with<F, T>(duration: Duration, fut: F) -> Result<T, GithubError>
where
    F: std::future::Future<Output = Result<T, GithubError>>,
{
    match tokio::time::timeout(duration, fut).await {
        Ok(result) => result,
        Err(_) => Err(GithubError::Offline),
    }
}

/// The ONE place a request is built and sent. Builds the `Authorization` header at the call
/// site from `secret` and never stores the composed header anywhere else (SPEC.md §18). No
/// retries: one attempt, bounded twice, and whatever happens is the answer.
async fn send(secret: &Secret, path: &str) -> Result<reqwest::Response, GithubError> {
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|_| GithubError::Offline)?;
    let url = format!("{API_BASE}{path}");

    bounded_with(OUTER_TIMEOUT, async {
        client
            .get(&url)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", secret.expose()))
            .header(reqwest::header::USER_AGENT, "Hangar")
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .map_err(|_| GithubError::Offline)
    })
    .await
}

/// GET /user — proves the token works BEFORE anything is stored (SPEC.md §18). Reads
/// `x-oauth-scopes` when present; classic PATs send it, fine-grained tokens do not, so an absent
/// header just means an empty scope list, never an error.
pub async fn validate(secret: &Secret) -> Result<ValidatedUser, GithubError> {
    #[derive(serde::Deserialize)]
    struct UserBody {
        login: String,
    }

    let response = send(secret, "/user").await?;
    let status = response.status();
    let headers = response.headers().clone();
    if !status.is_success() {
        return Err(classify_error(status, &headers));
    }
    let scopes = scopes_from_headers(&headers);
    let body: UserBody = response.json().await.map_err(|_| GithubError::Offline)?;
    Ok(ValidatedUser { login: body.login, scopes })
}

fn header_str<'a>(headers: &'a reqwest::header::HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

fn header_u64(headers: &reqwest::header::HeaderMap, name: &str) -> Option<u64> {
    header_str(headers, name)?.trim().parse().ok()
}

fn scopes_from_headers(headers: &reqwest::header::HeaderMap) -> Vec<String> {
    header_str(headers, "x-oauth-scopes")
        .map(|raw| raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}

/// SPEC.md §18 / plan 053: "Get the 403 split right. It hinges on `x-ratelimit-remaining`;
/// inverting it swaps the two most confusing messages in the feature." `retry-after` is checked
/// FIRST — GitHub only ever sends it for the secondary (abuse-detection) limit, never the
/// primary one, so its mere presence is unambiguous.
fn classify_error(status: reqwest::StatusCode, headers: &reqwest::header::HeaderMap) -> GithubError {
    if let Some(retry_after_sec) = header_u64(headers, "retry-after") {
        return GithubError::SecondaryRateLimited { retry_after_sec };
    }
    let quota_exhausted = header_str(headers, "x-ratelimit-remaining") == Some("0");
    if quota_exhausted && (status.as_u16() == 403 || status.as_u16() == 429) {
        let reset_at = header_u64(headers, "x-ratelimit-reset")
            .map(|epoch| {
                crate::run::iso8601_utc(std::time::UNIX_EPOCH + Duration::from_secs(epoch))
            })
            .unwrap_or_default();
        return GithubError::RateLimited { reset_at };
    }
    match status.as_u16() {
        401 => GithubError::Unauthorized,
        403 => GithubError::InsufficientScope,
        other => GithubError::Unexpected { status: other },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC.md §18: "the outer one is what a test can assert." `block_on` uses Tauri's runtime —
    /// SPEC.md §4 forbids creating one of our own, tests included (same idiom as
    /// `process.rs`'s async tests). A 5ms bound against a 10s sleep proves the wrapper actually
    /// cuts a hung future off and reads it as `Offline`, with no real network call involved.
    #[test]
    fn the_outer_timeout_fires_and_reads_as_offline() {
        let result: Result<(), GithubError> = tauri::async_runtime::block_on(bounded_with(
            Duration::from_millis(5),
            async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok(())
            },
        ));
        assert_eq!(result, Err(GithubError::Offline));
    }

    fn headers(pairs: &[(&str, &str)]) -> reqwest::header::HeaderMap {
        let mut map = reqwest::header::HeaderMap::new();
        for (k, v) in pairs {
            map.insert(
                reqwest::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        map
    }

    /// SPEC.md §18 / plan 053: "Get the 403 split right." Plenty of quota left (`remaining` != 0)
    /// on a 403 is a genuine scope refusal, never a rate limit.
    #[test]
    fn a_403_with_quota_remaining_is_insufficient_scope_not_a_rate_limit() {
        let h = headers(&[("x-ratelimit-remaining", "42")]);
        assert_eq!(
            classify_error(reqwest::StatusCode::FORBIDDEN, &h),
            GithubError::InsufficientScope
        );
    }

    /// The other half of the same split: `remaining == 0` on a 403 IS the primary rate limit.
    #[test]
    fn a_403_with_zero_remaining_is_rate_limited_not_a_scope_refusal() {
        let h = headers(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "0")]);
        assert!(matches!(
            classify_error(reqwest::StatusCode::FORBIDDEN, &h),
            GithubError::RateLimited { .. }
        ));
    }

    /// `retry-after` marks the SECONDARY limit and must win even over a zero primary quota —
    /// "telling someone to wait an hour when it is 60s is its own bug" (SPEC.md plan 053).
    #[test]
    fn retry_after_wins_as_the_secondary_limit_even_with_zero_primary_quota() {
        let h = headers(&[("retry-after", "60"), ("x-ratelimit-remaining", "0")]);
        assert_eq!(
            classify_error(reqwest::StatusCode::TOO_MANY_REQUESTS, &h),
            GithubError::SecondaryRateLimited { retry_after_sec: 60 }
        );
    }

    #[test]
    fn a_plain_401_is_unauthorized() {
        let h = headers(&[]);
        assert_eq!(classify_error(reqwest::StatusCode::UNAUTHORIZED, &h), GithubError::Unauthorized);
    }
}
