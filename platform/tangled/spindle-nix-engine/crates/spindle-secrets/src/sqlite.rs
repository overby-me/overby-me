//! SQLite secrets backend with AES-256-GCM encryption at rest.
//!
//! Secrets are stored in a `secrets` table in the same SQLite database as
//! other spindle state. Each secret value is encrypted using AES-256-GCM
//! with a randomly generated 96-bit nonce. The encryption key is derived
//! from a master key provided at construction time.
//!
//! # Storage Format
//!
//! Each row in the `secrets` table contains:
//! - `repo` — The sanitized repository path (e.g. `"did_plc_alice_myrepo"`).
//! - `key` — The secret name / environment variable name.
//! - `nonce` — The 12-byte AES-GCM nonce, base64-encoded.
//! - `ciphertext` — The encrypted secret value, base64-encoded.
//!
//! # Thread Safety
//!
//! The SQLite connection is wrapped in a `Mutex` for synchronized access.
//! All database operations are run inside `tokio::task::spawn_blocking`
//! to avoid blocking the async runtime.
//!
//! Matches the upstream Go `SqliteManager` behavior.

use std::path::Path;
use std::sync::{Arc, Mutex};

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Nonce};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rusqlite::Connection;
use spindle_models::UnlockedSecret;
use tracing::{debug, instrument};

use crate::traits::Manager;
use crate::{SecretsError, sanitize_repo_path};

/// SQL statement to create the secrets table if it doesn't exist.
const CREATE_TABLE_SQL: &str = "
CREATE TABLE IF NOT EXISTS secrets (
    repo       TEXT NOT NULL,
    key        TEXT NOT NULL,
    nonce      TEXT NOT NULL,
    ciphertext TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo, key)
)";

/// SQLite-backed secrets manager with AES-256-GCM encryption at rest.
///
/// Suitable for single-node deployments where secrets can be stored
/// alongside other spindle state in the same SQLite database file.
#[derive(Clone)]
pub struct SqliteManager {
    inner: Arc<SqliteManagerInner>,
}

impl std::fmt::Debug for SqliteManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteManager").finish_non_exhaustive()
    }
}

struct SqliteManagerInner {
    conn: Mutex<Connection>,
    cipher: Aes256Gcm,
}

impl SqliteManager {
    /// Create a new SQLite secrets manager.
    ///
    /// Opens (or creates) the SQLite database at the given path and
    /// initializes the `secrets` table. The `master_key` must be exactly
    /// 32 bytes (256 bits) and is used as the AES-256-GCM encryption key.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened, the table cannot
    /// be created, or the master key is not exactly 32 bytes.
    pub fn new(db_path: &Path, master_key: &[u8]) -> Result<Self, SecretsError> {
        if master_key.len() != 32 {
            return Err(SecretsError::InvalidKey(format!(
                "master key must be exactly 32 bytes, got {}",
                master_key.len()
            )));
        }

        let conn = Connection::open(db_path).map_err(|e| SecretsError::Database(e.to_string()))?;

        // Enable WAL mode for better concurrent read performance.
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| SecretsError::Database(e.to_string()))?;

        // Create the secrets table if it doesn't exist.
        conn.execute_batch(CREATE_TABLE_SQL)
            .map_err(|e| SecretsError::Database(e.to_string()))?;

        let cipher = Aes256Gcm::new_from_slice(master_key)
            .map_err(|e| SecretsError::Encryption(e.to_string()))?;

        Ok(Self {
            inner: Arc::new(SqliteManagerInner {
                conn: Mutex::new(conn),
                cipher,
            }),
        })
    }

    /// Create a new SQLite secrets manager using an in-memory database.
    ///
    /// Useful for testing. The database is ephemeral and will be lost
    /// when the manager is dropped.
    #[cfg(test)]
    pub fn new_in_memory(master_key: &[u8]) -> Result<Self, SecretsError> {
        if master_key.len() != 32 {
            return Err(SecretsError::InvalidKey(format!(
                "master key must be exactly 32 bytes, got {}",
                master_key.len()
            )));
        }

        let conn =
            Connection::open_in_memory().map_err(|e| SecretsError::Database(e.to_string()))?;

        conn.execute_batch(CREATE_TABLE_SQL)
            .map_err(|e| SecretsError::Database(e.to_string()))?;

        let cipher = Aes256Gcm::new_from_slice(master_key)
            .map_err(|e| SecretsError::Encryption(e.to_string()))?;

        Ok(Self {
            inner: Arc::new(SqliteManagerInner {
                conn: Mutex::new(conn),
                cipher,
            }),
        })
    }

    /// Encrypt a plaintext value using AES-256-GCM.
    ///
    /// Returns the (nonce, ciphertext) pair, both as raw bytes.
    fn encrypt(&self, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), SecretsError> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .inner
            .cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| SecretsError::Encryption(e.to_string()))?;
        Ok((nonce.to_vec(), ciphertext))
    }

    /// Decrypt a ciphertext using AES-256-GCM.
    ///
    /// The `nonce_bytes` must be exactly 12 bytes.
    #[cfg_attr(not(test), allow(dead_code))]
    fn decrypt(&self, nonce_bytes: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, SecretsError> {
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = self
            .inner
            .cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| SecretsError::Decryption(e.to_string()))?;
        Ok(plaintext)
    }
}

