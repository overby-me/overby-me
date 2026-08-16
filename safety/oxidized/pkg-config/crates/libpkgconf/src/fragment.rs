//! Compiler and linker flag fragment management.
//!
//! This module handles parsing, filtering, deduplication, and rendering of
//! compiler flags (`-I`, `-D`, etc.) and linker flags (`-L`, `-l`, etc.)
//! as used by pkg-config/pkgconf.
//!
//! Fragments are the individual pieces that make up a flags string. Each
//! fragment has a **type** (a single character like `I`, `L`, `l`, `D`, etc.)
//! and associated **data** (the path or value). Fragments without a recognized
//! type character are stored as "untyped" with type `\0`.
//!
//! The module supports:
//! - Parsing flags strings into fragment lists
//! - Filtering fragments (e.g. only `-I`, only `-L`, only `-l`)
//! - Deduplication with correct semantics (some flags like `-l` must keep
//!   last occurrence, while `-I` and `-L` keep first)
//! - System directory filtering (removing `-I/usr/include`, `-L/usr/lib`, etc.)
//! - Rendering fragment lists back to strings

use std::collections::HashSet;

/// A single compiler or linker flag fragment.
///
/// Each fragment consists of a type character and a data string.
/// For example, `-I/usr/include` has type `'I'` and data `"/usr/include"`,
/// while `-lfoo` has type `'l'` and data `"foo"`.
///
/// Fragments without a recognized type prefix (e.g. bare words or flags like
/// `-pthread`) have type `'\0'` (represented as `None` in the public API).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Fragment {
    /// The fragment type character (`I`, `L`, `l`, `D`, etc.), or `\0` for untyped.
    frag_type: char,

    /// The data associated with this fragment (path, library name, define, etc.).
    pub data: String,

    /// Flags controlling fragment behaviour.
    pub flags: FragmentFlags,
}

impl Fragment {
    /// Create a new typed fragment.
    ///
    /// # Examples
    ///
    /// ```
    /// use libpkgconf::fragment::Fragment;
    ///
    /// let frag = Fragment::new('I', "/usr/include");
    /// assert_eq!(frag.frag_type(), Some('I'));
    /// assert_eq!(frag.data, "/usr/include");
    /// ```
    pub fn new(frag_type: char, data: impl Into<String>) -> Self {
        Self {
            frag_type,
            data: data.into(),
            flags: FragmentFlags::NONE,
        }
    }

    /// Create a new untyped fragment (for flags like `-pthread` or bare words).
    pub fn untyped(data: impl Into<String>) -> Self {
        Self {
            frag_type: '\0',
            data: data.into(),
            flags: FragmentFlags::NONE,
        }
    }

    /// Get the fragment type character, or `None` if untyped.
    pub fn frag_type(&self) -> Option<char> {
        if self.frag_type == '\0' {
            None
        } else {
            Some(self.frag_type)
        }
    }

    /// Get the raw type character (including `\0` for untyped).
    pub fn frag_type_raw(&self) -> char {
        self.frag_type
    }

    /// Check whether this fragment is typed.
    pub fn is_typed(&self) -> bool {
        self.frag_type != '\0'
    }

    /// Check whether this is an include-path fragment (`-I`).
    pub fn is_include(&self) -> bool {
        self.frag_type == 'I'
    }

    /// Check whether this is a library-path fragment (`-L`).
    pub fn is_lib_path(&self) -> bool {
        self.frag_type == 'L'
    }

    /// Check whether this is a library-name fragment (`-l`).
    pub fn is_lib_name(&self) -> bool {
        self.frag_type == 'l'
    }

    /// Check whether this is a define fragment (`-D`).
    pub fn is_define(&self) -> bool {
        self.frag_type == 'D'
    }

    /// Check whether this fragment refers to a system directory in the given
    /// filter lists.
    ///
    /// System directory fragments are typically filtered out by default to
    /// avoid interfering with the compiler's built-in search paths.
    pub fn has_system_dir(&self, system_libdirs: &[String], system_includedirs: &[String]) -> bool {
        match self.frag_type {
            'L' => is_path_in_list(&self.data, system_libdirs),
            'I' => is_path_in_list(&self.data, system_includedirs),
            _ => false,
        }
    }

    /// Render this fragment as a flag string (e.g. `-I/usr/include`, `-lfoo`).
    pub fn render(&self) -> String {
        if self.frag_type == '\0' {
            self.data.clone()
        } else {
            format!("-{}{}", self.frag_type, self.data)
        }
    }

    /// Render this fragment, escaping spaces in the data with backslashes.
    pub fn render_escaped(&self) -> String {
        let escaped_data = escape_fragment_data(&self.data);
        if self.frag_type == '\0' {
            escaped_data
        } else {
            format!("-{}{}", self.frag_type, escaped_data)
        }
    }

    /// Render this fragment using MSVC syntax.
    ///
    /// Translates GCC-style flags to their MSVC equivalents:
    /// - `-I<path>` → `/I<path>`
    /// - `-L<path>` → `/LIBPATH:<path>`
    /// - `-l<name>` → `<name>.lib`
    /// - `-D<def>`  → `/D<def>`
    /// - `-U<def>`  → `/U<def>`
    /// - Everything else is passed through unchanged.
    pub fn render_msvc(&self) -> String {
        match self.frag_type {
            'I' => format!("/I{}", self.data),
            'L' => format!("/LIBPATH:{}", self.data),
            'l' => format!("{}.lib", self.data),
            'D' => format!("/D{}", self.data),
            'U' => format!("/U{}", self.data),
            '\0' => self.data.clone(),
            _ => self.render(), // fall through for unknown types
        }
    }

    /// Render this fragment using MSVC syntax with escaped spaces.
    pub fn render_msvc_escaped(&self) -> String {
        let escaped_data = escape_fragment_data(&self.data);
        match self.frag_type {
            'I' => format!("/I{}", escaped_data),
            'L' => format!("/LIBPATH:{}", escaped_data),
            'l' => format!("{}.lib", escaped_data),
            'D' => format!("/D{}", escaped_data),
            'U' => format!("/U{}", escaped_data),
            '\0' => escaped_data,
            _ => self.render_escaped(), // fall through for unknown types
        }
    }

    /// Check whether this fragment type should keep the *first* occurrence
    /// during deduplication.
    ///
    /// Include paths (`-I`) and library search paths (`-L`) keep the first
    /// occurrence to preserve priority ordering. Library names (`-l`) keep
    /// the *last* occurrence to satisfy link ordering requirements.
    pub fn keeps_first(&self) -> bool {
        matches!(self.frag_type, 'I' | 'L' | 'D')
    }

