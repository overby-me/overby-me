//! A key as a DID document has it: a multikey, P-256 or secp256k1, the two
//! curves atproto signs with. Signatures are ECDSA over SHA-256, 64 bytes.

/// Whether `sig` is `message` signed by the key `multikey` names. `None` when
/// that is no key of either curve.
pub fn verifies(multikey: &str, message: &[u8], sig: &[u8]) -> Option<bool> {
    let (_, bytes) = multibase::decode(multikey).ok()?;
    match bytes.as_slice() {
        // multicodec secp256k1-pub, then the compressed point
        [0xe7, 0x01, point @ ..] => {
            use k256::ecdsa::signature::Verifier;
            let key = k256::ecdsa::VerifyingKey::from_sec1_bytes(point).ok()?;
            let Ok(sig) = k256::ecdsa::Signature::from_slice(sig) else {
                return Some(false);
            };
            Some(key.verify(message, &sig).is_ok())
        }
        // multicodec p256-pub
        [0x80, 0x24, point @ ..] => {
            use p256::ecdsa::signature::Verifier;
            let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(point).ok()?;
            let Ok(sig) = p256::ecdsa::Signature::from_slice(sig) else {
                return Some(false);
            };
            Some(key.verify(message, &sig).is_ok())
        }
        _ => None,
    }
}

/// A P-256 public key as a multikey, which is how a test (or the AppView's own
/// service document) publishes one.
pub fn p256_multikey(key: &p256::ecdsa::VerifyingKey) -> String {
    let mut bytes = vec![0x80, 0x24];
    bytes.extend_from_slice(key.to_encoded_point(true).as_bytes());
    multibase::encode(multibase::Base::Base58Btc, bytes)
}
