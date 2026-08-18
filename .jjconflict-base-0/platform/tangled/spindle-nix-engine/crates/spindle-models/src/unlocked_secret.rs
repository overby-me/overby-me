//! Unlocked (decrypted) secret type for workflow step injection.
//!
//! This type represents a secret that has been retrieved from the secrets
//! backend and decrypted, ready to be injected as an environment variable
//! into a workflow step's execution environment.
//!
//! Defined in `spindle-models` (rather than `spindle-secrets` or `spindle-engine`)
//! so that both crates can depend on it without circular dependencies.

use serde::{Deserialize, Serialize};

/// An unlocked secret ready for injection into a workflow step's environment.
///
/// Secrets are stored encrypted at rest (in SQLite or OpenBao) and decrypted
/// into this representation when needed for step execution. The `key` becomes
/// the environment variable name, and `value` becomes its value.
///
/// # Log Safety
///
/// Secret values should **never** be logged directly. Use
/// [`SecretMask`](crate::SecretMask) to redact secret values from log output
/// before writing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnlockedSecret {
    /// The secret key / environment variable name.
    pub key: String,
    /// The decrypted secret value.
    pub value: String,
}

impl UnlockedSecret {
    /// Create a new unlocked secret.
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction() {
        let secret = UnlockedSecret::new("API_KEY", "sk-abc123");
        assert_eq!(secret.key, "API_KEY");
        assert_eq!(secret.value, "sk-abc123");
    }

    #[test]
    fn serialization_roundtrip() {
        let secret = UnlockedSecret::new("DB_PASSWORD", "hunter2");
        let json = serde_json::to_string(&secret).unwrap();
        let deserialized: UnlockedSecret = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.key, "DB_PASSWORD");
        assert_eq!(deserialized.value, "hunter2");
    }

    #[test]
    fn clone_independence() {
        let original = UnlockedSecret::new("KEY", "value");
        let cloned = original.clone();
        assert_eq!(original.key, cloned.key);
        assert_eq!(original.value, cloned.value);
    }
}