    /// Check whether this fragment type should keep the *last* occurrence
    /// during deduplication.
    pub fn keeps_last(&self) -> bool {
        !self.keeps_first()
    }
}

impl std::fmt::Display for Fragment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.render())
    }
}

/// Flags that can be associated with a fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FragmentFlags(u32);

impl FragmentFlags {
    /// No flags.
    pub const NONE: Self = Self(0);
    /// Fragment has been terminated / finalized.
    pub const TERMINATED: Self = Self(0x1);

    /// Check if a flag is set.
    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Set a flag.
    pub fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Unset a flag.
    pub fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

/// An ordered list of flag fragments.
///
/// This is the primary data structure for working with compiler and linker
/// flags. It supports parsing, filtering, deduplication, and rendering.
#[derive(Debug, Clone, Default)]
pub struct FragmentList {
    fragments: Vec<Fragment>,
}

impl FragmentList {
    /// Create a new empty fragment list.
    pub fn new() -> Self {
        Self {
            fragments: Vec::new(),
        }
    }

    /// Parse a flags string into a fragment list.
    ///
    /// The input is split on whitespace (respecting quoting), then each token
    /// is classified as a typed fragment (`-Ipath`, `-lfoo`, `-Ldir`, `-DFOO`)
    /// or an untyped fragment.
    ///
    /// # Examples
    ///
    /// ```
    /// use libpkgconf::fragment::FragmentList;
    ///
    /// let list = FragmentList::parse("-I/usr/include -lfoo -L/usr/lib -DBAR=1 -pthread");
    /// assert_eq!(list.len(), 5);
    /// ```
    pub fn parse(input: &str) -> Self {
        let mut list = Self::new();

        if input.trim().is_empty() {
            return list;
        }

        let tokens = split_flags(input);

        for token in tokens {
            list.add_token(&token);
        }

        list
    }

    /// Add a single flag token, classifying it as typed or untyped.
    fn add_token(&mut self, token: &str) {
        if token.len() < 2 || !token.starts_with('-') {
            // Not a flag or too short to be a typed flag
            if !token.is_empty() {
                self.fragments.push(Fragment::untyped(token));
            }
            return;
        }

        let type_char = token.as_bytes()[1] as char;

        // Recognized single-char flag types that take an argument concatenated
        // or in the same token
        match type_char {
            'I' | 'L' | 'l' | 'D' | 'U' | 'F' | 'W' => {
                let data = &token[2..];
                // Special case: `-l foo` is handled at tokenization time; here
                // we always expect concatenated form like `-lfoo`.
                // If data is empty but type is one that requires data, store the
                // flag as-is (untyped).
                if data.is_empty() && matches!(type_char, 'I' | 'L' | 'l' | 'D' | 'U') {
                    // Flag like `-I` with no path is unusual but we emit it as-is
                    self.fragments.push(Fragment::untyped(token));
                } else {
                    self.fragments.push(Fragment::new(type_char, data));
                }
            }
            // Some flags like `-framework` on macOS are multi-character typed flags.
            // We handle `-framework Name` as two tokens: the first is untyped
            // `-framework`, the second is an untyped `Name`.
            _ => {
                self.fragments.push(Fragment::untyped(token));
            }
        }
    }

    /// Add a pre-built fragment to the list.
    pub fn push(&mut self, fragment: Fragment) {
        self.fragments.push(fragment);
    }

    /// Insert a fragment at the beginning or end of the list.
    pub fn insert(&mut self, fragment: Fragment, at_tail: bool) {
        if at_tail {
            self.fragments.push(fragment);
        } else {
            self.fragments.insert(0, fragment);
        }
    }

    /// Remove a fragment at the given index.
    pub fn remove(&mut self, index: usize) -> Fragment {
        self.fragments.remove(index)
    }

    /// Get a reference to the fragment at the given index.
    pub fn get(&self, index: usize) -> Option<&Fragment> {
        self.fragments.get(index)
    }

    /// Return the number of fragments.
    pub fn len(&self) -> usize {
        self.fragments.len()
    }

    /// Check if the list is empty.
    pub fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }

    /// Return a slice of all fragments.
    pub fn fragments(&self) -> &[Fragment] {
        &self.fragments
    }

    /// Return a mutable slice of all fragments.
    pub fn fragments_mut(&mut self) -> &mut [Fragment] {
        &mut self.fragments
    }

    /// Iterate over the fragments.
    pub fn iter(&self) -> impl Iterator<Item = &Fragment> {
        self.fragments.iter()
    }

