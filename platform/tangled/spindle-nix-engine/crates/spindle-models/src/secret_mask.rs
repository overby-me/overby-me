//! Secret value masking for log output.
//!
//! Matches the upstream Go `SecretMask` type. When secrets are injected into
//! workflow steps as environment variables, their values must be redacted from
//! any log output to prevent accidental exposure.
//!
//! The mask replaces:
//! - The raw secret value
//! - The base64-encoded secret value (with padding)
//! - The base64-encoded secret value (without padding)
//!
//! All matches are replaced with `"***"`.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

/// Replaces secret values (and their base64 encodings) with `"***"` in strings.
///
/// Matches the upstream Go `SecretMask` struct. Created from a list of secret
/// values; returns `None` if no non-empty values are provided (matching upstream
/// behavior where `NewSecretMask` returns `nil` for empty input).
///
/// # Example
///
/// ```
/// use spindle_models::SecretMask;
///
/// let mask = SecretMask::new(&["my-api-key"]).unwrap();
/// let masked = mask.mask("Token: my-api-key");
/// assert_eq!(masked, "Token: ***");
/// ```
#[derive(Debug, Clone)]
pub struct SecretMask {
    /// (needle, replacement) pairs. We store needles sorted longest-first
    /// so that longer matches take priority over shorter substrings.
    patterns: Vec<String>,
}

impl SecretMask {
    /// Create a new `SecretMask` for the given secret values.
    ///
    /// Also registers base64-encoded variants (with and without padding) of
    /// each secret, matching the upstream Go `NewSecretMask` behavior.
    ///
    /// Returns `None` if no non-empty values are provided (matching upstream
    /// Go behavior where `NewSecretMask` returns `nil`).
    pub fn new(values: &[impl AsRef<str>]) -> Option<Self> {
        let mut patterns = Vec::new();

        for value in values {
            let value = value.as_ref();
            if value.is_empty() {
                continue;
            }

            // Add the raw secret value
            patterns.push(value.to_owned());

            // Add base64-encoded variant
            let b64 = BASE64.encode(value.as_bytes());
            if b64 != value {
                patterns.push(b64.clone());
            }

            // Add base64 without padding (if different from padded version and raw value)
            let b64_no_pad = b64.trim_end_matches('=');
            if b64_no_pad != b64 && b64_no_pad != value {
                patterns.push(b64_no_pad.to_owned());
            }
        }

        if patterns.is_empty() {
            return None;
        }

        // Sort longest-first so longer matches are replaced before shorter substrings
        patterns.sort_by_key(|b| std::cmp::Reverse(b.len()));
        // Deduplicate (after sorting, duplicates are adjacent)
        patterns.dedup();

        Some(Self { patterns })
    }

    /// Replace all registered secret values with `"***"`.
    ///
    /// If called on a `None` (via the `Option<SecretMask>` helper), returns
    /// the input unchanged — use [`SecretMask::mask_optional`] for that pattern.
    pub fn mask(&self, input: &str) -> String {
        let mut result = input.to_owned();
        for pattern in &self.patterns {
            result = result.replace(pattern.as_str(), "***");
        }
        result
    }

