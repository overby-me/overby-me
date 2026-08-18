//! Search path management for `.pc` file discovery.
//!
//! This module provides the [`SearchPath`] type for managing ordered lists of
//! directories to search when resolving package names to `.pc` files.
//!
//! Search paths can be constructed from:
//! - Environment variables (`PKG_CONFIG_PATH`, `PKG_CONFIG_LIBDIR`)
//! - Command-line arguments (`--with-path`)
//! - Default system paths
//! - Colon-delimited (or semicolon-delimited on Windows) strings
//!
//! The module handles path normalization, deduplication, and matching against
//! system directory filter lists.

use std::path::{Path, PathBuf};

/// The path separator used in environment variables like `PKG_CONFIG_PATH`.
///
/// On Unix systems this is `:`, on Windows it is `;`.
#[cfg(unix)]
pub const PATH_SEPARATOR: char = ':';

#[cfg(windows)]
pub const PATH_SEPARATOR: char = ';';

/// An ordered list of directories to search for `.pc` files.
///
/// Directories are searched in order, and the first match wins. The list
/// supports prepending (for higher-priority paths like `--with-path`),
/// appending (for defaults), and filtering against system directory lists.
///
/// # Examples
///
/// ```
/// use libpkgconf::path::SearchPath;
///
/// let mut sp = SearchPath::new();
/// sp.add("/usr/lib/pkgconfig");
/// sp.add("/usr/share/pkgconfig");
/// assert_eq!(sp.len(), 2);
///
/// // Parse from a colon-delimited string
/// let sp2 = SearchPath::from_delimited("/opt/lib/pkgconfig:/opt/share/pkgconfig", ':');
/// assert_eq!(sp2.len(), 2);
/// ```
#[derive(Debug, Clone, Default)]
pub struct SearchPath {
    dirs: Vec<PathBuf>,
}

impl SearchPath {
    /// Create an empty search path.
    pub fn new() -> Self {
        Self { dirs: Vec::new() }
    }

    /// Create a search path from a slice of path strings.
    pub fn from_paths(paths: &[&str]) -> Self {
        let dirs = paths.iter().map(PathBuf::from).collect();
        Self { dirs }
    }

    /// Parse a search path from a delimited string (e.g. colon-separated).
    ///
    /// Empty segments are silently skipped.
    ///
    /// # Examples
    ///
    /// ```
    /// use libpkgconf::path::SearchPath;
    ///
    /// let sp = SearchPath::from_delimited("/a:/b:/c", ':');
    /// assert_eq!(sp.len(), 3);
    ///
    /// // Empty segments are skipped
    /// let sp = SearchPath::from_delimited("/a::/b:", ':');
    /// assert_eq!(sp.len(), 2);
    /// ```
    pub fn from_delimited(s: &str, separator: char) -> Self {
        let dirs = s
            .split(separator)
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .collect();
        Self { dirs }
    }

    /// Parse a search path from an environment-variable-style string.
    ///
    /// Uses the platform-appropriate separator (`:` on Unix, `;` on Windows).
    pub fn from_env_value(s: &str) -> Self {
        Self::from_delimited(s, PATH_SEPARATOR)
    }

    /// Build a search path from an environment variable, if it is set.
    ///
    /// Returns `None` if the variable is not set. Returns an empty `SearchPath`
    /// if the variable is set but empty.
    pub fn from_environ(var_name: &str) -> Option<Self> {
        std::env::var(var_name)
            .ok()
            .map(|v| Self::from_env_value(&v))
    }

    /// Add a directory to the end of the search path.
    ///
    /// The path is normalized before insertion.
    pub fn add<P: Into<PathBuf>>(&mut self, path: P) {
        let path = path.into();
        if !path.as_os_str().is_empty() {
            self.dirs.push(normalize_path(&path));
        }
    }

    /// Add a directory to the end of the search path (from a string reference).
    pub fn add_str(&mut self, path: &str) {
        if !path.is_empty() {
            self.add(PathBuf::from(path));
        }
    }