    /// Iterate mutably over the fragments.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Fragment> {
        self.fragments.iter_mut()
    }

    /// Remove all fragments.
    pub fn clear(&mut self) {
        self.fragments.clear();
    }

    /// Append all fragments from another list.
    pub fn append(&mut self, other: &FragmentList) {
        self.fragments.extend(other.fragments.iter().cloned());
    }

    /// Copy all fragments from another list into this one.
    pub fn copy_from(&mut self, other: &FragmentList) {
        self.fragments.extend(other.fragments.iter().cloned());
    }

    /// Filter fragments, keeping only those for which the predicate returns `true`.
    pub fn filter<F>(&self, predicate: F) -> FragmentList
    where
        F: Fn(&Fragment) -> bool,
    {
        FragmentList {
            fragments: self
                .fragments
                .iter()
                .filter(|f| predicate(f))
                .cloned()
                .collect(),
        }
    }

    /// Filter out fragments that refer to system directories.
    ///
    /// This removes `-I` fragments pointing to system include directories and
    /// `-L` fragments pointing to system library directories, which is the
    /// default behaviour of pkg-config.
    pub fn filter_system_dirs(
        &self,
        system_libdirs: &[String],
        system_includedirs: &[String],
    ) -> FragmentList {
        self.filter(|f| !f.has_system_dir(system_libdirs, system_includedirs))
    }

    /// Filter to only include `-I` fragments.
    pub fn filter_cflags_only_i(&self) -> FragmentList {
        self.filter(|f| f.frag_type == 'I')
    }

    /// Filter to only include non-`-I` cflags fragments.
    pub fn filter_cflags_only_other(&self) -> FragmentList {
        self.filter(|f| f.frag_type != 'I')
    }

    /// Filter to only include `-L` fragments.
    pub fn filter_libs_only_ldpath(&self) -> FragmentList {
        self.filter(|f| f.frag_type == 'L')
    }

    /// Filter to only include `-l` fragments.
    pub fn filter_libs_only_libname(&self) -> FragmentList {
        self.filter(|f| f.frag_type == 'l')
    }

    /// Filter to only include non-`-L`, non-`-l` lib fragments.
    pub fn filter_libs_only_other(&self) -> FragmentList {
        self.filter(|f| f.frag_type != 'L' && f.frag_type != 'l')
    }

    /// Filter fragments to only those whose type character is in the given set.
    ///
    /// This corresponds to pkgconf's `--fragment-filter` option.
    pub fn filter_by_types(&self, types: &str) -> FragmentList {
        self.filter(|f| {
            if let Some(t) = f.frag_type() {
                types.contains(t)
            } else {
                // Untyped fragments are included if the filter string is empty
                // or contains '\0'. In practice, untyped fragments are usually
                // excluded by type filters.
                false
            }
        })
    }

    /// Deduplicate fragments.
    ///
    /// The deduplication strategy depends on the fragment type:
    /// - For types that "keep first" (`-I`, `-L`, `-D`): the first occurrence
    ///   is kept and subsequent duplicates are removed.
    /// - For types that "keep last" (`-l` and untyped): the last occurrence
    ///   is kept and earlier duplicates are removed.
    ///
    /// This matches pkgconf's deduplication behaviour, which is important for
    /// correct link ordering.
    pub fn deduplicate(&self) -> FragmentList {
        let mut result = Vec::new();

        // For "keeps first" types, track what we've already seen
        let mut seen_first: HashSet<(char, String)> = HashSet::new();
        // For "keeps last" types, we need to know the last occurrence index
        let mut last_occurrence: std::collections::HashMap<(char, String), usize> =
            std::collections::HashMap::new();

        // First pass: find last occurrence of each "keeps last" fragment
        for (i, frag) in self.fragments.iter().enumerate() {
            if frag.keeps_last() {
                let key = (frag.frag_type, frag.data.clone());
                last_occurrence.insert(key, i);
            }
        }

        // Second pass: build deduplicated list
        for (i, frag) in self.fragments.iter().enumerate() {
            let key = (frag.frag_type, frag.data.clone());

            if frag.keeps_first() {
                if seen_first.insert(key) {
                    result.push(frag.clone());
                }
            } else {
                // Keep only the last occurrence
                if last_occurrence.get(&key) == Some(&i) {
                    result.push(frag.clone());
                }
            }
        }

        FragmentList { fragments: result }
    }

    /// Apply a sysroot prefix to all `-I` and `-L` path fragments.
    ///
    /// For each fragment with type `I` (include path) or `L` (library path),
    /// if the path is absolute and does not already start with the sysroot,
    /// the sysroot is prepended.
    ///
    /// This implements the sysroot path rewriting that pkgconf performs
    /// when `PKG_CONFIG_SYSROOT_DIR` is set or `--define-prefix` is used
    /// with a cross-compilation sysroot.
    ///
    /// # Arguments
    ///
    /// * `sysroot` — The sysroot directory to prepend (e.g. `/usr/x86_64-linux-gnu`).
    ///
    /// # Example
    ///
    /// ```
    /// use libpkgconf::fragment::FragmentList;
    ///
    /// let mut list = FragmentList::parse("-I/usr/include -L/usr/lib -lz");
    /// list.apply_sysroot("/cross");
    /// assert_eq!(list.render(' '), "-I/cross/usr/include -L/cross/usr/lib -lz");
    /// ```
    pub fn apply_sysroot(&mut self, sysroot: &str) {
        if sysroot.is_empty() {
            return;
        }
        for frag in &mut self.fragments {
            match frag.frag_type {
                'I' | 'L' => {
                    if frag.data.starts_with('/') && !frag.data.starts_with(sysroot) {
                        frag.data = format!("{sysroot}{}", frag.data);
                    }
                }
                _ => {}
            }
        }
    }

    /// Apply FDO sysroot rules to `-I` and `-L` path fragments.
    ///
    /// Under FDO sysroot rules (`PKG_CONFIG_FDO_SYSROOT_RULES`), the sysroot
    /// is prepended to all absolute paths in `-I` and `-L` fragments, similar
    /// to [`apply_sysroot()`](FragmentList::apply_sysroot), but the sysroot
    /// is always prepended even if the path already starts with the sysroot
    /// prefix. This matches the freedesktop.org specification behavior.
    ///
    /// # Arguments
    ///
    /// * `sysroot` — The sysroot directory to prepend.
    pub fn apply_sysroot_fdo(&mut self, sysroot: &str) {
        if sysroot.is_empty() {
            return;
        }
        for frag in &mut self.fragments {
            match frag.frag_type {
                'I' | 'L' => {
                    if frag.data.starts_with('/') {
                        frag.data = format!("{sysroot}{}", frag.data);
                    }
                }
                _ => {}
            }
        }
    }

    /// Render the fragment list as a single string.
    ///
    /// Fragments are joined by the given delimiter (typically `' '` or `'\n'`).
    pub fn render(&self, delimiter: char) -> String {
        self.fragments
            .iter()
            .map(|f| f.render())
            .collect::<Vec<_>>()
            .join(&delimiter.to_string())
    }

    /// Render the fragment list with escaped spaces in data.
    pub fn render_escaped(&self, delimiter: char) -> String {
        self.fragments
            .iter()
            .map(|f| f.render_escaped())
            .collect::<Vec<_>>()
            .join(&delimiter.to_string())
    }

    /// Render the fragment list using MSVC syntax.
    ///
    /// Translates GCC-style flags to MSVC equivalents:
    /// - `-I` → `/I`
    /// - `-L` → `/LIBPATH:`
    /// - `-l` → `<name>.lib`
    /// - `-D` → `/D`
    pub fn render_msvc(&self, delimiter: char) -> String {
        self.fragments
            .iter()
            .map(|f| f.render_msvc())
            .collect::<Vec<_>>()
            .join(&delimiter.to_string())
    }

    /// Render the fragment list using MSVC syntax with escaped spaces.
    pub fn render_msvc_escaped(&self, delimiter: char) -> String {
        self.fragments
            .iter()
            .map(|f| f.render_msvc_escaped())
            .collect::<Vec<_>>()
            .join(&delimiter.to_string())
    }

    /// Compute the total rendered length (for pre-allocation).
    pub fn render_len(&self, escaped: bool) -> usize {
        if self.fragments.is_empty() {
            return 0;
        }

        let mut total = 0;
        for (i, frag) in self.fragments.iter().enumerate() {
            if i > 0 {
                total += 1; // delimiter
            }
            if escaped {
                total += frag.render_escaped().len();
            } else {
                total += frag.render().len();
            }
        }
        total
    }
}