    /// Convenience method for `Option<SecretMask>`: mask if present, pass through if `None`.
    ///
    /// Matches the upstream Go nil-receiver pattern where `(*SecretMask).Mask()`
    /// returns the input unchanged when the receiver is nil.
    pub fn mask_optional(mask: Option<&Self>, input: &str) -> String {
        match mask {
            Some(m) => m.mask(input),
            None => input.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_masking() {
        let mask = SecretMask::new(&["mysecret123"]).unwrap();
        let input = "The password is mysecret123 in this log";
        let expected = "The password is *** in this log";
        assert_eq!(mask.mask(input), expected);
    }

    #[test]
    fn base64_encoded() {
        let secret = "mysecret123";
        let mask = SecretMask::new(&[secret]).unwrap();
        let b64 = BASE64.encode(secret.as_bytes());
        let input = format!("Encoded: {b64}");
        assert_eq!(mask.mask(&input), "Encoded: ***");
    }

    #[test]
    fn base64_no_padding() {
        // "test" encodes to "dGVzdA==" with padding
        let secret = "test";
        let mask = SecretMask::new(&[secret]).unwrap();
        let b64_no_pad = "dGVzdA"; // base64 without padding
        let input = format!("Token: {b64_no_pad}");
        assert_eq!(mask.mask(&input), "Token: ***");
    }

    #[test]
    fn multiple_secrets() {
        let mask = SecretMask::new(&["password1", "apikey123"]).unwrap();
        let input = "Using password1 and apikey123 for auth";
        assert_eq!(mask.mask(input), "Using *** and *** for auth");
    }

    #[test]
    fn multiple_occurrences() {
        let mask = SecretMask::new(&["secret"]).unwrap();
        let input = "secret appears twice: secret";
        assert_eq!(mask.mask(input), "*** appears twice: ***");
    }

    #[test]
    fn short_values() {
        let mask = SecretMask::new(&["abc", "xy", ""]).unwrap();
        let input = "abc xy test";
        assert_eq!(mask.mask(input), "*** *** test");
    }

    #[test]
    fn nil_mask_via_optional() {
        let input = "some input text";
        let result = SecretMask::mask_optional(None, input);
        assert_eq!(result, input);
    }

    #[test]
    fn some_mask_via_optional() {
        let mask = SecretMask::new(&["secret"]).unwrap();
        let input = "my secret value";
        let result = SecretMask::mask_optional(Some(&mask), input);
        assert_eq!(result, "my *** value");
    }

    #[test]
    fn empty_input() {
        let mask = SecretMask::new(&["secret"]).unwrap();
        assert_eq!(mask.mask(""), "");
    }

    #[test]
    fn no_match() {
        let mask = SecretMask::new(&["secretvalue"]).unwrap();
        let input = "nothing to mask here";
        assert_eq!(mask.mask(input), input);
    }

    #[test]
    fn empty_secrets_list() {
        let mask = SecretMask::new(&[] as &[&str]);
        assert!(mask.is_none());
    }

    #[test]
    fn all_empty_secrets_filtered() {
        let mask = SecretMask::new(&["", "", ""]);
        assert!(mask.is_none());
    }

    #[test]
    fn mixed_empty_and_valid_secrets() {
        let mask = SecretMask::new(&["ab", "validpassword", "", "xyz"]).unwrap();
        let input = "Using validpassword here";
        assert_eq!(mask.mask(input), "Using *** here");
    }

    #[test]
    fn secret_that_is_already_base64() {
        // If the secret value happens to already be valid base64, we shouldn't
        // double-register it. The mask should still work correctly.
        let secret = "dGVzdA=="; // This is base64 for "test"
        let mask = SecretMask::new(&[secret]).unwrap();
        let input = format!("value: {secret}");
        assert_eq!(mask.mask(&input), "value: ***");
    }

    #[test]
    fn overlapping_secrets_longer_wins() {
        // If one secret is a substring of another, the longer one should be
        // replaced first to avoid partial replacements.
        let mask = SecretMask::new(&["secret", "my-secret-key"]).unwrap();
        let input = "key=my-secret-key";
        let result = mask.mask(input);
        // The longer pattern "my-secret-key" should be replaced as a whole
        assert_eq!(result, "key=***");
    }

    #[test]
    fn serialization_not_required() {
        // SecretMask intentionally does not implement Serialize/Deserialize
        // to prevent accidental serialization of secret patterns.
        // This is a design test — just verify the mask works after clone.
        let mask = SecretMask::new(&["secret"]).unwrap();
        let cloned = mask.clone();
        assert_eq!(cloned.mask("my secret"), "my ***");
    }
}
