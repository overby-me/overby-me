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

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
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

/// The present, to the millisecond, as the database's own `strftime` default
/// writes it. A row stamped to the second sorts before one the database stamped
/// earlier in that same second, which put a reaction ahead of the comment it
/// was to.
pub fn now_stamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let whole = rfc3339_utc(now.as_secs());
    format!("{}{:03}Z", &whole[..whole.len() - 4], now.subsec_millis())
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

/// Whether a date a person set names a day and no time of it. Setting the day a
/// node already has must leave its time alone: the editor sends the day back
/// with every save, and a page made a minute ago would read as made at midnight.
pub fn names_a_day(input: &str) -> bool {
    !input.trim().contains(['T', ' '])
}

/// `column`, re-dated to the timestamp bound at `?at`. With `?day` set, a row
/// already dated that day keeps its time of day (see [`names_a_day`]).
pub fn redated_sql(column: &str, at: usize, day: usize) -> String {
    format!(
        "CASE WHEN ?{at} IS NULL THEN {column} \
              WHEN ?{day} = 1 AND substr({column}, 1, 10) = substr(?{at}, 1, 10) THEN {column} \
              ELSE ?{at} END"
    )
}

/// A date a person may set, as timestamps are stored here: ISO-8601, UTC,
/// milliseconds, so that they compare as text. Takes `2026-05-01`, or a UTC
/// RFC 3339 timestamp with or without a fraction. `None` for anything else, an
/// offset other than UTC included: converting one takes a calendar.
pub fn stored_timestamp(input: &str) -> Option<String> {
    let input = input.trim();
    let digits = |s: &str, max: u32| {
        (s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u32>().ok())
            .flatten()
            .filter(|n| *n <= max)
    };
    let (date, time) = match input.split_once(['T', ' ']) {
        Some((date, time)) => (date, Some(time)),
        None => (input, None),
    };
    let mut parts = date.split('-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    let year_ok = year.len() == 4 && year.bytes().all(|b| b.is_ascii_digit());
    let in_range = digits(month, 12)? >= 1 && digits(day, 31)? >= 1;
    if parts.next().is_some() || !year_ok || !in_range {
        return None;
    }
    let Some(time) = time else {
        return Some(format!("{date}T00:00:00.000Z"));
    };
    let time = time
        .strip_suffix('Z')
        .or_else(|| time.strip_suffix("+00:00"))?;
    let (clock, fraction) = time.split_once('.').unwrap_or((time, "0"));
    let mut parts = clock.split(':');
    let (hour, minute, second) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some()
        || fraction.is_empty()
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    digits(hour, 23)?;
    digits(minute, 59)?;
    digits(second, 60)?;
    let millis: String = fraction.chars().chain("000".chars()).take(3).collect();
    Some(format!("{date}T{clock}.{millis}Z"))
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
    fn the_present_is_stamped_to_the_millisecond() {
        let stamp = now_stamp();
        assert_eq!(stamp.len(), "2026-05-01T18:30:00.123Z".len(), "{stamp}");
        assert_eq!(stored_timestamp(&stamp).as_deref(), Some(stamp.as_str()));
    }

    #[test]
    fn a_date_is_stored_as_every_timestamp_is() {
        for (given, stored) in [
            ("2026-05-01", "2026-05-01T00:00:00.000Z"),
            (" 2026-05-01T18:30:00Z ", "2026-05-01T18:30:00.000Z"),
            ("2026-05-01T18:30:00.5Z", "2026-05-01T18:30:00.500Z"),
            (
                "2026-05-01T18:30:00.123456+00:00",
                "2026-05-01T18:30:00.123Z",
            ),
            ("2026-05-01 18:30:00Z", "2026-05-01T18:30:00.000Z"),
        ] {
            assert_eq!(stored_timestamp(given).as_deref(), Some(stored), "{given}");
        }
        for not in [
            "",
            "1. maj",
            "2026-13-01",
            "2026-05-00",
            "26-05-01",
            "2026-05-01T25:00:00Z",
            "2026-05-01T18:30:00+02:00",
            "2026-05-01T18:30:00",
            "2026-05-01T18:30:00.Z",
            "2026-05-01-01",
            "2026-05-01T18:30Z",
        ] {
            assert_eq!(stored_timestamp(not), None, "{not}");
        }
    }

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
