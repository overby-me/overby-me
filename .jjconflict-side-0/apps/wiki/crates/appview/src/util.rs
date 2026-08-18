//! Small shared helpers: URL-safe base64 (no padding), CSRNG bytes/tokens, and
//! the current Unix time. Transferred verbatim from the interim backend
//! (`backend/src/util.rs`): pure, depends only on base64/rand, no query
//! rebinding, so it is the first module to land in the AppView crate.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use rand::RngCore;

pub fn b64url(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

pub fn b64url_decode(s: &str) -> Result<Vec<u8>, String> {
    B64.decode(s).map_err(|e| e.to_string())
}

pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    rand::rngs::OsRng.fill_bytes(&mut v);
    v
}

/// A URL-safe random token of `n` bytes of entropy.
pub fn random_token(n: usize) -> String {
    b64url(&random_bytes(n))
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Format a Unix timestamp as an RFC 3339 / ISO 8601 UTC string (e.g.
/// `2026-07-13T12:34:56.000Z`), for the `createdAt` of an atproto record. Uses
/// Howard Hinnant's civil-from-days algorithm so no date crate is needed.
pub fn rfc3339_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}T{hh:02}:{mm:02}:{ss:02}.000Z")
}

/// Percent-decode one `application/x-www-form-urlencoded` component.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                    }
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse a `a=b&c=d` query string (without the leading `?`) into decoded pairs.
pub fn parse_query(query: Option<&str>) -> Vec<(String, String)> {
    query
        .unwrap_or("")
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_query_decodes_pairs() {
        let q = parse_query(Some("handle=alice.bsky.social&state=a%2Bb&x="));
        assert_eq!(q[0], ("handle".into(), "alice.bsky.social".into()));
        assert_eq!(q[1], ("state".into(), "a+b".into()));
        assert_eq!(q[2], ("x".into(), String::new()));
    }

    #[test]
    fn parse_query_edge_cases() {
        assert!(parse_query(None).is_empty());
        assert!(parse_query(Some("")).is_empty());
        // A bare key with no '=' yields an empty value, not a dropped pair.
        assert_eq!(
            parse_query(Some("flag")),
            vec![("flag".into(), String::new())]
        );
        // Empty segments (leading/trailing/double '&') are skipped.
        assert_eq!(
            parse_query(Some("&a=1&&b=2&")),
            vec![("a".into(), "1".into()), ("b".into(), "2".into())]
        );
        // Only the first '=' splits; later '=' stay in the value.
        assert_eq!(
            parse_query(Some("token=ab=cd")),
            vec![("token".into(), "ab=cd".into())]
        );
        // Duplicate keys are preserved in order (caller decides precedence).
        assert_eq!(
            parse_query(Some("k=1&k=2")),
            vec![("k".into(), "1".into()), ("k".into(), "2".into())]
        );
    }

    #[test]
    fn percent_decode_handles_escapes_and_plus() {
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("%2B"), "+");
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(percent_decode("caf%C3%A9"), "café"); // multi-byte UTF-8
        assert_eq!(percent_decode("caf%c3%a9"), "café"); // lowercase hex
        assert_eq!(percent_decode("plain"), "plain");
    }

    #[test]
    fn percent_decode_leaves_malformed_escapes_literal() {
        // Invalid hex digits: the '%' is passed through untouched.
        assert_eq!(percent_decode("%GG"), "%GG");
        // A truncated escape at the very end has no two following bytes.
        assert_eq!(percent_decode("x%4"), "x%4");
        assert_eq!(percent_decode("%"), "%");
    }

    #[test]
    fn rfc3339_utc_known_vectors() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339_utc(86_400), "1970-01-02T00:00:00.000Z");
        assert_eq!(rfc3339_utc(1_700_000_000), "2023-11-14T22:13:20.000Z");
        // Leap day and end-of-year boundaries exercise the civil-date math.
        assert_eq!(rfc3339_utc(1_582_934_400), "2020-02-29T00:00:00.000Z");
        assert_eq!(rfc3339_utc(1_609_459_199), "2020-12-31T23:59:59.000Z");
    }

    #[test]
    fn b64url_roundtrips_and_is_url_safe() {
        // 0xFB 0xFF encodes to bytes that force the '+' and '/' positions in
        // standard base64; url-safe must use '-'/'_' and no '=' padding.
        let enc = b64url(&[0xFB, 0xFF]);
        assert!(!enc.contains('+') && !enc.contains('/') && !enc.contains('='));
        assert_eq!(b64url_decode(&enc).unwrap(), vec![0xFB, 0xFF]);
        assert_eq!(b64url(&[]), "");
        assert_eq!(b64url_decode("").unwrap(), Vec::<u8>::new());
        assert!(b64url_decode("!!!not base64!!!").is_err());
    }

    #[test]
    fn random_token_has_expected_length_and_varies() {
        // b64url of n bytes (no padding) is ceil(n*4/3) chars; 16 -> 22.
        assert_eq!(random_token(16).len(), 22);
        assert_eq!(random_bytes(16).len(), 16);
        // Two draws must (practically) never collide.
        assert_ne!(random_token(16), random_token(16));
    }
}
