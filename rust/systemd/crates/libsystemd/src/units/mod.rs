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

/// One `ExecDirectory=` entry, parsed and ordered for creation.
pub struct ExecDirEntry {
    /// Source directory name, relative to the type's base (e.g. `aaa/bbb`).
    pub src: String,
    /// Optional symlink alias.
    pub dest: Option<String>,
    /// `:ro` access mode.
    pub read_only: bool,
    /// A parent of this entry is also configured, so its own symlink must NOT
    /// be created: the parent's symlink already covers this path, and the two
    /// resolve to the same inode (upstream `EXEC_DIRECTORY_ONLY_CREATE`, added
    /// for systemd issue #24783).
    pub only_create: bool,
}

/// Parse and order `ExecDirectory=` entries the way upstream's
/// `exec_directory_sort` does.
///
/// Parents must be created before their children, even when the unit lists them
/// the other way round (`StateDirectory=foo/bar foo`): otherwise the
/// intermediate `foo` is created as a plain directory on the way to `foo/bar`,
/// and the later `foo` entry can no longer become the symlink it needs to be.
///
/// Sorting by the source path is enough to get parents first, because `/` sorts
/// after the end of a string: `aaa` < `aaa/bbb` < `aaa/ccc`.
pub fn sorted_exec_dir_entries(entries: &[String]) -> Vec<ExecDirEntry> {
    let mut parsed: Vec<ExecDirEntry> = entries
        .iter()
        .map(|e| {
            let (src, dest, read_only) = parse_exec_dir_entry(e);
            ExecDirEntry {
                src,
                dest,
                read_only,
                only_create: false,
            }
        })
        .collect();
    parsed.sort_by(|a, b| a.src.cmp(&b.src));

    for i in 0..parsed.len() {
        let is_child = parsed[..i]
            .iter()
            .any(|p| parsed[i].src.starts_with(&format!("{}/", p.src)));
        parsed[i].only_create = is_child;
    }
    parsed
}

#[cfg(test)]
mod exec_dir_sort_tests {
    use super::sorted_exec_dir_entries;

    fn names(v: &[super::ExecDirEntry]) -> Vec<&str> {
        v.iter().map(|e| e.src.as_str()).collect()
    }

    #[test]
    fn parents_are_ordered_before_their_children() {
        let e = sorted_exec_dir_entries(&["foo/bar".into(), "foo".into()]);
        assert_eq!(names(&e), vec!["foo", "foo/bar"]);
    }

    #[test]
    fn a_child_is_flagged_only_create_and_its_parent_is_not() {
        let e = sorted_exec_dir_entries(&["aaa/bbb".into(), "aaa".into()]);
        assert!(!e[0].only_create, "aaa owns its symlink");
        assert!(e[1].only_create, "aaa/bbb is covered by aaa's symlink");
    }

    #[test]
    fn an_unrelated_prefix_is_not_treated_as_a_parent() {
        // `aa` is a string prefix of `aaa` but not a path parent.
        let e = sorted_exec_dir_entries(&["aa".into(), "aaa".into()]);
        assert!(e.iter().all(|x| !x.only_create));
    }

    #[test]
    fn the_full_upstream_case_orders_and_flags_correctly() {
        let e = sorted_exec_dir_entries(&[
            "waldo".into(),
            "quux/pief".into(),
            "aaa/bbb".into(),
            "aaa".into(),
            "aaa/ccc".into(),
        ]);
        assert_eq!(
            names(&e),
            vec!["aaa", "aaa/bbb", "aaa/ccc", "quux/pief", "waldo"]
        );
        // Only the two children of the configured `aaa` are ONLY_CREATE.
        // `quux/pief` is nested but `quux` is not itself configured.
        let flagged: Vec<&str> = e
            .iter()
            .filter(|x| x.only_create)
            .map(|x| x.src.as_str())
            .collect();
        assert_eq!(flagged, vec!["aaa/bbb", "aaa/ccc"]);
    }

    #[test]
    fn aliases_and_access_mode_survive_the_sort() {
        let e = sorted_exec_dir_entries(&["xxx/yyy:aaa/111".into(), "www::ro".into()]);
        assert_eq!(names(&e), vec!["www", "xxx/yyy"]);
        assert!(e[0].read_only);
        assert_eq!(e[1].dest.as_deref(), Some("aaa/111"));
    }
}
