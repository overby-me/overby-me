//! Unit name escaping and unescaping.
//!
//! This module implements the systemd unit name escaping rules as documented
//! in `systemd.unit(5)` and `systemd-escape(1)`.
//!
//! ## Escaping rules
//!
//! - `/` is replaced with `-`
//! - All characters except `[a-zA-Z0-9:_.\]` are replaced with `\xHH`
//!   (C-style hex escape using the byte's hex value)
//! - Leading `.` is escaped as `\x2e`
//! - The empty string becomes `-` (representing `/`)
//!
//! ## Path escaping
//!
//! Path escaping is similar but first normalizes the path:
//! - Leading and trailing `/` are stripped
//! - Consecutive `/` are collapsed
//! - The root path `/` becomes `-`
//! - Then normal escaping is applied to the result

/// Characters that do NOT need escaping in a unit name.
/// Matches systemd: ASCII letters, digits, `:`, `_`, `.`
fn is_valid_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == ':' || c == '_' || c == '.'
}

/// Escape a string for use in a systemd unit name.
///
/// This applies the escaping rules from `systemd.unit(5)`:
/// - `/` → `-`
/// - Characters outside `[a-zA-Z0-9:_.\]` → `\xHH`
/// - Leading `.` → `\x2e`
///
/// # Examples
///
/// ```
/// use libsystemd::unit_name::unit_name_escape;
/// assert_eq!(unit_name_escape("foo bar"), r"foo\x20bar");
/// assert_eq!(unit_name_escape("foo/bar"), "foo-bar");
/// assert_eq!(unit_name_escape(".hidden"), r"\x2ehidden");
/// assert_eq!(unit_name_escape(""), "");
/// ```
pub fn unit_name_escape(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let mut result = String::with_capacity(s.len() * 2);

    for (i, c) in s.chars().enumerate() {
        if c == '/' {
            result.push('-');
        } else if i == 0 && c == '.' {
            // Leading dot must be escaped
            result.push_str(&format!("\\x{:02x}", c as u32));
        } else if is_valid_char(c) {
            result.push(c);
        } else {
            // Escape each byte of the UTF-8 encoding
            let mut buf = [0u8; 4];
            let encoded = c.encode_utf8(&mut buf);
            for b in encoded.bytes() {
                result.push_str(&format!("\\x{:02x}", b));
            }
        }
    }

    result
}

/// Unescape a systemd unit name back to the original string.
///
/// This reverses the escaping applied by [`unit_name_escape`]:
/// - `-` → `/`
/// - `\xHH` → the corresponding byte
///
/// Returns `None` if the input contains invalid escape sequences.
///
/// # Examples
///
/// ```
/// use libsystemd::unit_name::unit_name_unescape;
/// assert_eq!(unit_name_unescape(r"foo\x20bar"), Some("foo bar".to_string()));
/// assert_eq!(unit_name_unescape("foo-bar"), Some("foo/bar".to_string()));
/// assert_eq!(unit_name_unescape("-"), Some("/".to_string()));
/// ```
pub fn unit_name_unescape(s: &str) -> Option<String> {
    let mut result = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'-' {
            result.push(b'/');
            i += 1;
        } else if bytes[i] == b'\\' && i + 3 < bytes.len() && bytes[i + 1] == b'x' {
            let hi = hex_digit(bytes[i + 2])?;
            let lo = hex_digit(bytes[i + 3])?;
            result.push(hi << 4 | lo);
            i += 4;
        } else if bytes[i] == b'\\' {
            // Invalid escape sequence
            return None;
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }

    String::from_utf8(result).ok()
}