impl IntoIterator for FragmentList {
    type Item = Fragment;
    type IntoIter = std::vec::IntoIter<Fragment>;

    fn into_iter(self) -> Self::IntoIter {
        self.fragments.into_iter()
    }
}

impl<'a> IntoIterator for &'a FragmentList {
    type Item = &'a Fragment;
    type IntoIter = std::slice::Iter<'a, Fragment>;

    fn into_iter(self) -> Self::IntoIter {
        self.fragments.iter()
    }
}

impl FromIterator<Fragment> for FragmentList {
    fn from_iter<I: IntoIterator<Item = Fragment>>(iter: I) -> Self {
        Self {
            fragments: iter.into_iter().collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Check if a path (possibly with trailing slash normalization) matches any
/// entry in a directory list.
fn is_path_in_list(path: &str, dirs: &[String]) -> bool {
    let normalized = normalize_path(path);
    for dir in dirs {
        let normalized_dir = normalize_path(dir);
        if normalized == normalized_dir {
            return true;
        }
    }
    false
}

/// Normalize a path string by removing trailing slashes (but keeping root `/`).
fn normalize_path(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() { "/" } else { trimmed }
}

/// Escape whitespace in fragment data with backslashes.
fn escape_fragment_data(data: &str) -> String {
    let mut result = String::with_capacity(data.len());
    for c in data.chars() {
        if c == ' ' || c == '\t' {
            result.push('\\');
        }
        result.push(c);
    }
    result
}

/// Split a flags string into tokens, respecting quoting and backslash escapes.
///
/// This handles:
/// - Single-quoted strings: `'path with spaces'`
/// - Double-quoted strings: `"path with spaces"`
/// - Backslash-escaped spaces: `path\ with\ spaces`
/// - Whitespace delimiters (space, tab, newline)
fn split_flags(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while let Some(c) = chars.next() {
        if in_single_quote {
            if c == '\'' {
                in_single_quote = false;
            } else {
                current.push(c);
            }
        } else if in_double_quote {
            if c == '"' {
                in_double_quote = false;
            } else if c == '\\' {
                if let Some(&('"' | '\\' | '$' | '`')) = chars.peek() {
                    current.push(chars.next().unwrap());
                } else {
                    current.push('\\');
                }
            } else {
                current.push(c);
            }
        } else {
            match c {
                '\'' => {
                    in_single_quote = true;
                }
                '"' => {
                    in_double_quote = true;
                }
                '\\' => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                c if c.is_ascii_whitespace() => {
                    if !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                    }
                }
                _ => {
                    current.push(c);
                }
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Fragment basics
    // -------------------------------------------------------------------------

    #[test]
    fn fragment_new_typed() {
        let frag = Fragment::new('I', "/usr/include");
        assert_eq!(frag.frag_type(), Some('I'));
        assert_eq!(frag.frag_type_raw(), 'I');
        assert_eq!(frag.data, "/usr/include");
        assert!(frag.is_typed());
        assert!(frag.is_include());
        assert!(!frag.is_lib_path());
        assert!(!frag.is_lib_name());
    }

    #[test]
    fn fragment_new_untyped() {
        let frag = Fragment::untyped("-pthread");
        assert_eq!(frag.frag_type(), None);
        assert!(!frag.is_typed());
        assert_eq!(frag.data, "-pthread");
    }

    #[test]
    fn fragment_types() {
        assert!(Fragment::new('L', "/usr/lib").is_lib_path());
        assert!(Fragment::new('l', "foo").is_lib_name());
        assert!(Fragment::new('D', "FOO=1").is_define());
        assert!(Fragment::new('I', "/inc").is_include());
    }

    #[test]
    fn fragment_render() {
        assert_eq!(
            Fragment::new('I', "/usr/include").render(),
            "-I/usr/include"
        );
        assert_eq!(Fragment::new('l', "foo").render(), "-lfoo");
        assert_eq!(Fragment::new('L', "/usr/lib").render(), "-L/usr/lib");
        assert_eq!(Fragment::new('D', "BAR=1").render(), "-DBAR=1");
        assert_eq!(Fragment::untyped("-pthread").render(), "-pthread");
    }

    #[test]
    fn fragment_render_escaped() {
        let frag = Fragment::new('I', "/path with spaces/include");
        assert_eq!(frag.render_escaped(), r"-I/path\ with\ spaces/include");

        let frag2 = Fragment::untyped("no spaces");
        assert_eq!(frag2.render_escaped(), r"no\ spaces");
    }

    #[test]
    fn fragment_display() {
        let frag = Fragment::new('l', "foo");
        assert_eq!(format!("{}", frag), "-lfoo");
    }

    #[test]
    fn fragment_keeps_first_and_last() {
        assert!(Fragment::new('I', "x").keeps_first());
        assert!(Fragment::new('L', "x").keeps_first());
        assert!(Fragment::new('D', "x").keeps_first());

        assert!(Fragment::new('l', "x").keeps_last());
        assert!(Fragment::untyped("x").keeps_last());
    }

    #[test]
    fn fragment_has_system_dir() {
        let libdirs = vec!["/usr/lib".to_string(), "/lib".to_string()];
        let incdirs = vec!["/usr/include".to_string()];

        assert!(Fragment::new('L', "/usr/lib").has_system_dir(&libdirs, &incdirs));
        assert!(Fragment::new('L', "/lib").has_system_dir(&libdirs, &incdirs));
        assert!(!Fragment::new('L', "/opt/lib").has_system_dir(&libdirs, &incdirs));

        assert!(Fragment::new('I', "/usr/include").has_system_dir(&libdirs, &incdirs));
        assert!(!Fragment::new('I', "/opt/include").has_system_dir(&libdirs, &incdirs));

        // Non-path types are never system dirs
        assert!(!Fragment::new('l', "foo").has_system_dir(&libdirs, &incdirs));
        assert!(!Fragment::new('D', "FOO").has_system_dir(&libdirs, &incdirs));
    }

    #[test]
    fn fragment_has_system_dir_trailing_slash() {
        let libdirs = vec!["/usr/lib".to_string()];
        let incdirs = vec!["/usr/include/".to_string()];

        // Should match even with trailing slash differences
        assert!(Fragment::new('L', "/usr/lib/").has_system_dir(&libdirs, &incdirs));
        assert!(Fragment::new('I', "/usr/include").has_system_dir(&libdirs, &incdirs));
    }

    // -------------------------------------------------------------------------
    // FragmentFlags
    // -------------------------------------------------------------------------

    #[test]
    fn fragment_flags() {
        let flags = FragmentFlags::NONE;
        assert!(!flags.contains(FragmentFlags::TERMINATED));

        let flags2 = flags.with(FragmentFlags::TERMINATED);
        assert!(flags2.contains(FragmentFlags::TERMINATED));

        let flags3 = flags2.without(FragmentFlags::TERMINATED);
        assert!(!flags3.contains(FragmentFlags::TERMINATED));
    }

    // -------------------------------------------------------------------------
    // FragmentList parsing
    // -------------------------------------------------------------------------

    #[test]
    fn parse_empty() {
        let list = FragmentList::parse("");
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
    }

    #[test]
    fn parse_whitespace_only() {
        let list = FragmentList::parse("   \t  \n  ");
        assert!(list.is_empty());
    }

    #[test]
    fn parse_single_include() {
        let list = FragmentList::parse("-I/usr/include");
        assert_eq!(list.len(), 1);
        assert_eq!(list.fragments()[0].frag_type(), Some('I'));
        assert_eq!(list.fragments()[0].data, "/usr/include");
    }

    #[test]
    fn parse_single_lib() {
        let list = FragmentList::parse("-lfoo");
        assert_eq!(list.len(), 1);
        assert_eq!(list.fragments()[0].frag_type(), Some('l'));
        assert_eq!(list.fragments()[0].data, "foo");
    }

    #[test]
    fn parse_single_libpath() {
        let list = FragmentList::parse("-L/usr/lib");
        assert_eq!(list.len(), 1);
        assert_eq!(list.fragments()[0].frag_type(), Some('L'));
        assert_eq!(list.fragments()[0].data, "/usr/lib");
    }

    #[test]
    fn parse_single_define() {
        let list = FragmentList::parse("-DFOO=bar");
        assert_eq!(list.len(), 1);
        assert_eq!(list.fragments()[0].frag_type(), Some('D'));
        assert_eq!(list.fragments()[0].data, "FOO=bar");
    }

    #[test]
    fn parse_untyped() {
        let list = FragmentList::parse("-pthread");
        assert_eq!(list.len(), 1);
        assert_eq!(list.fragments()[0].frag_type(), None);
        assert_eq!(list.fragments()[0].data, "-pthread");
    }

    #[test]
    fn parse_multiple_flags() {
        let list = FragmentList::parse("-I/usr/include -L/usr/lib -lfoo -lbar -pthread");
        assert_eq!(list.len(), 5);
        assert_eq!(list.fragments()[0].frag_type(), Some('I'));
        assert_eq!(list.fragments()[0].data, "/usr/include");
        assert_eq!(list.fragments()[1].frag_type(), Some('L'));
        assert_eq!(list.fragments()[1].data, "/usr/lib");
        assert_eq!(list.fragments()[2].frag_type(), Some('l'));
        assert_eq!(list.fragments()[2].data, "foo");
        assert_eq!(list.fragments()[3].frag_type(), Some('l'));
        assert_eq!(list.fragments()[3].data, "bar");
        assert_eq!(list.fragments()[4].frag_type(), None);
        assert_eq!(list.fragments()[4].data, "-pthread");
    }

    #[test]
    fn parse_defines_with_values() {
        let list = FragmentList::parse("-DVERSION=2 -DDEBUG -UOLD");
        assert_eq!(list.len(), 3);
        assert_eq!(list.fragments()[0].frag_type(), Some('D'));
        assert_eq!(list.fragments()[0].data, "VERSION=2");
        assert_eq!(list.fragments()[1].frag_type(), Some('D'));
        assert_eq!(list.fragments()[1].data, "DEBUG");
        assert_eq!(list.fragments()[2].frag_type(), Some('U'));
        assert_eq!(list.fragments()[2].data, "OLD");
    }

    #[test]
    fn parse_with_extra_whitespace() {
        let list = FragmentList::parse("  -lfoo   -lbar  ");
        assert_eq!(list.len(), 2);
        assert_eq!(list.fragments()[0].data, "foo");
        assert_eq!(list.fragments()[1].data, "bar");
    }

    #[test]
    fn parse_quoted_paths() {
        let list = FragmentList::parse(r#"-I"/path with spaces/include" -lfoo"#);
        assert_eq!(list.len(), 2);
        assert_eq!(list.fragments()[0].frag_type(), Some('I'));
        assert_eq!(list.fragments()[0].data, "/path with spaces/include");
    }

    #[test]
    fn parse_backslash_escaped_spaces() {
        let list = FragmentList::parse(r"-I/path\ with\ spaces/include -lfoo");
        assert_eq!(list.len(), 2);
        assert_eq!(list.fragments()[0].frag_type(), Some('I'));
        assert_eq!(list.fragments()[0].data, "/path with spaces/include");
    }

    #[test]
    fn parse_framework_flags() {
        let list = FragmentList::parse("-framework CoreFoundation");
        assert_eq!(list.len(), 2);
        // `-framework` is an untyped fragment, `CoreFoundation` is also untyped
        assert_eq!(list.fragments()[0].frag_type(), None);
        assert_eq!(list.fragments()[0].data, "-framework");
        assert_eq!(list.fragments()[1].frag_type(), None);
        assert_eq!(list.fragments()[1].data, "CoreFoundation");
    }

    #[test]
    fn parse_warning_flags() {
        let list = FragmentList::parse("-Wall -Wextra -Werror");
        assert_eq!(list.len(), 3);
        assert_eq!(list.fragments()[0].frag_type(), Some('W'));
        assert_eq!(list.fragments()[0].data, "all");
    }

    #[test]
    fn parse_bare_flag_no_data() {
        // `-I` with no path should become untyped
        let list = FragmentList::parse("-I -lfoo");
        assert_eq!(list.len(), 2);
        assert_eq!(list.fragments()[0].frag_type(), None);
        assert_eq!(list.fragments()[0].data, "-I");
        assert_eq!(list.fragments()[1].frag_type(), Some('l'));
    }

    // -------------------------------------------------------------------------
    // FragmentList operations
    // -------------------------------------------------------------------------

    #[test]
    fn list_push_and_get() {
        let mut list = FragmentList::new();
        list.push(Fragment::new('l', "foo"));
        list.push(Fragment::new('l', "bar"));

        assert_eq!(list.len(), 2);
        assert_eq!(list.get(0).unwrap().data, "foo");
        assert_eq!(list.get(1).unwrap().data, "bar");
        assert!(list.get(2).is_none());
    }

    #[test]
    fn list_insert_head() {
        let mut list = FragmentList::new();
        list.push(Fragment::new('l', "bar"));
        list.insert(Fragment::new('l', "foo"), false);

        assert_eq!(list.fragments()[0].data, "foo");
        assert_eq!(list.fragments()[1].data, "bar");
    }

    #[test]
    fn list_insert_tail() {
        let mut list = FragmentList::new();
        list.push(Fragment::new('l', "foo"));
        list.insert(Fragment::new('l', "bar"), true);

        assert_eq!(list.fragments()[0].data, "foo");
        assert_eq!(list.fragments()[1].data, "bar");
    }

    #[test]
    fn list_remove() {
        let mut list = FragmentList::parse("-lfoo -lbar -lbaz");
        let removed = list.remove(1);
        assert_eq!(removed.data, "bar");
        assert_eq!(list.len(), 2);
        assert_eq!(list.fragments()[0].data, "foo");
        assert_eq!(list.fragments()[1].data, "baz");
    }

    #[test]
    fn list_clear() {
        let mut list = FragmentList::parse("-lfoo -lbar");
        assert!(!list.is_empty());
        list.clear();
        assert!(list.is_empty());
    }

    #[test]
    fn list_append() {
        let mut list1 = FragmentList::parse("-lfoo");
        let list2 = FragmentList::parse("-lbar -lbaz");
        list1.append(&list2);

        assert_eq!(list1.len(), 3);
        assert_eq!(list1.fragments()[0].data, "foo");
        assert_eq!(list1.fragments()[1].data, "bar");
        assert_eq!(list1.fragments()[2].data, "baz");
    }

    #[test]
    fn list_into_iter() {
        let list = FragmentList::parse("-lfoo -lbar");
        let data: Vec<String> = list.into_iter().map(|f| f.data).collect();
        assert_eq!(data, vec!["foo", "bar"]);
    }

    #[test]
    fn list_from_iterator() {
        let frags = vec![Fragment::new('l', "foo"), Fragment::new('l', "bar")];
        let list: FragmentList = frags.into_iter().collect();
        assert_eq!(list.len(), 2);
    }

    // -------------------------------------------------------------------------
    // Filtering
    // -------------------------------------------------------------------------

    #[test]
    fn filter_custom_predicate() {
        let list = FragmentList::parse("-I/inc -L/lib -lfoo -DBAR -pthread");
        let only_typed = list.filter(|f| f.is_typed());
        assert_eq!(only_typed.len(), 4); // -I, -L, -l, -D
    }

    #[test]
    fn filter_system_dirs() {
        let list = FragmentList::parse("-I/usr/include -I/opt/include -L/usr/lib -L/opt/lib -lfoo");
        let libdirs = vec!["/usr/lib".to_string()];
        let incdirs = vec!["/usr/include".to_string()];

        let filtered = list.filter_system_dirs(&libdirs, &incdirs);
        assert_eq!(filtered.len(), 3);
        assert_eq!(filtered.fragments()[0].data, "/opt/include");
        assert_eq!(filtered.fragments()[1].data, "/opt/lib");
        assert_eq!(filtered.fragments()[2].data, "foo");
    }

    #[test]
    fn filter_cflags_only_i() {
        let list = FragmentList::parse("-I/inc1 -I/inc2 -DFOO -Wall");
        let result = list.filter_cflags_only_i();
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|f| f.frag_type() == Some('I')));
    }

    #[test]
    fn filter_cflags_only_other() {
        let list = FragmentList::parse("-I/inc -DFOO -Wall -pthread");
        let result = list.filter_cflags_only_other();
        assert_eq!(result.len(), 3);
        assert!(result.iter().all(|f| f.frag_type() != Some('I')));
    }

    #[test]
    fn filter_libs_only_ldpath() {
        let list = FragmentList::parse("-L/lib1 -L/lib2 -lfoo -lbar -pthread");
        let result = list.filter_libs_only_ldpath();
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|f| f.frag_type() == Some('L')));
    }

    #[test]
    fn filter_libs_only_libname() {
        let list = FragmentList::parse("-L/lib -lfoo -lbar -pthread");
        let result = list.filter_libs_only_libname();
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|f| f.frag_type() == Some('l')));
    }