#[async_trait]
impl Manager for SqliteManager {
    #[instrument(skip(self), fields(repo = %repo))]
    async fn get_secrets_unlocked(&self, repo: &str) -> Result<Vec<UnlockedSecret>, SecretsError> {
        let sanitized = sanitize_repo_path(repo);
        let inner = self.inner.clone();

        tokio::task::spawn_blocking(move || {
            let conn = inner
                .conn
                .lock()
                .map_err(|e| SecretsError::Database(e.to_string()))?;

            let mut stmt = conn
                .prepare("SELECT key, nonce, ciphertext FROM secrets WHERE repo = ?1")
                .map_err(|e| SecretsError::Database(e.to_string()))?;

            let rows = stmt
                .query_map([&sanitized], |row| {
                    let key: String = row.get(0)?;
                    let nonce_b64: String = row.get(1)?;
                    let ct_b64: String = row.get(2)?;
                    Ok((key, nonce_b64, ct_b64))
                })
                .map_err(|e| SecretsError::Database(e.to_string()))?;

            let cipher = &inner.cipher;
            let mut secrets = Vec::new();
            for row in rows {
                let (key, nonce_b64, ct_b64) =
                    row.map_err(|e| SecretsError::Database(e.to_string()))?;

                let nonce_bytes = BASE64
                    .decode(&nonce_b64)
                    .map_err(|e| SecretsError::Decryption(e.to_string()))?;
                let ct_bytes = BASE64
                    .decode(&ct_b64)
                    .map_err(|e| SecretsError::Decryption(e.to_string()))?;

                let nonce = Nonce::from_slice(&nonce_bytes);
                let plaintext = cipher
                    .decrypt(nonce, ct_bytes.as_ref())
                    .map_err(|e| SecretsError::Decryption(e.to_string()))?;

                let value = String::from_utf8(plaintext)
                    .map_err(|e| SecretsError::Decryption(e.to_string()))?;

                secrets.push(UnlockedSecret::new(key, value));
            }

            debug!(repo = sanitized, count = secrets.len(), "retrieved secrets");
            Ok(secrets)
        })
        .await
        .map_err(|e| SecretsError::Other(e.to_string()))?
    }

    #[instrument(skip(self, value), fields(repo = %repo, key = %key))]
    async fn put_secret(&self, repo: &str, key: &str, value: &str) -> Result<(), SecretsError> {
        let sanitized = sanitize_repo_path(repo);
        let key = key.to_owned();

        // Encrypt outside the blocking task since we need &self.
        let (nonce_bytes, ct_bytes) = self.encrypt(value.as_bytes())?;
        let nonce_b64 = BASE64.encode(&nonce_bytes);
        let ct_b64 = BASE64.encode(&ct_bytes);

        let inner = self.inner.clone();

        tokio::task::spawn_blocking(move || {
            let conn = inner
                .conn
                .lock()
                .map_err(|e| SecretsError::Database(e.to_string()))?;

            conn.execute(
                "INSERT INTO secrets (repo, key, nonce, ciphertext, updated_at)
                 VALUES (?1, ?2, ?3, ?4, datetime('now'))
                 ON CONFLICT(repo, key) DO UPDATE SET
                     nonce = excluded.nonce,
                     ciphertext = excluded.ciphertext,
                     updated_at = datetime('now')",
                rusqlite::params![sanitized, key, nonce_b64, ct_b64],
            )
            .map_err(|e| SecretsError::Database(e.to_string()))?;

            debug!(repo = sanitized, key = key, "stored secret");
            Ok(())
        })
        .await
        .map_err(|e| SecretsError::Other(e.to_string()))?
    }