/// Escape a filesystem path for use in a systemd unit name.
///
/// The path is first normalized:
/// - Leading and trailing `/` are stripped
/// - Consecutive `/` are collapsed
/// - The root path `/` becomes `-`
///
/// Then normal unit name escaping is applied.
///
/// # Examples
///
/// ```
/// use libsystemd::unit_name::unit_name_path_escape;
/// assert_eq!(unit_name_path_escape("/"), "-");
/// assert_eq!(unit_name_path_escape("/foo/bar"), "foo-bar");
/// assert_eq!(unit_name_path_escape("/foo//bar/"), "foo-bar");
/// assert_eq!(unit_name_path_escape("/foo bar/baz"), r"foo\x20bar-baz");
/// ```
/// Escape a filesystem path for use in a systemd unit name, matching C's
/// `unit_name_path_escape` (used by `systemd-escape --path`). The empty input
/// and any root-like path escape to `-`; a *normalized* path, whether absolute
/// (`/foo/bar`) or relative (`foo`, `foo/bar`, `./foo`), is escaped. `None` is
/// returned only for a path that still contains `.`/`..` components after
/// simplification (a bare `.`/`..`, or e.g. `/a/../b`) or one exceeding the unit
/// name length limit. The caller (the CLI) still warns when the input is not a
/// valid or not an absolute path even though escaping succeeds.
pub fn unit_name_path_escape_checked(path: &str) -> Option<String> {
    // The empty string simplifies to empty, so C escapes it to "-". This must
    // come first and key off the *original* input being empty rather than the
    // normalized form: normalize_path() also reduces "." to empty, but a bare
    // "." is a non-absolute path C rejects, handled in the branch below.
    if path.is_empty() {
        return Some("-".to_string());
    }

    let normalized = normalize_path(path);
    if normalized.is_empty() {
        // "/" (and "//", "/.", ...) collapse to root and escape to "-". A
        // relative dotty input such as "." or "./" also simplifies to nothing
        // but is not absolute, and C rejects it (unlike a genuine root path).
        return if path.starts_with('/') {
            Some("-".to_string())
        } else {
            None
        };
    }

    // Reject paths that still contain `..` after normalization
    if !is_valid_normalized_path(&normalized) {
        return None;
    }

    let escaped = unit_name_escape(&normalized);
    // Unit names have a length limit (256 chars)
    if escaped.len() > 256 {
        return None;
    }
    Some(escaped)
}

pub fn unit_name_path_escape(path: &str) -> String {
    let normalized = normalize_path(path);
    if normalized.is_empty() {
        return "-".to_string();
    }
    unit_name_escape(&normalized)
}

/// Unescape a systemd unit name back to a filesystem path.
///
/// This reverses [`unit_name_path_escape`]. The result always starts with `/`.
///
/// Returns `None` if the input contains invalid escape sequences.
///
/// # Examples
///
/// ```
/// use libsystemd::unit_name::unit_name_path_unescape;
/// assert_eq!(unit_name_path_unescape("-"), Some("/".to_string()));
/// assert_eq!(unit_name_path_unescape("foo-bar"), Some("/foo/bar".to_string()));
/// ```
pub fn unit_name_path_unescape(s: &str) -> Option<String> {
    if s.is_empty() {
        return None;
    }

    // Special case: "-" is the escaped form of "/"
    if s == "-" {
        return Some("/".to_string());
    }

    let unescaped = unit_name_unescape(s)?;

    if unescaped.is_empty() {
        return Some("/".to_string());
    }

    // A properly path-escaped string never starts with '/' after unescaping,
    // because unit_name_path_escape strips the leading '/'. If the unescaped
    // result starts with '/', it means the input was not validly path-escaped.
    if unescaped.starts_with('/') {
        return None;
    }

    let path = format!("/{unescaped}");

    // Validate: must be a normalized absolute path
    if !is_normalized_path(&path) {
        return None;
    }

    Some(path)
}

/// Check if a path is normalized (no double slashes, no `.`/`..` components,
/// no trailing slash except for root).
fn is_normalized_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }

    if path == "/" {
        return true;
    }

    // No trailing slash
    if path.ends_with('/') {
        return false;
    }

    // No double slashes
    if path.contains("//") {
        return false;
    }

    // No . or .. components
    for component in path.split('/') {
        if component == "." || component == ".." {
            return false;
        }
    }

    true
}

/// Escape a string for use in a unit name, preserving characters that are
/// valid in unit names (unlike `unit_name_escape` which also escapes `-`).
///
/// This is used by `unit_name_mangle` to escape the name part while
/// preserving `-` and `@` which are valid in unit names.
fn unit_name_mangle_escape(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let mut result = String::with_capacity(s.len() * 2);

    // C's do_escape_mangle only escapes characters outside VALID_CHARS_WITH_AT
    // and maps '/' to '-'. Unlike unit_name_escape (the default escape mode), it
    // does NOT escape a leading '.', because '.' is a valid unit-name character.
    for c in s.chars() {
        if c == '/' {
            // A path separator mangles to '-', not \x2f: C's do_escape_mangle
            // maps '/' -> '-' (only --path escaping keeps the byte value).
            result.push('-');
        } else if is_valid_char(c) || c == '-' || c == '@' || c == '\\' {
            result.push(c);
        } else {
            // Escape each byte of the UTF-8 encoding
            let mut buf = [0u8; 4];
            let encoded = c.encode_utf8(&mut buf);
            for b in encoded.bytes() {
                result.push_str(&format!("\\x{:02x}", b));
            }
        }
    }

    result
}