    #[test]
    fn filter_libs_only_other() {
        let list = FragmentList::parse("-L/lib -lfoo -pthread -Wl,--as-needed");
        let result = list.filter_libs_only_other();
        assert_eq!(result.len(), 2);
        assert!(
            result
                .iter()
                .all(|f| f.frag_type() != Some('L') && f.frag_type() != Some('l'))
        );
    }

    #[test]
    fn filter_by_types() {
        let list = FragmentList::parse("-I/inc -L/lib -lfoo -DBAR -pthread");
        let result = list.filter_by_types("Il");
        assert_eq!(result.len(), 2);
        assert_eq!(result.fragments()[0].frag_type(), Some('I'));
        assert_eq!(result.fragments()[1].frag_type(), Some('l'));
    }

    // -------------------------------------------------------------------------
    // Deduplication
    // -------------------------------------------------------------------------

    #[test]
    fn deduplicate_keeps_first_for_includes() {
        let list = FragmentList::parse("-I/foo -I/bar -I/foo -I/baz");
        let deduped = list.deduplicate();
        assert_eq!(deduped.len(), 3);
        assert_eq!(deduped.fragments()[0].data, "/foo");
        assert_eq!(deduped.fragments()[1].data, "/bar");
        assert_eq!(deduped.fragments()[2].data, "/baz");
    }

