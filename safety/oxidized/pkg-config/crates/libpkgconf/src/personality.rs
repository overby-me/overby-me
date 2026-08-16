//! Cross-compilation personality support.
//!
//! This module implements pkgconf's "personality" system, which allows
//! configuring pkg-config behavior for cross-compilation targets. A
//! personality defines target-specific search paths, system directories,
//! and default flags.
//!
//! Personalities can be loaded from:
//! - Personality files (INI-like format) in system personality directories
//! - The `--personality` CLI flag
//! - Deduction from `argv[0]` (e.g. `x86_64-linux-gnu-pkg-config`)
//!
//! # Personality File Format
//!
//! Personality files use a simple INI-like format with `key=value` pairs:
//!
//! ```text
//! Triplet: x86_64-linux-gnu
//! SysrootDir: /usr/x86_64-linux-gnu
//! DefaultSearchPaths: /usr/x86_64-linux-gnu/lib/pkgconfig:/usr/x86_64-linux-gnu/share/pkgconfig
//! SystemIncludePaths: /usr/x86_64-linux-gnu/include
//! SystemLibraryPaths: /usr/x86_64-linux-gnu/lib
//! WantDefaultStatic: false
//! WantDefaultPure: false
//! ```

use std::collections::HashMap;
use std::path::Path;

use crate::path::SearchPath;

/// Default personality directory paths where `.personality` files are searched.
#[cfg(unix)]
pub const DEFAULT_PERSONALITY_DIRS: &[&str] = &[
    "/usr/share/pkgconfig/personality.d",
    "/usr/local/share/pkgconfig/personality.d",
    "/etc/pkgconfig/personality.d",
];

#[cfg(windows)]
pub const DEFAULT_PERSONALITY_DIRS: &[&str] = &[];

/// A cross-compilation personality defining target-specific configuration.
///
/// This struct mirrors pkgconf's `pkgconf_cross_personality_t` and holds
/// all the configuration that varies between native and cross-compilation
/// targets.
#[derive(Debug, Clone)]
pub struct CrossPersonality {
    /// The target triplet (e.g. `x86_64-linux-gnu`).
    pub name: String,

    /// Search paths for `.pc` files specific to this personality.
    pub dir_list: SearchPath,

    /// System library directories to filter from `-L` output.
    pub filter_libdirs: SearchPath,

    /// System include directories to filter from `-I` output.
    pub filter_includedirs: SearchPath,

    /// Sysroot directory for this personality.
    pub sysroot_dir: Option<String>,

    /// Whether to default to static linking for this personality.
    pub want_default_static: bool,

    /// Whether to default to pure dependency graph mode.
    pub want_default_pure: bool,
}

impl CrossPersonality {
    /// Create the default (native) personality using system defaults.
    ///
    /// This uses the compile-time default paths for the host system.
    pub fn default_personality() -> Self {
        let mut dir_list = SearchPath::new();
        #[cfg(unix)]
        for p in crate::DEFAULT_PKGCONFIG_PATH {
            dir_list.add(*p);
        }
        #[cfg(windows)]
        for p in crate::DEFAULT_PKGCONFIG_PATH {
            dir_list.add(*p);
        }

        // On macOS, add Homebrew paths based on architecture or the
        // HOMEBREW_PREFIX environment variable so that packages installed
        // via `brew` are discovered automatically.
        #[cfg(target_os = "macos")]
        {
            // Prefer the environment variable when available.
            let homebrew_prefix = std::env::var("HOMEBREW_PREFIX").ok();
            let prefix = homebrew_prefix.as_deref().unwrap_or_else(|| {
                if cfg!(target_arch = "aarch64") {
                    crate::HOMEBREW_PREFIX_ARM64
                } else {
                    crate::HOMEBREW_PREFIX_X86_64
                }
            });
            let lib_path = format!("{prefix}/lib/pkgconfig");
            let share_path = format!("{prefix}/share/pkgconfig");
            dir_list.add(&lib_path);
            dir_list.add(&share_path);
        }

        let mut filter_libdirs = SearchPath::new();
        #[cfg(unix)]
        for p in crate::DEFAULT_SYSTEM_LIBDIRS {
            filter_libdirs.add(*p);
        }
        #[cfg(windows)]
        for p in crate::DEFAULT_SYSTEM_LIBDIRS {
            filter_libdirs.add(*p);
        }

        let mut filter_includedirs = SearchPath::new();
        #[cfg(unix)]
        for p in crate::DEFAULT_SYSTEM_INCLUDEDIRS {
            filter_includedirs.add(*p);
        }
        #[cfg(windows)]
        for p in crate::DEFAULT_SYSTEM_INCLUDEDIRS {
            filter_includedirs.add(*p);
        }

        Self {
            name: "default".to_string(),
            dir_list,
            filter_libdirs,
            filter_includedirs,
            sysroot_dir: None,
            want_default_static: false,
            want_default_pure: false,
        }
    }