/// Mangle an arbitrary string into a valid unit name.
///
/// This is similar to `unit_name_escape` but also:
/// - Preserves `-` and `@` which are valid in unit names
/// - Appends `.service` suffix if the result doesn't already have a known
///   unit suffix
/// - Handles the case where the input is already a valid unit name
///
/// This matches `systemd-escape --mangle`.
///
/// # Examples
///
/// ```
/// use libsystemd::unit_name::unit_name_mangle;
/// assert_eq!(unit_name_mangle("foo"), "foo.service");
/// assert_eq!(unit_name_mangle("foo.service"), "foo.service");
/// assert_eq!(unit_name_mangle("foo bar"), r"foo\x20bar.service");
/// assert_eq!(unit_name_mangle("/dev/sda"), "dev-sda.device");
/// assert_eq!(unit_name_mangle("hello-world"), "hello-world.service");
/// assert_eq!(unit_name_mangle("/mount/this"), "mount-this.mount");
/// ```
pub fn unit_name_mangle(s: &str) -> String {
    // If the string already has a recognized unit suffix in front of a non-empty
    // name, mangle-escape the name part and keep the suffix. A bare suffix like
    // ".service" has an empty name, so C's unit_name_to_type() rejects it and a
    // suffix is appended (".service" -> ".service.service"); fall through to that.
    if let Some(suffix) = recognized_suffix(s) {
        let name_part = &s[..s.len() - suffix.len()];
        if !name_part.is_empty() {
            let escaped = unit_name_mangle_escape(name_part);
            return format!("{escaped}{suffix}");
        }
    }

    // If it looks like an absolute path, determine the appropriate unit type.
    if s.starts_with('/') {
        if s.starts_with("/dev/") {
            let escaped = unit_name_path_escape(s);
            return format!("{escaped}.device");
        }
        let escaped = unit_name_path_escape(s);
        return format!("{escaped}.mount");
    }

    // Otherwise, mangle-escape and append .service
    let escaped = unit_name_mangle_escape(s);
    format!("{escaped}.service")
}

/// Extract the template name from a template instance unit name.
///
/// For `foo@bar.service`, returns `Some(("foo@", "bar", ".service"))`.
/// For `foo.service`, returns `None`.
pub fn unit_name_template_split(name: &str) -> Option<(&str, &str, &str)> {
    let at_pos = name.find('@')?;
    let dot_pos = name.rfind('.')?;

    // Must have a non-empty prefix before '@' and '@' must come before the suffix
    if at_pos == 0 || at_pos >= dot_pos {
        return None;
    }

    let prefix = &name[..=at_pos]; // "foo@"
    let instance = &name[at_pos + 1..dot_pos]; // "bar"
    let suffix = &name[dot_pos..]; // ".service"

    Some((prefix, instance, suffix))
}

/// Check if a unit name is a template (contains `@` before the suffix).
pub fn is_template(name: &str) -> bool {
    if let Some(at_pos) = name.find('@')
        && let Some(dot_pos) = name.rfind('.')
    {
        // Template: foo@.service (instance is empty)
        return at_pos < dot_pos && at_pos + 1 == dot_pos;
    }
    false
}

/// Check if a unit name is a template instance (contains `@` with an
/// instance string before the suffix).
pub fn is_instance(name: &str) -> bool {
    if let Some(at_pos) = name.find('@')
        && let Some(dot_pos) = name.rfind('.')
    {
        // Instance: foo@bar.service (instance is non-empty)
        return at_pos < dot_pos && at_pos + 1 < dot_pos;
    }
    false
}

/// Instantiate a template with a given instance string.
///
/// `template` should be like `foo@.service` and `instance` is the
/// instance name (unescaped). Returns `foo@instance.service`.
///
/// Returns `None` if `template` is not a valid template name.
pub fn template_instantiate(template: &str, instance: &str) -> Option<String> {
    if !is_template(template) {
        return None;
    }

    let at_pos = template.find('@')?;
    let dot_pos = template.rfind('.')?;

    let prefix = &template[..=at_pos];
    let suffix = &template[dot_pos..];

    Some(format!("{prefix}{instance}{suffix}"))
}