    /// Prepend a directory to the beginning of the search path.
    ///
    /// The path is normalized before insertion.
    pub fn prepend<P: Into<PathBuf>>(&mut self, path: P) {
        let path = path.into();
        if !path.as_os_str().is_empty() {
            self.dirs.insert(0, normalize_path(&path));
        }
    }

    /// Prepend a directory from a string reference.
    pub fn prepend_str(&mut self, path: &str) {
        if !path.is_empty() {
            self.prepend(PathBuf::from(path));
        }
    }

    /// Split a delimited string and add all resulting paths.
    pub fn add_delimited(&mut self, s: &str, separator: char) {
        for segment in s.split(separator) {
            if !segment.is_empty() {
                self.add(PathBuf::from(segment));
            }
        }
    }

    /// Split a delimited string and prepend all resulting paths (in order).
    ///
    /// The first segment in the string becomes the first entry in the path,
    /// preserving the relative ordering.
    pub fn prepend_delimited(&mut self, s: &str, separator: char) {
        let new_paths: Vec<PathBuf> = s
            .split(separator)
            .filter(|p| !p.is_empty())
            .map(|p| normalize_path(&PathBuf::from(p)))
            .collect();
        let mut merged = new_paths;
        merged.append(&mut self.dirs);
        self.dirs = merged;
    }

    /// Append all directories from another `SearchPath`.
    pub fn append_list(&mut self, other: &SearchPath) {
        self.dirs.extend(other.dirs.iter().cloned());
    }

    /// Prepend all directories from another `SearchPath` (preserving their order).
    pub fn prepend_list(&mut self, other: &SearchPath) {
        let mut merged = other.dirs.clone();
        merged.append(&mut self.dirs);
        self.dirs = merged;
    }

    /// Create a copy of this search path.
    pub fn copy_list(&self) -> SearchPath {
        self.clone()
    }

    /// Check whether a given path is contained in this search path list.
    ///
    /// Both paths are normalized before comparison, and trailing slashes
    /// are ignored.
    pub fn contains<P: AsRef<Path>>(&self, path: P) -> bool {
        let normalized = normalize_path(path.as_ref());
        self.dirs.iter().any(|d| paths_equal(d, &normalized))
    }

    /// Check whether a given path string matches any entry in this search path.
    ///
    /// This is the equivalent of pkgconf's `pkgconf_path_match_list()`.
    pub fn match_list(&self, path: &str) -> bool {
        self.contains(Path::new(path))
    }

    /// Remove duplicate entries, keeping only the first occurrence of each path.
    pub fn deduplicate(&mut self) {
        let mut seen = Vec::new();
        self.dirs.retain(|dir| {
            let dominated = seen.iter().any(|s: &PathBuf| paths_equal(s, dir));
            if !dominated {
                seen.push(dir.clone());
                true
            } else {
                false
            }
        });
    }

    /// Return a deduplicated copy without modifying the original.
    pub fn deduplicated(&self) -> Self {
        let mut copy = self.clone();
        copy.deduplicate();
        copy
    }

    /// Get the directories as a slice.
    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    /// Get a mutable reference to the directories.
    pub fn dirs_mut(&mut self) -> &mut Vec<PathBuf> {
        &mut self.dirs
    }

    /// Iterate over the directories.
    pub fn iter(&self) -> impl Iterator<Item = &PathBuf> {
        self.dirs.iter()
    }

    /// The number of directories in this search path.
    pub fn len(&self) -> usize {
        self.dirs.len()
    }

    /// Whether this search path is empty.
    pub fn is_empty(&self) -> bool {
        self.dirs.is_empty()
    }

    /// Clear all directories.
    pub fn clear(&mut self) {
        self.dirs.clear();
    }