    /// Create a new empty personality with the given triplet name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            dir_list: SearchPath::new(),
            filter_libdirs: SearchPath::new(),
            filter_includedirs: SearchPath::new(),
            sysroot_dir: None,
            want_default_static: false,
            want_default_pure: false,
        }
    }

    /// Find and load a personality by triplet name.
    ///
    /// Searches the default personality directories for a file named
    /// `{triplet}.personality` and parses it.
    ///
    /// Returns `None` if no personality file is found.
    pub fn find(triplet: &str) -> Option<Self> {
        Self::find_in_dirs(triplet, DEFAULT_PERSONALITY_DIRS)
    }

    /// Find and load a personality by triplet name, searching the given directories.
    pub fn find_in_dirs(triplet: &str, dirs: &[&str]) -> Option<Self> {
        let filename = format!("{triplet}.personality");

        for dir in dirs {
            let path = Path::new(dir).join(&filename);
            if path.is_file() {
                return Self::from_file(&path).ok();
            }
        }

        None
    }

    /// Load a personality from a file path.
    pub fn from_file(path: &Path) -> Result<Self, std::io::Error> {
        let content = std::fs::read_to_string(path)?;
        let mut personality = Self::parse(&content);

        // If no name was set, derive from filename
        if (personality.name.is_empty() || personality.name == "default")
            && let Some(stem) = path.file_stem()
        {
            let stem_str = stem.to_string_lossy();
            // Strip .personality extension if present in stem
            let name = stem_str.strip_suffix(".personality").unwrap_or(&stem_str);
            personality.name = name.to_string();
        }

        Ok(personality)
    }

    /// Parse a personality from its text content.
    ///
    /// The format is simple `Key: Value` pairs, one per line.
    /// Lines starting with `#` are comments. Blank lines are ignored.
    pub fn parse(content: &str) -> Self {
        let mut fields = HashMap::new();

        for line in content.lines() {
            let line = line.trim();

            // Skip comments and blank lines
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Split on the first `:` or `=`
            let (key, value) = if let Some(pos) = line.find(':') {
                let eq_pos = line.find('=');
                // Use whichever delimiter comes first
                let split_pos = match eq_pos {
                    Some(ep) if ep < pos => ep,
                    _ => pos,
                };
                let (k, v) = line.split_at(split_pos);
                (k.trim(), v[1..].trim())
            } else if let Some(pos) = line.find('=') {
                let (k, v) = line.split_at(pos);
                (k.trim(), v[1..].trim())
            } else {
                continue;
            };

            fields.insert(key.to_lowercase(), value.to_string());
        }

        let name = fields
            .get("triplet")
            .cloned()
            .unwrap_or_else(|| "default".to_string());

        let dir_list = fields
            .get("defaultsearchpaths")
            .map(|s| SearchPath::from_delimited(s, crate::path::PATH_SEPARATOR))
            .unwrap_or_default();

        let filter_includedirs = fields
            .get("systemincludepaths")
            .map(|s| SearchPath::from_delimited(s, crate::path::PATH_SEPARATOR))
            .unwrap_or_default();

        let filter_libdirs = fields
            .get("systemlibrarypaths")
            .map(|s| SearchPath::from_delimited(s, crate::path::PATH_SEPARATOR))
            .unwrap_or_default();

        let sysroot_dir = fields.get("sysrootdir").filter(|s| !s.is_empty()).cloned();

        let want_default_static = fields
            .get("wantdefaultstatic")
            .map(|s| parse_bool(s))
            .unwrap_or(false);

        let want_default_pure = fields
            .get("wantdefaultpure")
            .map(|s| parse_bool(s))
            .unwrap_or(false);

        Self {
            name,
            dir_list,
            filter_libdirs,
            filter_includedirs,
            sysroot_dir,
            want_default_static,
            want_default_pure,
        }
    }

    /// Attempt to deduce a personality from the program name (argv[0]).
    ///
    /// If the binary was invoked as `x86_64-linux-gnu-pkg-config`, this
    /// extracts the triplet `x86_64-linux-gnu` and tries to find a matching
    /// personality.
    ///
    /// Returns `None` if no triplet could be extracted or no personality
    /// file was found.
    pub fn from_argv0(argv0: &str) -> Option<Self> {
        let triplet = deduce_triplet(argv0)?;
        Self::find(&triplet)
    }

    /// Format this personality for `--dump-personality` output.
    pub fn dump(&self) -> String {
        let mut out = String::new();

        out.push_str(&format!("Triplet: {}\n", self.name));

        if let Some(ref sysroot) = self.sysroot_dir {
            out.push_str(&format!("SysrootDir: {sysroot}\n"));
        }

        out.push_str(&format!(
            "DefaultSearchPaths: {}\n",
            self.dir_list.to_delimited(crate::path::PATH_SEPARATOR)
        ));

        out.push_str(&format!(
            "SystemIncludePaths: {}\n",
            self.filter_includedirs
                .to_delimited(crate::path::PATH_SEPARATOR)
        ));

        out.push_str(&format!(
            "SystemLibraryPaths: {}\n",
            self.filter_libdirs
                .to_delimited(crate::path::PATH_SEPARATOR)
        ));

        out.push_str(&format!(
            "WantDefaultStatic: {}\n",
            if self.want_default_static {
                "true"
            } else {
                "false"
            }
        ));

        out.push_str(&format!(
            "WantDefaultPure: {}\n",
            if self.want_default_pure {
                "true"
            } else {
                "false"
            }
        ));

        out
    }
}

