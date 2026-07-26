//! The different parts of unit handling: parsing and activating

pub(crate) mod from_parsed_config;
mod id;
pub(crate) mod loading;
mod mount_monitor;
mod status;
mod udev_event;
mod unit;
pub(crate) mod unit_parsing;
mod unitset_manipulation;

pub use id::*;
pub use loading::*;
pub use mount_monitor::*;
pub use status::*;
pub use udev_event::*;
pub use unit::*;
pub use unit_parsing::*;
pub use unitset_manipulation::*;

/// Split one `ExecDirectory=` entry into `(source, destination, read_only)`.
///
/// The syntax is `source[:destination[:access-mode]]`, where a colon inside a
/// path is escaped as `\:` (systemd.exec(5)). Only the first two unescaped
/// colons separate fields; anything after them belongs to the access-mode
/// field, which may itself be colon- or comma-separated.
///
/// Shared between the exec helper (which creates the directories) and anything
/// else that has to agree with it on what a given entry names.
pub fn parse_exec_dir_entry(entry: &str) -> (String, Option<String>, bool) {
    let mut fields: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = entry.chars();
    while let Some(c) = chars.next() {
        match c {
            // `\:` and `\\` are literals; any other backslash is kept as-is so
            // a Windows-ish path or a stray backslash is not silently eaten.
            '\\' => match chars.next() {
                Some(escaped @ (':' | '\\')) => cur.push(escaped),
                Some(other) => {
                    cur.push('\\');
                    cur.push(other);
                }
                None => cur.push('\\'),
            },
            ':' if fields.len() < 2 => fields.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    fields.push(cur);

    let src = fields.first().cloned().unwrap_or_default();
    let dest = fields.get(1).filter(|s| !s.is_empty()).cloned();
    let read_only = fields
        .get(2)
        .is_some_and(|flags| flags.split([':', ',']).any(|f| f == "ro"));
    (src, dest, read_only)
}

#[cfg(test)]
mod exec_dir_entry_tests {
    use super::parse_exec_dir_entry;

    #[test]
    fn plain_source_only() {
        assert_eq!(parse_exec_dir_entry("zzz"), ("zzz".into(), None, false));
    }

    #[test]
    fn source_and_destination_alias() {
        assert_eq!(
            parse_exec_dir_entry("zzz:yyy"),
            ("zzz".into(), Some("yyy".into()), false)
        );
    }

    #[test]
    fn empty_destination_is_none_and_access_mode_still_parses() {
        // `www::ro` means "no alias, read-only", not "alias named empty".
        assert_eq!(
            parse_exec_dir_entry("www::ro"),
            ("www".into(), None, true)
        );
    }

    #[test]
    fn destination_plus_read_only() {
        assert_eq!(
            parse_exec_dir_entry("www:ro:ro"),
            ("www".into(), Some("ro".into()), true)
        );
    }

    #[test]
    fn escaped_colon_stays_inside_the_name() {
        // TEST-34-DYNAMICUSERMIGRATE uses `StateDirectory=zzz:x\:yz`, which
        // means an alias literally named `x:yz`, not a third field.
        assert_eq!(
            parse_exec_dir_entry(r"zzz:x\:yz"),
            ("zzz".into(), Some("x:yz".into()), false)
        );
    }

    #[test]
    fn escaped_colon_in_the_source_too() {
        assert_eq!(
            parse_exec_dir_entry(r"a\:b:c\:d"),
            ("a:b".into(), Some("c:d".into()), false)
        );
    }

    #[test]
    fn escaped_backslash_is_literal() {
        assert_eq!(
            parse_exec_dir_entry(r"a\\b"),
            (r"a\b".into(), None, false)
        );
    }

    #[test]
    fn only_the_first_two_colons_split() {
        // A third colon belongs to the access-mode field.
        assert_eq!(
            parse_exec_dir_entry("a:b:ro:extra"),
            ("a".into(), Some("b".into()), true)
        );
    }
}