    #[instrument(skip(self), fields(repo = %repo, key = %key))]
    async fn delete_secret(&self, repo: &str, key: &str) -> Result<(), SecretsError> {
        let sanitized = sanitize_repo_path(repo);
        let key = key.to_owned();
        let inner = self.inner.clone();

        tokio::task::spawn_blocking(move || {
            let conn = inner
                .conn
                .lock()
                .map_err(|e| SecretsError::Database(e.to_string()))?;

            conn.execute(
                "DELETE FROM secrets WHERE repo = ?1 AND key = ?2",
                rusqlite::params![sanitized, key],
            )
            .map_err(|e| SecretsError::Database(e.to_string()))?;

            debug!(repo = sanitized, key = key, "deleted secret");
            Ok(())
        })
        .await
        .map_err(|e| SecretsError::Other(e.to_string()))?
    }

    #[instrument(skip(self), fields(repo = %repo))]
    async fn list_secrets(&self, repo: &str) -> Result<Vec<String>, SecretsError> {
        let sanitized = sanitize_repo_path(repo);
        let inner = self.inner.clone();

        tokio::task::spawn_blocking(move || {
            let conn = inner
                .conn
                .lock()
                .map_err(|e| SecretsError::Database(e.to_string()))?;

            let mut stmt = conn
                .prepare("SELECT key FROM secrets WHERE repo = ?1 ORDER BY key")
                .map_err(|e| SecretsError::Database(e.to_string()))?;

            let keys: Vec<String> = stmt
                .query_map([&sanitized], |row| row.get(0))
                .map_err(|e| SecretsError::Database(e.to_string()))?
                .collect::<Result<_, _>>()
                .map_err(|e| SecretsError::Database(e.to_string()))?;

            debug!(repo = sanitized, count = keys.len(), "listed secrets");
            Ok(keys)
        })
        .await
        .map_err(|e| SecretsError::Other(e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a deterministic 32-byte test key.
    fn test_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        for (i, byte) in key.iter_mut().enumerate() {
            *byte = i as u8;
        }
        key
    }

    fn make_manager() -> SqliteManager {
        SqliteManager::new_in_memory(&test_key()).unwrap()
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let mgr = make_manager();
        let plaintext = b"super-secret-value-123";
        let (nonce, ciphertext) = mgr.encrypt(plaintext).unwrap();
        assert_eq!(nonce.len(), 12); // AES-GCM uses 96-bit nonces
        assert_ne!(ciphertext.as_slice(), plaintext); // Ciphertext != plaintext
        let decrypted = mgr.decrypt(&nonce, &ciphertext).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_produces_unique_nonces() {
        let mgr = make_manager();
        let (nonce1, _) = mgr.encrypt(b"value").unwrap();
        let (nonce2, _) = mgr.encrypt(b"value").unwrap();
        // Two encryptions of the same value should use different nonces
        assert_ne!(nonce1, nonce2);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let mgr1 = make_manager();
        let (nonce, ciphertext) = mgr1.encrypt(b"secret").unwrap();

        let mut other_key = [0u8; 32];
        other_key[0] = 0xFF;
        let mgr2 = SqliteManager::new_in_memory(&other_key).unwrap();
        let result = mgr2.decrypt(&nonce, &ciphertext);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_key_length_rejected() {
        let short_key = [0u8; 16];
        let result = SqliteManager::new_in_memory(&short_key);
        assert!(result.is_err());
        match result.unwrap_err() {
            SecretsError::InvalidKey(msg) => {
                assert!(msg.contains("32 bytes"));
            }
            other => panic!("expected InvalidKey, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn put_and_get_secrets() {
        let mgr = make_manager();

        mgr.put_secret("did:plc:alice/myrepo", "API_KEY", "sk-abc123")
            .await
            .unwrap();
        mgr.put_secret("did:plc:alice/myrepo", "DB_PASS", "hunter2")
            .await
            .unwrap();

        let secrets = mgr
            .get_secrets_unlocked("did:plc:alice/myrepo")
            .await
            .unwrap();
        assert_eq!(secrets.len(), 2);

        // Secrets are returned in arbitrary order; sort for deterministic comparison.
        let mut sorted: Vec<_> = secrets
            .iter()
            .map(|s| (s.key.as_str(), s.value.as_str()))
            .collect();
        sorted.sort_by_key(|(k, _)| k.to_string());

        assert_eq!(sorted[0], ("API_KEY", "sk-abc123"));
        assert_eq!(sorted[1], ("DB_PASS", "hunter2"));
    }

    #[tokio::test]
    async fn put_overwrites_existing() {
        let mgr = make_manager();

        mgr.put_secret("did:plc:alice/repo", "KEY", "value1")
            .await
            .unwrap();
        mgr.put_secret("did:plc:alice/repo", "KEY", "value2")
            .await
            .unwrap();

        let secrets = mgr
            .get_secrets_unlocked("did:plc:alice/repo")
            .await
            .unwrap();
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].key, "KEY");
        assert_eq!(secrets[0].value, "value2");
    }

    #[tokio::test]
    async fn delete_secret() {
        let mgr = make_manager();

        mgr.put_secret("did:plc:alice/repo", "KEY", "value")
            .await
            .unwrap();
        mgr.delete_secret("did:plc:alice/repo", "KEY")
            .await
            .unwrap();

        let secrets = mgr
            .get_secrets_unlocked("did:plc:alice/repo")
            .await
            .unwrap();
        assert!(secrets.is_empty());
    }

    #[tokio::test]
    async fn delete_nonexistent_is_ok() {
        let mgr = make_manager();
        let result = mgr.delete_secret("did:plc:alice/repo", "NONEXISTENT").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn list_secrets() {
        let mgr = make_manager();

        mgr.put_secret("did:plc:alice/repo", "B_KEY", "v1")
            .await
            .unwrap();
        mgr.put_secret("did:plc:alice/repo", "A_KEY", "v2")
            .await
            .unwrap();
        mgr.put_secret("did:plc:alice/repo", "C_KEY", "v3")
            .await
            .unwrap();

        let keys = mgr.list_secrets("did:plc:alice/repo").await.unwrap();
        // Should be sorted alphabetically
        assert_eq!(keys, vec!["A_KEY", "B_KEY", "C_KEY"]);
    }

    #[tokio::test]
    async fn list_secrets_empty_repo() {
        let mgr = make_manager();
        let keys = mgr.list_secrets("did:plc:nobody/repo").await.unwrap();
        assert!(keys.is_empty());
    }

    #[tokio::test]
    async fn get_secrets_empty_repo() {
        let mgr = make_manager();
        let secrets = mgr
            .get_secrets_unlocked("did:plc:nobody/repo")
            .await
            .unwrap();
        assert!(secrets.is_empty());
    }

    #[tokio::test]
    async fn secrets_are_repo_scoped() {
        let mgr = make_manager();

        mgr.put_secret("did:plc:alice/repo-a", "KEY", "alice-value")
            .await
            .unwrap();
        mgr.put_secret("did:plc:bob/repo-b", "KEY", "bob-value")
            .await
            .unwrap();

        let alice_secrets = mgr
            .get_secrets_unlocked("did:plc:alice/repo-a")
            .await
            .unwrap();
        assert_eq!(alice_secrets.len(), 1);
        assert_eq!(alice_secrets[0].value, "alice-value");

        let bob_secrets = mgr
            .get_secrets_unlocked("did:plc:bob/repo-b")
            .await
            .unwrap();
        assert_eq!(bob_secrets.len(), 1);
        assert_eq!(bob_secrets[0].value, "bob-value");
    }

    #[tokio::test]
    async fn special_characters_in_values() {
        let mgr = make_manager();

        let special_value = "p@$$w0rd!#%^&*()_+={}\n\ttabs and newlines 🔑";
        mgr.put_secret("did:plc:alice/repo", "SPECIAL", special_value)
            .await
            .unwrap();

        let secrets = mgr
            .get_secrets_unlocked("did:plc:alice/repo")
            .await
            .unwrap();
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].value, special_value);
    }

    #[tokio::test]
    async fn empty_value_is_valid() {
        let mgr = make_manager();

        mgr.put_secret("did:plc:alice/repo", "EMPTY", "")
            .await
            .unwrap();

        let secrets = mgr
            .get_secrets_unlocked("did:plc:alice/repo")
            .await
            .unwrap();
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].value, "");
    }

    #[tokio::test]
    async fn large_value() {
        let mgr = make_manager();

        // 1 MB secret value
        let large_value: String = "A".repeat(1_000_000);
        mgr.put_secret("did:plc:alice/repo", "LARGE", &large_value)
            .await
            .unwrap();

        let secrets = mgr
            .get_secrets_unlocked("did:plc:alice/repo")
            .await
            .unwrap();
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].value, large_value);
    }
}