impl Default for CrossPersonality {
    fn default() -> Self {
        Self::default_personality()
    }
}

impl std::fmt::Display for CrossPersonality {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.dump())
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Parse a boolean value from a personality file.
///
/// Accepts `true`, `yes`, `1` (case-insensitive) as true; everything
/// else is false.
fn parse_bool(s: &str) -> bool {
    matches!(s.to_lowercase().as_str(), "true" | "yes" | "1")
}

/// Deduce a target triplet from the program name.
///
/// Given something like `/usr/bin/x86_64-linux-gnu-pkg-config` or
/// `aarch64-unknown-linux-gnu-pkgconf`, this extracts the triplet
/// portion before the `-pkg-config` or `-pkgconf` suffix.
///
/// Returns `None` if the program name doesn't contain a recognizable
/// suffix or has no triplet prefix.
pub fn deduce_triplet(argv0: &str) -> Option<String> {
    // Get the basename (last path component)
    let basename = Path::new(argv0).file_name()?.to_str()?;

    // Known suffixes that indicate a cross-compilation invocation
    let suffixes = ["-pkg-config", "-pkgconf"];

    for suffix in &suffixes {
        if let Some(triplet) = basename.strip_suffix(suffix)
            && !triplet.is_empty()
            && triplet.contains('-')
        {
            return Some(triplet.to_string());
        }
    }

    None
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_personality_has_system_paths() {
        let p = CrossPersonality::default_personality();
        assert_eq!(p.name, "default");
        assert!(!p.want_default_static);
        assert!(!p.want_default_pure);
        assert!(p.sysroot_dir.is_none());

        #[cfg(unix)]
        {
            assert!(!p.dir_list.is_empty());
            assert!(!p.filter_libdirs.is_empty());
            assert!(!p.filter_includedirs.is_empty());
        }
    }

    #[test]
    fn new_personality_is_empty() {
        let p = CrossPersonality::new("arm-linux-gnueabihf");
        assert_eq!(p.name, "arm-linux-gnueabihf");
        assert!(p.dir_list.is_empty());
        assert!(p.filter_libdirs.is_empty());
        assert!(p.filter_includedirs.is_empty());
        assert!(p.sysroot_dir.is_none());
        assert!(!p.want_default_static);
        assert!(!p.want_default_pure);
    }

    #[test]
    fn parse_full_personality() {
        let content = r#"
# Cross-compilation personality for x86_64-linux-gnu
Triplet: x86_64-linux-gnu
SysrootDir: /usr/x86_64-linux-gnu
DefaultSearchPaths: /usr/x86_64-linux-gnu/lib/pkgconfig:/usr/x86_64-linux-gnu/share/pkgconfig
SystemIncludePaths: /usr/x86_64-linux-gnu/include
SystemLibraryPaths: /usr/x86_64-linux-gnu/lib
WantDefaultStatic: false
WantDefaultPure: true
"#;

        let p = CrossPersonality::parse(content);
        assert_eq!(p.name, "x86_64-linux-gnu");
        assert_eq!(p.sysroot_dir.as_deref(), Some("/usr/x86_64-linux-gnu"));
        assert_eq!(p.dir_list.len(), 2);
        assert_eq!(p.filter_includedirs.len(), 1);
        assert_eq!(p.filter_libdirs.len(), 1);
        assert!(!p.want_default_static);
        assert!(p.want_default_pure);
    }

    #[test]
    fn parse_minimal_personality() {
        let content = "Triplet: aarch64-linux-gnu\n";
        let p = CrossPersonality::parse(content);
        assert_eq!(p.name, "aarch64-linux-gnu");
        assert!(p.dir_list.is_empty());
        assert!(p.sysroot_dir.is_none());
    }

    #[test]
    fn parse_with_equals_delimiter() {
        let content = r#"
Triplet=riscv64-linux-gnu
SysrootDir=/opt/riscv
DefaultSearchPaths=/opt/riscv/lib/pkgconfig
WantDefaultStatic=true
"#;

        let p = CrossPersonality::parse(content);
        assert_eq!(p.name, "riscv64-linux-gnu");
        assert_eq!(p.sysroot_dir.as_deref(), Some("/opt/riscv"));
        assert_eq!(p.dir_list.len(), 1);
        assert!(p.want_default_static);
    }

    #[test]
    fn parse_empty_content() {
        let p = CrossPersonality::parse("");
        assert_eq!(p.name, "default");
        assert!(p.dir_list.is_empty());
    }

    #[test]
    fn parse_comments_and_blanks() {
        let content = r#"
# This is a comment
# Another comment

Triplet: test-triplet

# Trailing comment
"#;

        let p = CrossPersonality::parse(content);
        assert_eq!(p.name, "test-triplet");
    }

    #[test]
    fn parse_bool_values() {
        assert!(parse_bool("true"));
        assert!(parse_bool("True"));
        assert!(parse_bool("TRUE"));
        assert!(parse_bool("yes"));
        assert!(parse_bool("Yes"));
        assert!(parse_bool("1"));
        assert!(!parse_bool("false"));
        assert!(!parse_bool("False"));
        assert!(!parse_bool("no"));
        assert!(!parse_bool("0"));
        assert!(!parse_bool(""));
        assert!(!parse_bool("anything"));
    }

    #[test]
    fn parse_want_default_static_true() {
        let content = "Triplet: test\nWantDefaultStatic: yes\n";
        let p = CrossPersonality::parse(content);
        assert!(p.want_default_static);
    }

    #[test]
    fn parse_want_default_pure_true() {
        let content = "Triplet: test\nWantDefaultPure: 1\n";
        let p = CrossPersonality::parse(content);
        assert!(p.want_default_pure);
    }

    #[test]
    fn deduce_triplet_pkg_config() {
        assert_eq!(
            deduce_triplet("x86_64-linux-gnu-pkg-config"),
            Some("x86_64-linux-gnu".to_string())
        );
    }

    #[test]
    fn deduce_triplet_pkgconf() {
        assert_eq!(
            deduce_triplet("aarch64-unknown-linux-gnu-pkgconf"),
            Some("aarch64-unknown-linux-gnu".to_string())
        );
    }

    #[test]
    fn deduce_triplet_with_path() {
        assert_eq!(
            deduce_triplet("/usr/bin/arm-linux-gnueabihf-pkg-config"),
            Some("arm-linux-gnueabihf".to_string())
        );
    }

    #[test]
    fn deduce_triplet_no_triplet() {
        assert_eq!(deduce_triplet("pkg-config"), None);
        assert_eq!(deduce_triplet("pkgconf"), None);
    }

    #[test]
    fn deduce_triplet_no_suffix() {
        assert_eq!(deduce_triplet("some-random-binary"), None);
    }

    #[test]
    fn deduce_triplet_empty() {
        assert_eq!(deduce_triplet(""), None);
    }

    #[test]
    fn deduce_triplet_no_dash_before_suffix() {
        // "foo-pkg-config" → "foo" has no dash, but it's still a valid
        // (though unusual) single-component prefix. Our implementation
        // requires at least one dash in the triplet.
        assert_eq!(deduce_triplet("foo-pkg-config"), None);
    }

    #[test]
    fn dump_full_personality() {
        let p = CrossPersonality {
            name: "x86_64-linux-gnu".to_string(),
            dir_list: SearchPath::from_delimited("/usr/x86_64-linux-gnu/lib/pkgconfig", ':'),
            filter_libdirs: SearchPath::from_delimited("/usr/x86_64-linux-gnu/lib", ':'),
            filter_includedirs: SearchPath::from_delimited("/usr/x86_64-linux-gnu/include", ':'),
            sysroot_dir: Some("/usr/x86_64-linux-gnu".to_string()),
            want_default_static: false,
            want_default_pure: false,
        };

        let dumped = p.dump();
        assert!(dumped.contains("Triplet: x86_64-linux-gnu"));
        assert!(dumped.contains("SysrootDir: /usr/x86_64-linux-gnu"));
        assert!(dumped.contains("DefaultSearchPaths:"));
        assert!(dumped.contains("SystemIncludePaths:"));
        assert!(dumped.contains("SystemLibraryPaths:"));
        assert!(dumped.contains("WantDefaultStatic: false"));
        assert!(dumped.contains("WantDefaultPure: false"));
    }

    #[test]
    fn dump_no_sysroot() {
        let p = CrossPersonality::new("test");
        let dumped = p.dump();
        assert!(!dumped.contains("SysrootDir:"));
    }

    #[test]
    fn display_trait() {
        let p = CrossPersonality::new("test-triplet");
        let s = format!("{p}");
        assert!(s.contains("Triplet: test-triplet"));
    }

    #[test]
    fn default_trait() {
        let p = CrossPersonality::default();
        assert_eq!(p.name, "default");
    }

    #[test]
    fn roundtrip_parse_dump() {
        let content = r#"Triplet: mips-linux-gnu
SysrootDir: /opt/mips
DefaultSearchPaths: /opt/mips/lib/pkgconfig:/opt/mips/share/pkgconfig
SystemIncludePaths: /opt/mips/include
SystemLibraryPaths: /opt/mips/lib
WantDefaultStatic: true
WantDefaultPure: false
"#;

        let parsed = CrossPersonality::parse(content);
        assert_eq!(parsed.name, "mips-linux-gnu");
        assert_eq!(parsed.sysroot_dir.as_deref(), Some("/opt/mips"));
        assert!(parsed.want_default_static);
        assert!(!parsed.want_default_pure);
        assert_eq!(parsed.dir_list.len(), 2);

        // Dump and re-parse should preserve semantics
        let dumped = parsed.dump();
        let reparsed = CrossPersonality::parse(&dumped);
        assert_eq!(reparsed.name, parsed.name);
        assert_eq!(reparsed.sysroot_dir, parsed.sysroot_dir);
        assert_eq!(reparsed.want_default_static, parsed.want_default_static);
        assert_eq!(reparsed.want_default_pure, parsed.want_default_pure);
        assert_eq!(reparsed.dir_list.len(), parsed.dir_list.len());
        assert_eq!(reparsed.filter_libdirs.len(), parsed.filter_libdirs.len());
        assert_eq!(
            reparsed.filter_includedirs.len(),
            parsed.filter_includedirs.len()
        );
    }

    #[test]
    fn find_in_nonexistent_dirs() {
        let result = CrossPersonality::find_in_dirs("nonexistent", &["/tmp/nonexistent-dir"]);
        assert!(result.is_none());
    }

    #[test]
    fn case_insensitive_keys() {
        let content = r#"
triplet: lowercase-test
sysrootdir: /opt/test
defaultsearchpaths: /opt/test/lib/pkgconfig
systemincludepaths: /opt/test/include
systemlibrarypaths: /opt/test/lib
wantdefaultstatic: true
wantdefaultpure: yes
"#;

        let p = CrossPersonality::parse(content);
        assert_eq!(p.name, "lowercase-test");
        assert_eq!(p.sysroot_dir.as_deref(), Some("/opt/test"));
        assert_eq!(p.dir_list.len(), 1);
        assert_eq!(p.filter_includedirs.len(), 1);
        assert_eq!(p.filter_libdirs.len(), 1);
        assert!(p.want_default_static);
        assert!(p.want_default_pure);
    }

    #[test]
    fn multiple_search_paths() {
        let content = "Triplet: multi\nDefaultSearchPaths: /a:/b:/c:/d\n";
        let p = CrossPersonality::parse(content);
        assert_eq!(p.dir_list.len(), 4);
    }

    #[test]
    fn empty_sysroot_is_none() {
        let content = "Triplet: test\nSysrootDir: \n";
        let p = CrossPersonality::parse(content);
        assert!(p.sysroot_dir.is_none());
    }

    #[test]
    fn argv0_with_nonexistent_personality() {
        // from_argv0 should return None when no personality file exists
        let result = CrossPersonality::from_argv0("x86_64-test-nonexistent-pkg-config");
        assert!(result.is_none());
    }
}