/// Return the unit type suffix if the name has a recognized one.
fn recognized_suffix(name: &str) -> Option<&'static str> {
    const SUFFIXES: &[&str] = &[
        ".service",
        ".socket",
        ".target",
        ".device",
        ".mount",
        ".automount",
        ".swap",
        ".timer",
        ".path",
        ".slice",
        ".scope",
    ];

    SUFFIXES
        .iter()
        .find(|&&suffix| name.ends_with(suffix))
        .copied()
        .map(|v| v as _)
}

/// Normalize a filesystem path following systemd's `path_simplify` rules:
/// - Remove `.` components
/// - Collapse consecutive `/`
/// - Skip `..` only at the beginning of an absolute path
/// - After a non-`..`/non-`.` component, `..` is kept (and makes the path invalid)
///
/// Returns `None` if the path contains `..` after a real component (invalid path).
/// Returns `Some(normalized)` with the path stripped of leading/trailing slashes.
fn normalize_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut components: Vec<&str> = Vec::new();
    let mut beginning = true;

    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if absolute && beginning {
                    // Skip leading `..` in absolute paths (can't go above root)
                    continue;
                }
                beginning = false;
                components.push(part);
            }
            other => {
                beginning = false;
                components.push(other);
            }
        }
    }
    components.join("/")
}

/// Check if a normalized path is valid for path escaping.
/// Returns `false` if the path contains `..` components.
fn is_valid_normalized_path(path: &str) -> bool {
    for component in path.split('/') {
        if component == ".." {
            return false;
        }
    }
    true
}

