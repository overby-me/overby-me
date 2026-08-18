//! Secrets manager for `tangled-spindle-nix`.
//!
//! This crate provides secret storage and retrieval for workflow execution,
//! supporting both SQLite (encrypted at rest) and OpenBao backends. Matches
//! the upstream Go spindle's secrets management interface.
//!
//! # Backends
//!
//! - **SQLite** — Secrets are encrypted using AES-256-GCM and stored in the same
//!   SQLite database as other spindle state. Suitable for single-node deployments.
//! - **OpenBao** — Proxies secret operations to an OpenBao (Vault-compatible)
//!   server via its HTTP API. Secrets are stored in a KV v2 mount at a
//!   configurable path. Suitable for multi-node or enterprise deployments.
//!
//! # Secret Path Convention
//!
//! Secrets are scoped per-repository using a sanitized path:
//! - `did:plc:alice/myrepo` → `did_plc_alice_myrepo`
//! - Full OpenBao path: `{mount}/repos/{sanitized_repo_path}/{key}`
//!
//! # Injection
//!
//! Secrets are injected into workflow steps as environment variables.
//! The [`UnlockedSecret`](spindle_models::UnlockedSecret) type represents a
//! decrypted secret ready for injection. Secret values are masked in log output
//! via [`SecretMask`](spindle_models::SecretMask).

pub mod openbao;
pub mod sqlite;
pub mod traits;

pub use openbao::OpenBaoManager;
pub use sqlite::SqliteManager;
pub use traits::{Manager, Stopper};

// Re-export the shared `UnlockedSecret` type for convenience.
pub use spindle_models::UnlockedSecret;

/// Errors that can occur during secrets management operations.
#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    /// A database error occurred (SQLite backend).
    #[error("database error: {0}")]
    Database(String),

    /// An encryption error occurred.
    #[error("encryption error: {0}")]
    Encryption(String),

    /// A decryption error occurred (e.g. wrong key, corrupted data).
    #[error("decryption error: {0}")]
    Decryption(String),

    /// The provided encryption key is invalid.
    #[error("invalid encryption key: {0}")]
    InvalidKey(String),

    /// An error occurred communicating with the OpenBao server.
    #[error("openbao error: {0}")]
    OpenBao(String),

    /// A catch-all for other errors.
    #[error("{0}")]
    Other(String),
}

/// Sanitize a repository path for use as a storage key.
///
/// Replaces characters that are problematic in file paths or URLs
/// (`:`, `/`) with underscores. This matches the upstream Go path
/// sanitization convention.
///
/// # Examples
///
/// ```
/// use spindle_secrets::sanitize_repo_path;
///
/// assert_eq!(sanitize_repo_path("did:plc:alice/myrepo"), "did_plc_alice_myrepo");
/// assert_eq!(sanitize_repo_path("did:plc:bob/my-project"), "did_plc_bob_my-project");
/// ```
pub fn sanitize_repo_path(repo: &str) -> String {
    repo.replace([':', '/'], "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_basic_did_path() {
        assert_eq!(
            sanitize_repo_path("did:plc:alice/myrepo"),
            "did_plc_alice_myrepo"
        );
    }

    #[test]
    fn sanitize_multiple_colons_and_slashes() {
        assert_eq!(
            sanitize_repo_path("did:plc:abc123/some/nested/path"),
            "did_plc_abc123_some_nested_path"
        );
    }

    #[test]
    fn sanitize_preserves_hyphens_and_dots() {
        assert_eq!(
            sanitize_repo_path("did:plc:alice/my-repo.v2"),
            "did_plc_alice_my-repo.v2"
        );
    }

    #[test]
    fn sanitize_no_special_chars() {
        assert_eq!(sanitize_repo_path("simple_path"), "simple_path");
    }

    #[test]
    fn sanitize_empty_string() {
        assert_eq!(sanitize_repo_path(""), "");
    }

    #[test]
    fn sanitize_only_colons_and_slashes() {
        assert_eq!(sanitize_repo_path(":/:/"), "____");
    }

    #[test]
    fn error_display_database() {
        let err = SecretsError::Database("connection failed".into());
        assert_eq!(err.to_string(), "database error: connection failed");
    }

    #[test]
    fn error_display_encryption() {
        let err = SecretsError::Encryption("bad cipher".into());
        assert_eq!(err.to_string(), "encryption error: bad cipher");
    }

    #[test]
    fn error_display_decryption() {
        let err = SecretsError::Decryption("authentication tag mismatch".into());
        assert_eq!(
            err.to_string(),
            "decryption error: authentication tag mismatch"
        );
    }

    #[test]
    fn error_display_invalid_key() {
        let err = SecretsError::InvalidKey("wrong length".into());
        assert_eq!(err.to_string(), "invalid encryption key: wrong length");
    }

    #[test]
    fn error_display_openbao() {
        let err = SecretsError::OpenBao("connection refused".into());
        assert_eq!(err.to_string(), "openbao error: connection refused");
    }

    #[test]
    fn error_display_other() {
        let err = SecretsError::Other("something went wrong".into());
        assert_eq!(err.to_string(), "something went wrong");
    }
}
