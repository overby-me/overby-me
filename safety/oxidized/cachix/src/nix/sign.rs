use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ed25519_dalek::{Signer, SigningKey};

/// Parse a Nix-format secret key: "name:base64data"
pub fn parse_secret_key(key_str: &str) -> Result<(String, SigningKey)> {
    let (name, b64) = key_str
        .split_once(':')
        .context("invalid secret key format, expected 'name:base64data'")?;
    let bytes = BASE64.decode(b64).context("invalid base64 in secret key")?;
    if bytes.len() != 64 {
        bail!(
            "invalid secret key length: expected 64 bytes, got {}",
            bytes.len()
        );
    }
    // Ed25519 secret key is first 32 bytes, public key is last 32
    let secret_bytes: [u8; 32] = bytes[..32]
        .try_into()
        .context("failed to extract secret key bytes")?;
    let signing_key = SigningKey::from_bytes(&secret_bytes);
    Ok((name.to_string(), signing_key))
}

/// Generate a new Ed25519 keypair for Nix signing.
/// Returns (secret_key_string, public_key_string) in Nix format "name:base64".
pub fn generate_keypair(cache_name: &str) -> (String, String) {
    let mut rng = rand::rngs::OsRng;
    let signing_key = SigningKey::generate(&mut rng);
    let verifying_key = signing_key.verifying_key();

    // Secret key format: 64 bytes = 32 byte secret + 32 byte public
    let mut secret_bytes = Vec::with_capacity(64);
    secret_bytes.extend_from_slice(signing_key.as_bytes());
    secret_bytes.extend_from_slice(verifying_key.as_bytes());

    let secret_str = format!("{}-1:{}", cache_name, BASE64.encode(&secret_bytes));
    let public_str = format!(
        "{}-1:{}",
        cache_name,
        BASE64.encode(verifying_key.as_bytes())
    );

    (secret_str, public_str)
}

/// Build the narinfo fingerprint for signing.
/// Format: "1;{store_path};{nar_hash};{nar_size};{sorted_refs}"
pub fn narinfo_fingerprint(
    store_path: &str,
    nar_hash: &str,
    nar_size: u64,
    references: &[String],
) -> String {
    let mut sorted_refs: Vec<&str> = references.iter().map(|s| s.as_str()).collect();
    sorted_refs.sort();
    format!(
        "1;{};{};{};{}",
        store_path,
        nar_hash,
        nar_size,
        sorted_refs.join(",")
    )
}

/// Sign a narinfo fingerprint with the given secret key string.
pub fn sign_narinfo(secret_key_str: &str, fingerprint: &str) -> Result<String> {
    let (name, signing_key) = parse_secret_key(secret_key_str)?;
    let signature = signing_key.sign(fingerprint.as_bytes());
    Ok(format!("{}:{}", name, BASE64.encode(signature.to_bytes())))
}

/// Nix base32 encoding (uses a non-standard alphabet).
const NIX_BASE32_CHARS: &[u8] = b"0123456789abcdfghijklmnpqrsvwxyz";

/// Encode bytes as nix-base32.
pub fn nix_base32_encode(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }
    let len = (data.len() * 8).div_ceil(5);
    let mut out = vec![0u8; len];
    for i in 0..len {
        let bit_offset = i * 5;
        let byte_idx = bit_offset / 8;
        let bit_idx = bit_offset % 8;
        let mut val = (data[byte_idx] >> bit_idx) as usize;
        if bit_idx > 3 && byte_idx + 1 < data.len() {
            val |= (data[byte_idx + 1] as usize) << (8 - bit_idx);
        }
        out[len - 1 - i] = NIX_BASE32_CHARS[val & 0x1f];
    }
    String::from_utf8(out).expect("nix base32 chars are ascii")
}

/// Extract the store hash (first 32 chars of basename) from a store path.
pub fn store_path_hash(store_path: &str) -> Result<String> {
    let basename = store_path
        .strip_prefix("/nix/store/")
        .context("not a valid store path")?;
    if basename.len() < 32 {
        bail!("store path basename too short: {store_path}");
    }
    Ok(basename[..32].to_string())
}

/// Extract the store suffix (everything after the hash) from a store path.
pub fn store_path_suffix(store_path: &str) -> Result<String> {
    let basename = store_path
        .strip_prefix("/nix/store/")
        .context("not a valid store path")?;
    if basename.len() < 33 {
        bail!("store path basename too short: {store_path}");
    }
    // Skip the hash (32 chars) and the dash
    Ok(basename[32..].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    #[test]
    fn test_nix_base32_encode() {
        // SHA256 of empty string
        let hash = sha2::Sha256::digest(b"");
        let encoded = nix_base32_encode(hash.as_slice());
        assert_eq!(encoded.len(), 52);
    }

    #[test]
    fn test_store_path_hash() {
        let path = "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1-test";
        let hash = store_path_hash(path).unwrap();
        assert_eq!(hash, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1");
    }

    #[test]
    fn test_store_path_suffix() {
        let path = "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1-test";
        let suffix = store_path_suffix(path).unwrap();
        assert_eq!(suffix, "-test");
    }

    #[test]
    fn test_narinfo_fingerprint() {
        let fp = narinfo_fingerprint(
            "/nix/store/abc-hello",
            "sha256:xyz",
            1000,
            &["ref-b".to_string(), "ref-a".to_string()],
        );
        assert_eq!(fp, "1;/nix/store/abc-hello;sha256:xyz;1000;ref-a,ref-b");
    }
}