/// Parse a single hex digit character to its numeric value.
fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_basic() {
        assert_eq!(unit_name_escape("foobar"), "foobar");
        assert_eq!(unit_name_escape("foo bar"), r"foo\x20bar");
        assert_eq!(unit_name_escape("foo/bar"), "foo-bar");
        assert_eq!(unit_name_escape(""), "");
    }

    #[test]
    fn test_escape_leading_dot() {
        assert_eq!(unit_name_escape(".hidden"), r"\x2ehidden");
        assert_eq!(unit_name_escape(".."), r"\x2e.");
    }

    #[test]
    fn test_escape_special_chars() {
        assert_eq!(unit_name_escape("foo@bar"), r"foo\x40bar");
        assert_eq!(unit_name_escape("a=b"), r"a\x3db");
    }

    #[test]
    fn test_escape_allowed_chars() {
        // These should pass through unchanged
        assert_eq!(unit_name_escape("foo_bar"), "foo_bar");
        assert_eq!(unit_name_escape("foo:bar"), "foo:bar");
        assert_eq!(unit_name_escape("foo.bar"), "foo.bar");
        assert_eq!(unit_name_escape("FOO123"), "FOO123");
    }

    #[test]
    fn test_unescape_basic() {
        assert_eq!(unit_name_unescape("foobar"), Some("foobar".to_string()));
        assert_eq!(
            unit_name_unescape(r"foo\x20bar"),
            Some("foo bar".to_string())
        );
        assert_eq!(unit_name_unescape("foo-bar"), Some("foo/bar".to_string()));
        assert_eq!(unit_name_unescape("-"), Some("/".to_string()));
    }

    #[test]
    fn test_unescape_leading_dot() {
        assert_eq!(
            unit_name_unescape(r"\x2ehidden"),
            Some(".hidden".to_string())
        );
    }

    #[test]
    fn test_roundtrip() {
        // Note: empty string is excluded because escaping "" gives "-"
        // and unescaping "-" gives "/" — this is by design in systemd
        // (the empty string and "/" are both represented as "-").
        let test_cases = &[
            "foo bar",
            "/dev/sda",
            ".hidden",
            "hello/world",
            "foo@bar",
            "a=b&c",
        ];

        for &original in test_cases {
            let escaped = unit_name_escape(original);
            let unescaped = unit_name_unescape(&escaped).unwrap();
            assert_eq!(
                unescaped, original,
                "Roundtrip failed for {:?}: escaped={:?}, unescaped={:?}",
                original, escaped, unescaped
            );
        }
    }

    #[test]
    fn test_escape_empty_string() {
        // Empty string escapes to empty string.
        // The path "-" unescapes to "/".
        assert_eq!(unit_name_escape(""), "");
        assert_eq!(unit_name_unescape("-"), Some("/".to_string()));
    }

    #[test]
    fn test_path_escape() {
        assert_eq!(unit_name_path_escape("/"), "-");
        assert_eq!(unit_name_path_escape("/foo/bar"), "foo-bar");
        assert_eq!(unit_name_path_escape("/foo//bar/"), "foo-bar");
        assert_eq!(unit_name_path_escape("/foo bar/baz"), r"foo\x20bar-baz");
    }

    #[test]
    fn test_path_escape_checked_empty_and_relative() {
        // The empty input and any root-like path escape to "-". A *normalized*
        // path escapes whether absolute or relative (C accepts both, warning
        // separately about non-absolute ones). Only a path that still contains
        // "." / ".." after simplification is rejected (None / C exit 1).
        assert_eq!(unit_name_path_escape_checked(""), Some("-".to_string()));
        assert_eq!(unit_name_path_escape_checked("/"), Some("-".to_string()));
        assert_eq!(unit_name_path_escape_checked("."), None);
        assert_eq!(unit_name_path_escape_checked(".."), None);
        assert_eq!(unit_name_path_escape_checked("/foo/../bar"), None);
        assert_eq!(unit_name_path_escape_checked("foo/../bar"), None);
        // Relative normalized paths are accepted, matching C's `--path foo`.
        assert_eq!(unit_name_path_escape_checked("foo"), Some("foo".to_string()));
        assert_eq!(
            unit_name_path_escape_checked("foo/bar"),
            Some("foo-bar".to_string())
        );
        assert_eq!(unit_name_path_escape_checked("./foo"), Some("foo".to_string()));
        assert_eq!(unit_name_path_escape_checked("foo/"), Some("foo".to_string()));
        assert_eq!(
            unit_name_path_escape_checked("/dev/sda1"),
            Some("dev-sda1".to_string())
        );
    }

    #[test]
    fn test_path_unescape() {
        assert_eq!(unit_name_path_unescape("-"), Some("/".to_string()));
        assert_eq!(
            unit_name_path_unescape("foo-bar"),
            Some("/foo/bar".to_string())
        );
    }

    #[test]
    fn test_path_roundtrip() {
        let paths = &["/", "/foo/bar", "/foo bar/baz", "/dev/sda1"];
        for &path in paths {
            let escaped = unit_name_path_escape(path);
            let unescaped = unit_name_path_unescape(&escaped).unwrap();
            // Normalize the original for comparison
            let normalized = if path == "/" {
                "/".to_string()
            } else {
                format!("/{}", normalize_path(path))
            };
            assert_eq!(
                unescaped, normalized,
                "Path roundtrip failed for {:?}: escaped={:?}, unescaped={:?}",
                path, escaped, unescaped
            );
        }
    }

    #[test]
    fn test_mangle() {
        assert_eq!(unit_name_mangle("foo"), "foo.service");
        assert_eq!(unit_name_mangle("foo.service"), "foo.service");
        assert_eq!(unit_name_mangle("foo.socket"), "foo.socket");
        assert_eq!(unit_name_mangle("foo bar"), r"foo\x20bar.service");
    }

    #[test]
    fn test_mangle_leading_dot() {
        // C's do_escape_mangle keeps a leading '.', unlike the default escape
        // mode. Values verified against systemd-escape --mangle (systemd 260).
        assert_eq!(unit_name_mangle(".hidden"), ".hidden.service");
        assert_eq!(unit_name_mangle("..two"), "..two.service");
        assert_eq!(unit_name_mangle(".a.b"), ".a.b.service");
        assert_eq!(unit_name_mangle(".-x"), ".-x.service");
        // A bare suffix has an empty name, so the suffix is appended.
        assert_eq!(unit_name_mangle(".service"), ".service.service");
        assert_eq!(unit_name_mangle(".mount"), ".mount.service");
    }

    #[test]
    fn test_mangle_path() {
        assert_eq!(unit_name_mangle("/dev/sda"), "dev-sda.device");
        assert_eq!(unit_name_mangle("/foo/bar"), "foo-bar.mount");
    }

    #[test]
    fn test_template_split() {
        assert_eq!(
            unit_name_template_split("foo@bar.service"),
            Some(("foo@", "bar", ".service"))
        );
        assert_eq!(
            unit_name_template_split("foo@.service"),
            Some(("foo@", "", ".service"))
        );
        assert_eq!(unit_name_template_split("foo.service"), None);
    }

    #[test]
    fn test_is_template() {
        assert!(is_template("foo@.service"));
        assert!(!is_template("foo@bar.service"));
        assert!(!is_template("foo.service"));
    }

    #[test]
    fn test_is_instance() {
        assert!(is_instance("foo@bar.service"));
        assert!(!is_instance("foo@.service"));
        assert!(!is_instance("foo.service"));
    }

    #[test]
    fn test_template_instantiate() {
        assert_eq!(
            template_instantiate("foo@.service", "bar"),
            Some("foo@bar.service".to_string())
        );
        assert_eq!(
            template_instantiate("getty@.service", "tty1"),
            Some("getty@tty1.service".to_string())
        );
        assert_eq!(template_instantiate("foo.service", "bar"), None);
    }

    #[test]
    fn test_unescape_invalid() {
        // Incomplete escape
        assert_eq!(unit_name_unescape(r"\x2"), None);
        assert_eq!(unit_name_unescape(r"\x"), None);
        assert_eq!(unit_name_unescape(r"\"), None);
        // Invalid hex
        assert_eq!(unit_name_unescape(r"\xzz"), None);
    }

    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path("/"), "");
        assert_eq!(normalize_path("///"), "");
        assert_eq!(normalize_path("/foo/bar"), "foo/bar");
        assert_eq!(normalize_path("/foo//bar/"), "foo/bar");
        assert_eq!(normalize_path("/foo///bar///baz/"), "foo/bar/baz");
        // Leading `..` in absolute paths are skipped
        assert_eq!(normalize_path("/.."), "");
        assert_eq!(normalize_path("/../.././../.././"), "");
        assert_eq!(normalize_path("/../.././../.././foo"), "foo");
        // `..` after a real component is kept (making the path invalid)
        assert_eq!(normalize_path("/../hello/.."), "hello/..");
    }

    #[test]
    fn fuzz_unit_name_functions_never_panic() {
        // Unit names and the paths they escape come from untrusted sources (the
        // filesystem, D-Bus, `systemd-escape`): a malformed `\xNN` run, a bare
        // backslash, stray `@`/`-`/`.`, non-ASCII bytes, or an over-long input
        // must never panic any of the escape/unescape/mangle/template helpers.
        const TOKENS: &[&str] = &[
            "\\", "\\x", "\\x2", "\\x2f", "\\xzz", "\\xGG", "-", "--", "@", "@.",
            ".", "..", "/", "//", "foo", "a-b", "a@b", "@.service", "getty@.service",
            ".mount", "foo.service", "\\x00", "%i", " ", "\t", "€", ":", "~", "0",
        ];
        let handle = std::thread::spawn(move || {
            let mut state: u64 = 0x1234_5678_9abc_def0;
            let mut next = || {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 33) as u32
            };
            for _ in 0..80_000u32 {
                let mut s = String::new();
                for _ in 0..(next() % 24) {
                    if next() % 4 == 0 {
                        s.push(char::from_u32(next() % 0x120).unwrap_or('?'));
                    } else {
                        s.push_str(TOKENS[(next() as usize) % TOKENS.len()]);
                    }
                }
                let inst = TOKENS[(next() as usize) % TOKENS.len()];
                let tmpl = ["getty@.service", "a@.mount", "@.service", "x", ""]
                    [(next() as usize) % 5];
                let input = s.clone();
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = unit_name_escape(&s);
                    let _ = unit_name_unescape(&s);
                    let _ = unit_name_path_escape(&s);
                    let _ = unit_name_path_escape_checked(&s);
                    let _ = unit_name_path_unescape(&s);
                    let _ = unit_name_mangle(&s);
                    let _ = unit_name_template_split(&s);
                    let _ = is_template(&s);
                    let _ = is_instance(&s);
                    let _ = template_instantiate(tmpl, inst);
                    // Round-trip: escaping then unescaping must not panic either.
                    let _ = unit_name_unescape(&unit_name_escape(&s));
                }));
                assert!(res.is_ok(), "unit_name helper panicked on: {input:?}");
            }
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !handle.is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "unit_name fuzz did not finish in 30s -- an input hangs a helper"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        handle.join().expect("unit_name fuzz thread panicked");
    }
}
