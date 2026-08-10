//! SPEC.md §18 / plan 053 — the OS keychain, and only the OS keychain: "if the keychain is
//! unavailable, the feature is unavailable — it does not silently fall back to disk."
//!
//! Uses the `keyring` crate's `v1` API (`keyring::Entry` — see Cargo.toml for the feature
//! justification). `SERVICE`/`ACCOUNT` name the one credential this app will ever store.

use crate::github::secret::Secret;

const SERVICE: &str = "com.hangar.app";
const ACCOUNT: &str = "github-personal-access-token";

/// SPEC.md §18's central UI requirement: "a denied keychain must never render as 'no token'."
/// A single variant on purpose — every non-absent failure (`PlatformFailure`, `NoStorageAccess`,
/// `Ambiguous`, `TooLong`, `Invalid`, `BadEncoding`, `NoDefaultStore`, …) collapses to `Denied`
/// rather than being modelled individually. See `plans/README.md`'s Environment-facts entry for
/// why: the executor could not verify which of these macOS actually raises for a declined "wants
/// to use your confidential information" panel, so every one of them is conservatively treated
/// as a refusal instead of guessed at.
#[derive(Debug)]
pub struct KeychainError;

fn entry() -> Result<keyring::Entry, KeychainError> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|_| KeychainError)
}

/// `Ok(None)` is `keyring::Error::NoEntry` — genuinely absent, nothing was ever stored (or it was
/// deleted). Every other error is `Err(KeychainError)` — a refusal, never re-labelled as absent.
pub fn read() -> Result<Option<Secret>, KeychainError> {
    match entry()?.get_password() {
        Ok(token) => Ok(Some(Secret::new(token))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(KeychainError),
    }
}

/// SPEC.md §18: called only after `client::validate` has already proven the token works — see
/// `commands::set_github_token`. Storage itself is never what decides whether a token is good.
pub fn store(secret: &Secret) -> Result<(), KeychainError> {
    entry()?.set_password(secret.expose()).map_err(|_| KeychainError)
}

/// SPEC.md §18: "Removing it must be one obvious action, and must leave no residue." An
/// already-absent entry is treated as success, not a refusal — deleting something already gone
/// is not a denial.
pub fn delete() -> Result<(), KeychainError> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(KeychainError),
    }
}