    #[test]
    fn deduplicate_keeps_first_for_libpaths() {
        let list = FragmentList::parse("-L/foo -L/bar -L/foo");
        let deduped = list.deduplicate();
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped.fragments()[0].data, "/foo");
        assert_eq!(deduped.fragments()[1].data, "/bar");
    }

    #[test]
    fn deduplicate_keeps_last_for_libnames() {
        let list = FragmentList::parse("-lfoo -lbar -lfoo -lbaz");
        let deduped = list.deduplicate();
        assert_eq!(deduped.len(), 3);
        // -lbar comes first since it's unique
        // -lfoo's last occurrence (index 2) is kept, -lbaz follows
        assert_eq!(deduped.fragments()[0].data, "bar");
        assert_eq!(deduped.fragments()[1].data, "foo");
        assert_eq!(deduped.fragments()[2].data, "baz");
    }

    #[test]
    fn deduplicate_mixed() {
        let list = FragmentList::parse("-I/inc -L/lib -lfoo -I/inc -lbar -lfoo -L/lib -lbaz");
        let deduped = list.deduplicate();

        // -I/inc: keeps first -> position 0
        // -L/lib: keeps first -> position 1
        // -lbar: keeps last, unique -> position 2 (was index 4, last)
        // -lfoo: keeps last -> position 3 (index 5 is last occurrence)
        // -lbaz: keeps last, unique -> position 4
        let rendered: Vec<String> = deduped.iter().map(|f| f.render()).collect();
        assert_eq!(
            rendered,
            vec!["-I/inc", "-L/lib", "-lbar", "-lfoo", "-lbaz"]
        );
    }

    #[test]
    fn deduplicate_preserves_untyped_order() {
        let list = FragmentList::parse("-pthread -lm -pthread");
        let deduped = list.deduplicate();
        // -pthread is untyped, keeps last -> only the second occurrence survives
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped.fragments()[0].data, "m");
        assert_eq!(deduped.fragments()[1].data, "-pthread");
    }

    #[test]
    fn deduplicate_no_duplicates() {
        let list = FragmentList::parse("-I/a -I/b -L/c -lfoo -lbar");
        let deduped = list.deduplicate();
        assert_eq!(deduped.len(), 5);
    }

    #[test]
    fn deduplicate_empty() {
        let list = FragmentList::new();
        let deduped = list.deduplicate();
        assert!(deduped.is_empty());
    }

    #[test]
    fn deduplicate_defines_keeps_first() {
        let list = FragmentList::parse("-DFOO -DBAR -DFOO -DBAZ");
        let deduped = list.deduplicate();
        assert_eq!(deduped.len(), 3);
        assert_eq!(deduped.fragments()[0].data, "FOO");
        assert_eq!(deduped.fragments()[1].data, "BAR");
        assert_eq!(deduped.fragments()[2].data, "BAZ");
    }

    // -------------------------------------------------------------------------
    // Rendering
    // -------------------------------------------------------------------------

    #[test]
    fn render_empty() {
        let list = FragmentList::new();
        assert_eq!(list.render(' '), "");
    }

    #[test]
    fn render_single() {
        let list = FragmentList::parse("-lfoo");
        assert_eq!(list.render(' '), "-lfoo");
    }

    #[test]
    fn render_multiple_space() {
        let list = FragmentList::parse("-I/inc -lfoo -lbar");
        assert_eq!(list.render(' '), "-I/inc -lfoo -lbar");
    }

    #[test]
    fn render_multiple_newline() {
        let list = FragmentList::parse("-I/inc -lfoo");
        assert_eq!(list.render('\n'), "-I/inc\n-lfoo");
    }

    #[test]
    fn render_escaped() {
        let mut list = FragmentList::new();
        list.push(Fragment::new('I', "/path with spaces"));
        list.push(Fragment::new('l', "foo"));
        assert_eq!(list.render_escaped(' '), r"-I/path\ with\ spaces -lfoo");
    }

    #[test]
    fn render_len() {
        let list = FragmentList::parse("-lfoo -lbar");
        // "-lfoo -lbar" = 11 chars
        assert_eq!(list.render_len(false), 11);
    }

    // -------------------------------------------------------------------------
    // Helper function tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path("/usr/lib"), "/usr/lib");
        assert_eq!(normalize_path("/usr/lib/"), "/usr/lib");
        assert_eq!(normalize_path("/usr/lib//"), "/usr/lib");
        assert_eq!(normalize_path("/"), "/");
        assert_eq!(normalize_path(""), "/");
    }

    #[test]
    fn test_is_path_in_list() {
        let dirs = vec!["/usr/lib".to_string(), "/lib".to_string()];
        assert!(is_path_in_list("/usr/lib", &dirs));
        assert!(is_path_in_list("/usr/lib/", &dirs));
        assert!(is_path_in_list("/lib", &dirs));
        assert!(!is_path_in_list("/opt/lib", &dirs));
    }

    #[test]
    fn test_escape_fragment_data() {
        assert_eq!(escape_fragment_data("hello"), "hello");
        assert_eq!(escape_fragment_data("hello world"), r"hello\ world");
        assert_eq!(escape_fragment_data("a\tb"), "a\\\tb");
    }

    #[test]
    fn test_split_flags() {
        assert_eq!(split_flags("-lfoo -lbar"), vec!["-lfoo", "-lbar"]);
        assert_eq!(split_flags("  -lfoo  "), vec!["-lfoo"]);
        assert!(split_flags("").is_empty());
    }

    #[test]
    fn test_split_flags_quoted() {
        let result = split_flags(r#"-I"/path with spaces" -lfoo"#);
        assert_eq!(result, vec!["-I/path with spaces", "-lfoo"]);
    }

    #[test]
    fn test_split_flags_backslash() {
        let result = split_flags(r"-I/path\ with\ spaces -lfoo");
        assert_eq!(result, vec!["-I/path with spaces", "-lfoo"]);
    }

    // -------------------------------------------------------------------------
    // Real-world examples
    // -------------------------------------------------------------------------

    // ── MSVC syntax rendering tests ──────────────────────────────────

    #[test]
    fn msvc_render_include() {
        let f = Fragment::new('I', "/usr/include/glib-2.0");
        assert_eq!(f.render_msvc(), "/I/usr/include/glib-2.0");
    }

    #[test]
    fn msvc_render_libpath() {
        let f = Fragment::new('L', "/usr/lib");
        assert_eq!(f.render_msvc(), "/LIBPATH:/usr/lib");
    }

    #[test]
    fn msvc_render_libname() {
        let f = Fragment::new('l', "z");
        assert_eq!(f.render_msvc(), "z.lib");
    }

    #[test]
    fn msvc_render_define() {
        let f = Fragment::new('D', "HAVE_CONFIG_H");
        assert_eq!(f.render_msvc(), "/DHAVE_CONFIG_H");
    }

    #[test]
    fn msvc_render_define_with_value() {
        let f = Fragment::new('D', "VERSION=\"1.0\"");
        assert_eq!(f.render_msvc(), "/DVERSION=\"1.0\"");
    }

    #[test]
    fn msvc_render_undefine() {
        let f = Fragment::new('U', "NDEBUG");
        assert_eq!(f.render_msvc(), "/UNDEBUG");
    }

    #[test]
    fn msvc_render_untyped_passthrough() {
        let f = Fragment::untyped("-pthread");
        assert_eq!(f.render_msvc(), "-pthread");
    }

    #[test]
    fn msvc_render_unknown_type_passthrough() {
        let f = Fragment::new('W', "all");
        assert_eq!(f.render_msvc(), "-Wall");
    }

    #[test]
    fn msvc_render_escaped_spaces() {
        let f = Fragment::new('I', "/usr/include/my dir");
        assert_eq!(f.render_msvc_escaped(), "/I/usr/include/my\\ dir");
    }

    #[test]
    fn msvc_render_libname_escaped() {
        let f = Fragment::new('l', "my lib");
        assert_eq!(f.render_msvc_escaped(), "my\\ lib.lib");
    }

    #[test]
    fn msvc_list_render() {
        let list = FragmentList::parse("-I/usr/include -L/usr/lib -lz -DFOO");
        assert_eq!(
            list.render_msvc(' '),
            "/I/usr/include /LIBPATH:/usr/lib z.lib /DFOO"
        );
    }

    #[test]
    fn msvc_list_render_escaped() {
        let list = FragmentList::parse("-I/usr/include -lz");
        assert_eq!(list.render_msvc_escaped(' '), "/I/usr/include z.lib");
    }

    #[test]
    fn msvc_list_render_with_newline_delimiter() {
        let list = FragmentList::parse("-I/usr/include -lz");
        assert_eq!(list.render_msvc('\n'), "/I/usr/include\nz.lib");
    }

    // ── Sysroot tests ────────────────────────────────────────────────

    #[test]
    fn apply_sysroot_to_include_and_libpath() {
        let mut list = FragmentList::parse("-I/usr/include -L/usr/lib -lz -DFOO");
        list.apply_sysroot("/cross");
        assert_eq!(
            list.render(' '),
            "-I/cross/usr/include -L/cross/usr/lib -lz -DFOO"
        );
    }

    #[test]
    fn apply_sysroot_skips_relative_paths() {
        let mut list = FragmentList::parse("-Iinclude -Llib -lz");
        list.apply_sysroot("/cross");
        assert_eq!(list.render(' '), "-Iinclude -Llib -lz");
    }

    #[test]
    fn apply_sysroot_skips_already_prefixed() {
        let mut list = FragmentList::parse("-I/cross/usr/include -L/cross/usr/lib");
        list.apply_sysroot("/cross");
        assert_eq!(list.render(' '), "-I/cross/usr/include -L/cross/usr/lib");
    }

    #[test]
    fn apply_sysroot_empty_is_noop() {
        let mut list = FragmentList::parse("-I/usr/include -L/usr/lib");
        list.apply_sysroot("");
        assert_eq!(list.render(' '), "-I/usr/include -L/usr/lib");
    }

    #[test]
    fn apply_sysroot_does_not_modify_libname_or_define() {
        let mut list = FragmentList::parse("-lfoo -DBAR");
        list.apply_sysroot("/cross");
        assert_eq!(list.render(' '), "-lfoo -DBAR");
    }

    #[test]
    fn apply_sysroot_fdo_always_prepends() {
        let mut list = FragmentList::parse("-I/cross/usr/include -L/usr/lib");
        list.apply_sysroot_fdo("/cross");
        assert_eq!(
            list.render(' '),
            "-I/cross/cross/usr/include -L/cross/usr/lib"
        );
    }

    #[test]
    fn apply_sysroot_fdo_empty_is_noop() {
        let mut list = FragmentList::parse("-I/usr/include");
        list.apply_sysroot_fdo("");
        assert_eq!(list.render(' '), "-I/usr/include");
    }

    // ── Real-world tests ─────────────────────────────────────────────

    #[test]
    fn real_world_glib() {
        let cflags = "-I/usr/include/glib-2.0 -I/usr/lib/glib-2.0/include -I/usr/include";
        let list = FragmentList::parse(cflags);
        assert_eq!(list.len(), 3);
        assert!(list.iter().all(|f| f.frag_type() == Some('I')));

        // Filter system dirs
        let system_incdirs = vec!["/usr/include".to_string()];
        let filtered = list.filter_system_dirs(&[], &system_incdirs);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered.fragments()[0].data, "/usr/include/glib-2.0");
        assert_eq!(filtered.fragments()[1].data, "/usr/lib/glib-2.0/include");
    }

    #[test]
    fn real_world_gtk_libs() {
        let libs = "-L/usr/lib -lgtk-3 -lgdk-3 -lpangocairo-1.0 -lpango-1.0 -lharfbuzz \
                    -latk-1.0 -lcairo-gobject -lcairo -lgdk_pixbuf-2.0 -lgio-2.0 \
                    -lgobject-2.0 -lglib-2.0";
        let list = FragmentList::parse(libs);

        let ldpaths = list.filter_libs_only_ldpath();
        assert_eq!(ldpaths.len(), 1);
        assert_eq!(ldpaths.fragments()[0].data, "/usr/lib");

        let libnames = list.filter_libs_only_libname();
        assert!(libnames.len() > 5);
        assert_eq!(libnames.fragments()[0].data, "gtk-3");
    }

    #[test]
    fn real_world_complex_flags() {
        let flags = "-I/usr/include -DNDEBUG -DG_DISABLE_ASSERT -O2 -Wall -Wextra -pthread";
        let list = FragmentList::parse(flags);
        assert_eq!(list.len(), 7);

        // Include paths
        let includes = list.filter_cflags_only_i();
        assert_eq!(includes.len(), 1);

        // Everything else
        let other = list.filter_cflags_only_other();
        assert_eq!(other.len(), 6);
    }

    #[test]
    fn deduplicate_real_world() {
        // Simulate what happens when multiple packages contribute the same flags
        let flags = "-I/usr/include/glib-2.0 -I/usr/include/glib-2.0 \
                     -L/usr/lib -L/usr/lib -lglib-2.0 -lgobject-2.0 -lglib-2.0";
        let list = FragmentList::parse(flags);
        let deduped = list.deduplicate();

        // Includes and lib paths: keep first, so one of each
        // Lib names: keep last, so -lgobject-2.0 and -lglib-2.0 (last occurrences)
        let rendered = deduped.render(' ');
        assert!(rendered.contains("-I/usr/include/glib-2.0"));
        assert!(rendered.contains("-L/usr/lib"));
        assert!(rendered.contains("-lgobject-2.0"));
        assert!(rendered.contains("-lglib-2.0"));

        // Count occurrences
        let i_count = deduped
            .iter()
            .filter(|f| f.frag_type() == Some('I'))
            .count();
        assert_eq!(i_count, 1);

        let l_path_count = deduped
            .iter()
            .filter(|f| f.frag_type() == Some('L'))
            .count();
        assert_eq!(l_path_count, 1);
    }
}
