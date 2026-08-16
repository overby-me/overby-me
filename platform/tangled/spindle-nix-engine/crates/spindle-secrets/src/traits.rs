//! Secrets manager trait definitions.
//!
//! Defines the [`Manager`] trait that all secrets backends must implement,
//! and the [`Stopper`] trait for backends that need graceful shutdown
//! (e.g. OpenBao token renewal cancellation).
//!
//! Matches the upstream Go `secrets.Manager` interface:
//! ```go
//! type Manager interface {
//!     GetSecretsUnlocked(repo string) ([]UnlockedSecret, error)
//!     PutSecret(repo, key, value string) error
//!     DeleteSecret(repo, key string) error
//!     ListSecrets(repo string) ([]string, error)
//! }
//! ```

use async_trait::async_trait;
use spindle_models::UnlockedSecret;

use crate::SecretsError;

/// Core secrets management trait.
///
/// Each backend (SQLite, OpenBao) implements this trait to provide
/// CRUD operations on per-repository secrets. Secrets are scoped by
/// repository path (e.g. `"did:plc:alice/myrepo"`).
///
/// All methods are async to support network-backed implementations
/// (OpenBao). The SQLite backend wraps its synchronous operations
/// in blocking tasks.
///
/// # Repository Path Convention
///
/// The `repo` parameter uses the format `"{owner_did}/{repo_name}"`,
/// e.g. `"did:plc:abc123/my-project"`. Backends may sanitize this
/// path for storage (e.g. replacing `:` and `/` with `_`).
#[async_trait]
pub trait Manager: Send + Sync {
    /// Retrieve all secrets for a repository, decrypted and ready for injection.
    ///
    /// Returns an empty `Vec` if the repository has no secrets.
    /// Each returned [`UnlockedSecret`] has its `key` set to the secret name
    /// (used as the environment variable name) and `value` set to the
    /// decrypted secret content.
    async fn get_secrets_unlocked(&self, repo: &str) -> Result<Vec<UnlockedSecret>, SecretsError>;

    /// Store a secret for a repository.
    ///
    /// If a secret with the same `key` already exists for this repository,
    /// it is overwritten (upsert semantics), matching the upstream Go behavior.
    ///
    /// # Arguments
    /// * `repo` — Repository path (e.g. `"did:plc:alice/myrepo"`).
    /// * `key` — Secret name / environment variable name.
    /// * `value` — The secret value to store (will be encrypted at rest).
    async fn put_secret(&self, repo: &str, key: &str, value: &str) -> Result<(), SecretsError>;

    /// Delete a secret for a repository.
    ///
    /// Returns `Ok(())` even if the secret did not exist (idempotent delete),
    /// matching the upstream Go behavior.
    ///
    /// # Arguments
    /// * `repo` — Repository path.
    /// * `key` — Secret name to delete.
    async fn delete_secret(&self, repo: &str, key: &str) -> Result<(), SecretsError>;

    /// List secret names (keys) for a repository.
    ///
    /// Returns only the key names, not the values. Useful for displaying
    /// available secrets in the UI without exposing their content.
    ///
    /// Returns an empty `Vec` if the repository has no secrets.
    async fn list_secrets(&self, repo: &str) -> Result<Vec<String>, SecretsError>;
}

/// Trait for secrets backends that require graceful shutdown.
///
/// For example, the OpenBao backend may have a background task for
/// token renewal that needs to be cancelled on shutdown. The SQLite
/// backend does not need this — it simply closes its connection.
///
/// Matches the upstream Go `Stopper` interface:
/// ```go
/// type Stopper interface {
///     Stop()
/// }
/// ```
pub trait Stopper: Send + Sync {
    /// Signal the backend to stop any background tasks and release resources.
    ///
    /// This method should be idempotent — calling it multiple times should
    /// not panic or produce errors.
    fn stop(&self);
}