    /// Render the search path as a delimited string.
    pub fn to_delimited(&self, separator: char) -> String {
        self.dirs
            .iter()
            .map(|d| d.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(&separator.to_string())
    }

    /// Render the search path using the platform separator.
    pub fn to_env_value(&self) -> String {
        self.to_delimited(PATH_SEPARATOR)
    }

    /// Apply a sysroot prefix to all paths in this search path.
    ///
    /// Each path that starts with `/` will be prefixed with the sysroot.
    /// Paths that already start with the sysroot are left unchanged.
    pub fn apply_sysroot(&mut self, sysroot: &str) {
        if sysroot.is_empty() {
            return;
        }
        let sysroot_path = Path::new(sysroot);
        for dir in &mut self.dirs {
            if dir.is_absolute() && !dir.starts_with(sysroot_path) {
                *dir = sysroot_path.join(dir.strip_prefix("/").unwrap_or(dir));
            }
        }
    }

    /// Find a `.pc` file by package name in this search path.
    ///
    /// Searches each directory for `{name}.pc` and returns the first match.
    pub fn find_pc_file(&self, name: &str) -> Option<PathBuf> {
        let filename = format!("{name}.pc");
        for dir in &self.dirs {
            let candidate = dir.join(&filename);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        None
    }

    /// List all `.pc` files found across all directories in this search path.
    ///
    /// Returns pairs of `(package_name, path)`. If the same package name
    /// appears in multiple directories, only the first occurrence is returned
    /// (matching pkgconf's behaviour where earlier paths take priority).
    pub fn list_all_pc_files(&self) -> Vec<(String, PathBuf)> {
        let mut result = Vec::new();
        let mut seen_names = std::collections::HashSet::new();

        for dir in &self.dirs {
            if let Ok(entries) = std::fs::read_dir(dir) {
                let mut dir_entries: Vec<_> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "pc"))
                    .collect();
                // Sort for deterministic output
                dir_entries.sort_by_key(|e| e.file_name());

                for entry in dir_entries {
                    let path = entry.path();
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        let name = stem.to_string();
                        if seen_names.insert(name.clone()) {
                            result.push((name, path));
                        }
                    }
                }
            }
        }

        result
    }
}

impl<'a> IntoIterator for &'a SearchPath {
    type Item = &'a PathBuf;
    type IntoIter = std::slice::Iter<'a, PathBuf>;

    fn into_iter(self) -> Self::IntoIter {
        self.dirs.iter()
    }
}

impl IntoIterator for SearchPath {
    type Item = PathBuf;
    type IntoIter = std::vec::IntoIter<PathBuf>;

    fn into_iter(self) -> Self::IntoIter {
        self.dirs.into_iter()
    }
}

impl FromIterator<PathBuf> for SearchPath {
    fn from_iter<I: IntoIterator<Item = PathBuf>>(iter: I) -> Self {
        Self {
            dirs: iter.into_iter().collect(),
        }
    }
}

impl std::fmt::Display for SearchPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_delimited(PATH_SEPARATOR))
    }
}

/// Normalize a path by stripping trailing separators.
///
/// This provides a canonical form for comparison. Unlike `canonicalize()`,
/// this does not resolve symlinks or require the path to exist.
fn normalize_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    let trimmed = s.trim_end_matches('/');
    if trimmed.is_empty() {
        PathBuf::from("/")
    } else {
        PathBuf::from(trimmed)
    }
}

/// Compare two paths for equality after normalization.
fn paths_equal(a: &Path, b: &Path) -> bool {
    let a_norm = normalize_path(a);
    let b_norm = normalize_path(b);
    a_norm == b_norm
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Construction ────────────────────────────────────────────────

    #[test]
    fn new_is_empty() {
        let sp = SearchPath::new();
        assert!(sp.is_empty());
        assert_eq!(sp.len(), 0);
    }

    #[test]
    fn from_paths() {
        let sp = SearchPath::from_paths(&["/usr/lib/pkgconfig", "/usr/share/pkgconfig"]);
        assert_eq!(sp.len(), 2);
        assert_eq!(sp.dirs()[0], PathBuf::from("/usr/lib/pkgconfig"));
        assert_eq!(sp.dirs()[1], PathBuf::from("/usr/share/pkgconfig"));
    }

    #[test]
    fn from_delimited_colon() {
        let sp = SearchPath::from_delimited("/a:/b:/c", ':');
        assert_eq!(sp.len(), 3);
        assert_eq!(sp.dirs()[0], PathBuf::from("/a"));
        assert_eq!(sp.dirs()[1], PathBuf::from("/b"));
        assert_eq!(sp.dirs()[2], PathBuf::from("/c"));
    }

    #[test]
    fn from_delimited_semicolon() {
        let sp = SearchPath::from_delimited("C:\\lib;D:\\lib", ';');
        assert_eq!(sp.len(), 2);
    }

    #[test]
    fn from_delimited_skips_empty() {
        let sp = SearchPath::from_delimited("/a::/b:", ':');
        assert_eq!(sp.len(), 2);
        assert_eq!(sp.dirs()[0], PathBuf::from("/a"));
        assert_eq!(sp.dirs()[1], PathBuf::from("/b"));
    }

    #[test]
    fn from_delimited_empty_string() {
        let sp = SearchPath::from_delimited("", ':');
        assert!(sp.is_empty());
    }

    #[test]
    fn from_env_value() {
        let sp = SearchPath::from_env_value("/a:/b");
        assert_eq!(sp.len(), 2);
    }

    // ── Add / Prepend ───────────────────────────────────────────────

    #[test]
    fn add_appends() {
        let mut sp = SearchPath::new();
        sp.add("/first");
        sp.add("/second");
        assert_eq!(sp.dirs()[0], PathBuf::from("/first"));
        assert_eq!(sp.dirs()[1], PathBuf::from("/second"));
    }

    #[test]
    fn add_skips_empty() {
        let mut sp = SearchPath::new();
        sp.add("");
        assert!(sp.is_empty());
    }

    #[test]
    fn add_str_works() {
        let mut sp = SearchPath::new();
        sp.add_str("/a");
        sp.add_str("");
        assert_eq!(sp.len(), 1);
    }

    #[test]
    fn prepend_inserts_at_front() {
        let mut sp = SearchPath::new();
        sp.add("/second");
        sp.prepend("/first");
        assert_eq!(sp.dirs()[0], PathBuf::from("/first"));
        assert_eq!(sp.dirs()[1], PathBuf::from("/second"));
    }

    #[test]
    fn prepend_str_works() {
        let mut sp = SearchPath::new();
        sp.add("/b");
        sp.prepend_str("/a");
        assert_eq!(sp.dirs()[0], PathBuf::from("/a"));
    }

    #[test]
    fn add_delimited() {
        let mut sp = SearchPath::new();
        sp.add("/existing");
        sp.add_delimited("/a:/b:/c", ':');
        assert_eq!(sp.len(), 4);
        assert_eq!(sp.dirs()[0], PathBuf::from("/existing"));
        assert_eq!(sp.dirs()[3], PathBuf::from("/c"));
    }

    #[test]
    fn prepend_delimited() {
        let mut sp = SearchPath::new();
        sp.add("/existing");
        sp.prepend_delimited("/a:/b", ':');
        assert_eq!(sp.len(), 3);
        assert_eq!(sp.dirs()[0], PathBuf::from("/a"));
        assert_eq!(sp.dirs()[1], PathBuf::from("/b"));
        assert_eq!(sp.dirs()[2], PathBuf::from("/existing"));
    }

    // ── List merging ────────────────────────────────────────────────

    #[test]
    fn append_list() {
        let mut sp1 = SearchPath::from_delimited("/a:/b", ':');
        let sp2 = SearchPath::from_delimited("/c:/d", ':');
        sp1.append_list(&sp2);
        assert_eq!(sp1.len(), 4);
        assert_eq!(sp1.dirs()[2], PathBuf::from("/c"));
    }

    #[test]
    fn prepend_list() {
        let mut sp1 = SearchPath::from_delimited("/c:/d", ':');
        let sp2 = SearchPath::from_delimited("/a:/b", ':');
        sp1.prepend_list(&sp2);
        assert_eq!(sp1.len(), 4);
        assert_eq!(sp1.dirs()[0], PathBuf::from("/a"));
        assert_eq!(sp1.dirs()[3], PathBuf::from("/d"));
    }

    #[test]
    fn copy_list() {
        let sp = SearchPath::from_delimited("/a:/b", ':');
        let copy = sp.copy_list();
        assert_eq!(copy.len(), 2);
        assert_eq!(copy.dirs(), sp.dirs());
    }

    // ── Contains / match_list ───────────────────────────────────────

    #[test]
    fn contains_exact() {
        let sp = SearchPath::from_delimited("/usr/lib:/usr/share", ':');
        assert!(sp.contains("/usr/lib"));
        assert!(sp.contains("/usr/share"));
        assert!(!sp.contains("/opt/lib"));
    }

    #[test]
    fn contains_with_trailing_slash() {
        let sp = SearchPath::from_delimited("/usr/lib:/usr/share", ':');
        assert!(sp.contains("/usr/lib/"));
        assert!(sp.contains("/usr/share/"));
    }

    #[test]
    fn contains_entry_has_trailing_slash() {
        let sp = SearchPath::from_delimited("/usr/lib/:/usr/share/", ':');
        assert!(sp.contains("/usr/lib"));
        assert!(sp.contains("/usr/share"));
    }

    #[test]
    fn match_list_works() {
        let sp = SearchPath::from_delimited("/usr/lib:/usr/share", ':');
        assert!(sp.match_list("/usr/lib"));
        assert!(!sp.match_list("/opt/lib"));
    }

    // ── Deduplication ───────────────────────────────────────────────

    #[test]
    fn deduplicate_removes_duplicates() {
        let mut sp = SearchPath::from_delimited("/a:/b:/a:/c:/b", ':');
        sp.deduplicate();
        assert_eq!(sp.len(), 3);
        assert_eq!(sp.dirs()[0], PathBuf::from("/a"));
        assert_eq!(sp.dirs()[1], PathBuf::from("/b"));
        assert_eq!(sp.dirs()[2], PathBuf::from("/c"));
    }

    #[test]
    fn deduplicate_trailing_slash() {
        let mut sp = SearchPath::new();
        sp.dirs.push(PathBuf::from("/usr/lib/"));
        sp.dirs.push(PathBuf::from("/usr/lib"));
        sp.deduplicate();
        assert_eq!(sp.len(), 1);
    }

    #[test]
    fn deduplicate_no_duplicates() {
        let mut sp = SearchPath::from_delimited("/a:/b:/c", ':');
        sp.deduplicate();
        assert_eq!(sp.len(), 3);
    }

    #[test]
    fn deduplicated_returns_copy() {
        let sp = SearchPath::from_delimited("/a:/b:/a", ':');
        let deduped = sp.deduplicated();
        assert_eq!(sp.len(), 3); // original unchanged
        assert_eq!(deduped.len(), 2);
    }

    // ── Rendering ───────────────────────────────────────────────────

    #[test]
    fn to_delimited() {
        let sp = SearchPath::from_delimited("/a:/b:/c", ':');
        assert_eq!(sp.to_delimited(':'), "/a:/b:/c");
    }

    #[test]
    fn to_delimited_empty() {
        let sp = SearchPath::new();
        assert_eq!(sp.to_delimited(':'), "");
    }

    #[test]
    fn display_trait() {
        let sp = SearchPath::from_delimited("/a:/b", ':');
        let displayed = format!("{sp}");
        assert!(displayed.contains("/a"));
        assert!(displayed.contains("/b"));
    }

    // ── Sysroot ─────────────────────────────────────────────────────

    #[test]
    fn apply_sysroot_prefixes_absolute_paths() {
        let mut sp = SearchPath::from_delimited("/usr/lib:/usr/share", ':');
        sp.apply_sysroot("/cross");
        assert_eq!(sp.dirs()[0], PathBuf::from("/cross/usr/lib"));
        assert_eq!(sp.dirs()[1], PathBuf::from("/cross/usr/share"));
    }

    #[test]
    fn apply_sysroot_skips_already_prefixed() {
        let mut sp = SearchPath::from_delimited("/cross/usr/lib:/usr/share", ':');
        sp.apply_sysroot("/cross");
        assert_eq!(sp.dirs()[0], PathBuf::from("/cross/usr/lib")); // unchanged
        assert_eq!(sp.dirs()[1], PathBuf::from("/cross/usr/share")); // prefixed
    }

    #[test]
    fn apply_sysroot_empty_is_noop() {
        let mut sp = SearchPath::from_delimited("/usr/lib:/usr/share", ':');
        let before = sp.dirs().to_vec();
        sp.apply_sysroot("");
        assert_eq!(sp.dirs(), &before);
    }

    // ── Iteration ───────────────────────────────────────────────────

    #[test]
    fn iter_works() {
        let sp = SearchPath::from_delimited("/a:/b", ':');
        let collected: Vec<_> = sp.iter().collect();
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn into_iter_ref() {
        let sp = SearchPath::from_delimited("/a:/b", ':');
        let collected: Vec<_> = (&sp).into_iter().collect();
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn into_iter_owned() {
        let sp = SearchPath::from_delimited("/a:/b", ':');
        let collected: Vec<PathBuf> = sp.into_iter().collect();
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn from_iterator() {
        let paths = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        let sp: SearchPath = paths.into_iter().collect();
        assert_eq!(sp.len(), 2);
    }

    // ── Clear ───────────────────────────────────────────────────────

    #[test]
    fn clear_empties_list() {
        let mut sp = SearchPath::from_delimited("/a:/b:/c", ':');
        assert!(!sp.is_empty());
        sp.clear();
        assert!(sp.is_empty());
        assert_eq!(sp.len(), 0);
    }

    // ── Helper functions ────────────────────────────────────────────

    #[test]
    fn normalize_trailing_slash() {
        assert_eq!(
            normalize_path(Path::new("/usr/lib/")),
            PathBuf::from("/usr/lib")
        );
        assert_eq!(
            normalize_path(Path::new("/usr/lib")),
            PathBuf::from("/usr/lib")
        );
        assert_eq!(normalize_path(Path::new("/")), PathBuf::from("/"));
    }

    #[test]
    fn paths_equal_ignores_trailing_slash() {
        assert!(paths_equal(Path::new("/usr/lib"), Path::new("/usr/lib/")));
        assert!(paths_equal(Path::new("/usr/lib/"), Path::new("/usr/lib")));
        assert!(!paths_equal(Path::new("/usr/lib"), Path::new("/usr/share")));
    }

    // ── File discovery ──────────────────────────────────────────────

    #[test]
    fn find_pc_file_in_test_data() {
        // Use the test data directory in our project
        let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/data");

        if test_dir.exists() {
            let sp = SearchPath::from_paths(&[test_dir.to_str().unwrap()]);
            let found = sp.find_pc_file("zlib");
            assert!(found.is_some());
            assert!(found.unwrap().ends_with("zlib.pc"));

            let not_found = sp.find_pc_file("nonexistent_package");
            assert!(not_found.is_none());
        }
    }

    #[test]
    fn list_all_pc_files_in_test_data() {
        let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/data");

        if test_dir.exists() {
            let sp = SearchPath::from_paths(&[test_dir.to_str().unwrap()]);
            let all = sp.list_all_pc_files();
            assert!(all.len() >= 3); // libbar, libfoo, zlib
            let names: Vec<_> = all.iter().map(|(n, _)| n.as_str()).collect();
            assert!(names.contains(&"zlib"));
            assert!(names.contains(&"libfoo"));
            assert!(names.contains(&"libbar"));
        }
    }

    #[test]
    fn list_all_pc_files_first_occurrence_wins() {
        // If we add the same directory twice, each package should appear only once
        let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/data");

        if test_dir.exists() {
            let dir_str = test_dir.to_str().unwrap();
            let sp = SearchPath::from_paths(&[dir_str, dir_str]);
            let all = sp.list_all_pc_files();
            // Each name should appear exactly once
            let mut names: Vec<_> = all.iter().map(|(n, _)| n.clone()).collect();
            names.sort();
            names.dedup();
            assert_eq!(names.len(), all.len());
        }
    }

    #[test]
    fn find_pc_file_empty_search_path() {
        let sp = SearchPath::new();
        assert!(sp.find_pc_file("anything").is_none());
    }

    #[test]
    fn list_all_pc_files_empty_search_path() {
        let sp = SearchPath::new();
        let all = sp.list_all_pc_files();
        assert!(all.is_empty());
    }

    #[test]
    fn list_all_pc_files_nonexistent_dir() {
        let sp = SearchPath::from_paths(&["/nonexistent/path/that/does/not/exist"]);
        let all = sp.list_all_pc_files();
        assert!(all.is_empty());
    }
}
