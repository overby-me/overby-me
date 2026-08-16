//! Parser for `.pc` (pkg-config) files.
//!
//! This module implements parsing of `.pc` files as used by pkg-config/pkgconf.
//! The file format consists of:
//!
//! - **Variable definitions**: `name=value` (no space before `=`)
//! - **Field declarations**: `Name: value` (keyword followed by `:`)
//! - **Comments**: lines starting with `#`
//! - **Variable interpolation**: `${variable_name}` within values
//!
//! The parser follows the same semantics as pkgconf's `parser.c` and `pkg.c`,
//! including support for multi-line values (trailing `\` continuation),
//! variable expansion, and the full set of standard fields.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// All known keyword fields in a `.pc` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keyword {
    Name,
    Description,
    Version,
    URL,
    Requires,
    RequiresPrivate,
    Conflicts,
    Provides,
    Libs,
    LibsPrivate,
    Cflags,
    CflagsPrivate,
    License,
    Maintainer,
    Copyright,
    Source,
    LicenseFile,
}

impl Keyword {
    /// Try to parse a keyword from a string (case-sensitive, as pkgconf does).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Name" => Some(Self::Name),
            "Description" => Some(Self::Description),
            "Version" => Some(Self::Version),
            "URL" => Some(Self::URL),
            "Requires" => Some(Self::Requires),
            "Requires.private" => Some(Self::RequiresPrivate),
            "Conflicts" => Some(Self::Conflicts),
            "Provides" => Some(Self::Provides),
            "Libs" => Some(Self::Libs),
            "Libs.private" => Some(Self::LibsPrivate),
            "Cflags" => Some(Self::Cflags),
            "Cflags.private" => Some(Self::CflagsPrivate),
            "License" => Some(Self::License),
            "Maintainer" => Some(Self::Maintainer),
            "Copyright" => Some(Self::Copyright),
            "Source" => Some(Self::Source),
            "License-File" => Some(Self::LicenseFile),
            _ => None,
        }
    }

    /// Return the canonical string representation of this keyword.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Description => "Description",
            Self::Version => "Version",
            Self::URL => "URL",
            Self::Requires => "Requires",
            Self::RequiresPrivate => "Requires.private",
            Self::Conflicts => "Conflicts",
            Self::Provides => "Provides",
            Self::Libs => "Libs",
            Self::LibsPrivate => "Libs.private",
            Self::Cflags => "Cflags",
            Self::CflagsPrivate => "Cflags.private",
            Self::License => "License",
            Self::Maintainer => "Maintainer",
            Self::Copyright => "Copyright",
            Self::Source => "Source",
            Self::LicenseFile => "License-File",
        }
    }
}

/// The result of parsing a single line in a `.pc` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    /// A variable definition: `key=value`.
    Variable { key: String, value: String },
    /// A field (keyword) declaration: `Keyword: value`.
    Field { keyword: Keyword, value: String },
    /// A comment line (starts with `#`).
    Comment(String),
    /// A blank / empty line.
    Blank,
}

/// Represents a fully parsed `.pc` file, with variables and fields separated.
#[derive(Debug, Clone)]
pub struct PcFile {
    /// The path this `.pc` file was loaded from, if any.
    pub path: Option<PathBuf>,

    /// The directory containing the `.pc` file (used for `pcfiledir`).
    pub pc_filedir: Option<PathBuf>,

    /// Ordered list of variable definitions (preserving insertion order).
    /// Later definitions of the same key override earlier ones.
    pub variables: Vec<(String, String)>,

    /// Field values, keyed by keyword.
    pub fields: HashMap<Keyword, String>,

    /// All directives in file order (useful for round-tripping / validation).
    pub directives: Vec<Directive>,
}

impl PcFile {
    /// Parse a `.pc` file from the given path.
    pub fn from_path(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                Error::PackageNotFound {
                    name: path.display().to_string(),
                }
            } else {
                Error::Io(e)
            }
        })?;

        let mut pc = Self::from_str(&content, path)?;
        let pc_filedir = path.parent().map(|p| p.to_path_buf());
        pc.path = Some(path.to_path_buf());
        pc.pc_filedir = pc_filedir.clone();

        // Insert the magic `pcfiledir` variable if not explicitly set.
        if !pc.variables.iter().any(|(k, _)| k == "pcfiledir")
            && let Some(ref dir) = pc_filedir
        {
            let dir_str = dir.to_string_lossy().to_string();
            pc.variables.insert(0, ("pcfiledir".to_string(), dir_str));
        }

        Ok(pc)
    }

    /// Parse a `.pc` file from a string, with `source_path` used for error messages.
    pub fn from_str(content: &str, source_path: &Path) -> Result<Self> {
        let mut variables = Vec::new();
        let mut fields = HashMap::new();
        let mut directives = Vec::new();

        let lines = LogicalLines::new(content);

        for (line_no, line) in lines.enumerate() {
            let line_no = line_no + 1; // 1-based

            let trimmed = line.trim();

            // Skip empty lines
            if trimmed.is_empty() {
                directives.push(Directive::Blank);
                continue;
            }

            // Skip comment lines
            if trimmed.starts_with('#') {
                directives.push(Directive::Comment(trimmed.to_string()));
                continue;
            }

            // Try to parse as a field (keyword: value) first, then as a variable (key=value).
            // pkgconf's parser checks for `:` (field) and `=` (variable).
            // A line like `Name: foo` is a field; `prefix=/usr` is a variable.
            // The distinction: fields have a keyword followed by `:`, variables use `=`.
            //
            // We need to be careful: `=` can appear in values, and `:` can appear in paths.
            // pkgconf's rule: scan for the first `:` or `=`. If `:` comes first (and the
            // key part matches a valid identifier), it's a field. Otherwise, if `=` comes
            // first, it's a variable.

            if let Some(directive) = parse_line(trimmed, source_path, line_no)? {
                match &directive {
                    Directive::Variable { key, value } => {
                        variables.push((key.clone(), value.clone()));
                    }
                    Directive::Field { keyword, value } => {
                        fields.insert(*keyword, value.clone());
                    }
                    _ => {}
                }
                directives.push(directive);
            }
        }

        Ok(Self {
            path: None,
            pc_filedir: None,
            variables,
            fields,
            directives,
        })
    }

    /// Look up a raw (unexpanded) variable value by name.
    pub fn get_variable_raw(&self, key: &str) -> Option<&str> {
        // Return the last definition (later overrides earlier)
        self.variables
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Look up a field value by keyword.
    pub fn get_field(&self, keyword: Keyword) -> Option<&str> {
        self.fields.get(&keyword).map(|s| s.as_str())
    }

    /// Get the `Name` field.
    pub fn name(&self) -> Option<&str> {
        self.get_field(Keyword::Name)
    }

    /// Get the `Description` field.
    pub fn description(&self) -> Option<&str> {
        self.get_field(Keyword::Description)
    }

    /// Get the `Version` field.
    pub fn version(&self) -> Option<&str> {
        self.get_field(Keyword::Version)
    }

    /// Get the `URL` field.
    pub fn url(&self) -> Option<&str> {
        self.get_field(Keyword::URL)
    }

    /// Get the `License` field.
    pub fn license(&self) -> Option<&str> {
        self.get_field(Keyword::License)
    }

    /// Get the `Source` field.
    pub fn source(&self) -> Option<&str> {
        self.get_field(Keyword::Source)
    }

    /// Collect all variable names defined in this file, in order.
    pub fn variable_names(&self) -> Vec<&str> {
        let mut seen = Vec::new();
        for (k, _) in &self.variables {
            if !seen.contains(&k.as_str()) {
                seen.push(k.as_str());
            }
        }
        seen
    }
}

/// Parse a single (logical) line as either a variable definition or a field declaration.
fn parse_line(line: &str, source_path: &Path, line_no: usize) -> Result<Option<Directive>> {
    // Find positions of first `=` and first `:` that could be delimiters.
    // We must be careful not to match `:` or `=` inside variable expansions `${}`.
    let first_eq = find_delimiter(line, '=');
    let first_colon = find_delimiter(line, ':');

    match (first_colon, first_eq) {
        // Both found: whichever comes first determines the type.
        (Some(ci), Some(ei)) => {
            if ci < ei {
                parse_field(line, ci, source_path, line_no)
            } else {
                parse_variable(line, ei)
            }
        }
        // Only colon: try as a field.
        (Some(ci), None) => parse_field(line, ci, source_path, line_no),
        // Only equals: it's a variable.
        (None, Some(ei)) => parse_variable(line, ei),
        // Neither: malformed line, ignore it (pkgconf silently skips unknown lines).
        (None, None) => Ok(None),
    }
}

/// Find the byte offset of the first occurrence of `delim` that is not inside a `${...}` block.
fn find_delimiter(line: &str, delim: char) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // Skip past the closing `}`
            i += 2;
            while i < bytes.len() && bytes[i] != b'}' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1; // skip `}`
            }
        } else if bytes[i] == delim as u8 {
            return Some(i);
        } else {
            i += 1;
        }
    }
    None
}

/// Parse a variable definition line given the position of `=`.
fn parse_variable(line: &str, eq_pos: usize) -> Result<Option<Directive>> {
    let key = line[..eq_pos].trim();
    let value = line[eq_pos + 1..].trim();

    // Variable names must be valid identifiers: non-empty, start with letter or underscore,
    // contain only alphanumerics, underscores, and dots (pkgconf allows dots for
    // cross-compilation personality variables).
    if key.is_empty() || !is_valid_variable_name(key) {
        return Ok(None);
    }

    Ok(Some(Directive::Variable {
        key: key.to_string(),
        value: value.to_string(),
    }))
}

/// Parse a field declaration line given the position of `:`.
fn parse_field(
    line: &str,
    colon_pos: usize,
    _source_path: &Path,
    _line_no: usize,
) -> Result<Option<Directive>> {
    let key = line[..colon_pos].trim();
    let value = line[colon_pos + 1..].trim();

    // Try to recognize the keyword. If it's not a known keyword, treat the line
    // as a variable definition if it contains `=` later, or skip it.
    if let Some(keyword) = Keyword::from_str(key) {
        Ok(Some(Directive::Field {
            keyword,
            value: value.to_string(),
        }))
    } else {
        // Unknown keyword. pkgconf ignores unknown keywords but still emits a warning.
        // Check if there's an `=` — if so, try to parse as a variable instead.
        if let Some(eq_pos) = find_delimiter(line, '=') {
            parse_variable(line, eq_pos)
        } else {
            // Truly unknown and not a variable either; skip.
            Ok(None)
        }
    }
}

/// Check whether `name` is a valid pkg-config variable name.
///
/// Valid names consist of ASCII letters, digits, underscores, dots, and hyphens,
/// and must start with a letter, underscore, or dot.
fn is_valid_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '.' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// Iterator that joins continuation lines (lines ending with `\`) into logical lines.
struct LogicalLines<'a> {
    lines: std::iter::Peekable<std::str::Lines<'a>>,
}

impl<'a> LogicalLines<'a> {
    fn new(content: &'a str) -> Self {
        Self {
            lines: content.lines().peekable(),
        }
    }
}

impl<'a> Iterator for LogicalLines<'a> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        let first = self.lines.next()?;

        // If the line ends with `\`, join with the next line.
        if let Some(stripped) = first.strip_suffix('\\') {
            let mut buf = String::from(stripped);
            while let Some(_next_line) = self.lines.peek() {
                let next = self.lines.next().unwrap();
                if let Some(stripped_next) = next.strip_suffix('\\') {
                    buf.push_str(stripped_next);
                } else {
                    buf.push_str(next);
                    break;
                }
            }
            Some(buf)
        } else {
            Some(first.to_string())
        }
    }
}

/// Expand variable references (`${varname}`) in a value string.
///
/// Uses the provided lookup function to resolve variable names. Variables can
/// be resolved from the package's own variables, global overrides, or
/// environment variables — the caller controls resolution order.
///
/// This function handles nested variable references (a variable value may itself
/// contain `${...}` references) by expanding recursively up to a depth limit.
///
/// # Errors
///
/// Returns an error if a variable is undefined (and `allow_undefined` is false)
/// or if the expansion depth limit (64) is exceeded (indicating a circular reference).
pub fn expand_variables<F>(value: &str, lookup: &F, allow_undefined: bool) -> Result<String>
where
    F: Fn(&str) -> Option<String>,
{
    expand_variables_depth(value, lookup, allow_undefined, 0)
}

fn expand_variables_depth<F>(
    value: &str,
    lookup: &F,
    allow_undefined: bool,
    depth: usize,
) -> Result<String>
where
    F: Fn(&str) -> Option<String>,
{
    const MAX_DEPTH: usize = 64;

    if depth > MAX_DEPTH {
        return Err(Error::CircularVariableReference {
            variable: value.to_string(),
        });
    }

    let bytes = value.as_bytes();
    let mut result = String::with_capacity(value.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // Find the closing `}`
            let start = i + 2;
            let mut end = start;
            let mut brace_depth = 1;
            while end < bytes.len() && brace_depth > 0 {
                if bytes[end] == b'{' {
                    brace_depth += 1;
                } else if bytes[end] == b'}' {
                    brace_depth -= 1;
                }
                if brace_depth > 0 {
                    end += 1;
                }
            }

            if brace_depth != 0 {
                // Unterminated `${`, emit literally
                result.push_str(&value[i..]);
                break;
            }

            let var_name = &value[start..end];

            match lookup(var_name) {
                Some(resolved) => {
                    // Recursively expand the resolved value
                    let expanded =
                        expand_variables_depth(&resolved, lookup, allow_undefined, depth + 1)?;
                    result.push_str(&expanded);
                }
                None => {
                    if allow_undefined {
                        // Leave unresolved references as empty string (pkgconf behaviour)
                        // or you could emit them literally; pkgconf emits empty.
                    } else {
                        return Err(Error::UndefinedVariable {
                            variable: var_name.to_string(),
                            context: value.to_string(),
                        });
                    }
                }
            }

            i = end + 1; // skip past `}`
        } else if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            // `$$` is an escape for a literal `$`
            result.push('$');
            i += 2;
        } else {
            let ch = value[i..].chars().next().unwrap();
            result.push(ch);
            i += ch.len_utf8();
        }
    }

    Ok(result)
}

/// Build a variable lookup function from a `PcFile` and optional global overrides.
///
/// Resolution order (matching pkgconf):
/// 1. Global variable overrides (from `--define-variable` or `PKG_CONFIG_*`)
/// 2. Package-level variables from the `.pc` file
/// 3. Built-in variables (`pcfiledir`, `pc_sysrootdir`, etc.)
pub fn build_lookup<'a>(
    pc: &'a PcFile,
    global_vars: &'a HashMap<String, String>,
    sysroot: Option<&'a str>,
) -> impl Fn(&str) -> Option<String> + 'a {
    move |name: &str| -> Option<String> {
        // 1. Global overrides first
        if let Some(val) = global_vars.get(name) {
            return Some(val.clone());
        }

        // 2. Package variables (last definition wins)
        if let Some(val) = pc.get_variable_raw(name) {
            return Some(val.to_string());
        }

        // 3. Built-in: pc_sysrootdir
        if name == "pc_sysrootdir" {
            return Some(sysroot.unwrap_or("").to_string());
        }

        None
    }
}

/// Resolve all variables in a `PcFile`, expanding `${...}` references.
///
/// Returns a map of variable name -> fully expanded value.
/// Variables are expanded in definition order so that earlier variables
/// are available for later ones.
pub fn resolve_variables(
    pc: &PcFile,
    global_vars: &HashMap<String, String>,
    sysroot: Option<&str>,
) -> Result<HashMap<String, String>> {
    let mut resolved: HashMap<String, String> = HashMap::new();

    // Insert built-ins
    resolved.insert(
        "pc_sysrootdir".to_string(),
        sysroot.unwrap_or("").to_string(),
    );
    if let Some(ref dir) = pc.pc_filedir {
        resolved.insert("pcfiledir".to_string(), dir.to_string_lossy().to_string());
    }

    // Process variables in definition order
    for (key, raw_value) in &pc.variables {
        // Global overrides take precedence
        if let Some(global_val) = global_vars.get(key) {
            resolved.insert(key.clone(), global_val.clone());
            continue;
        }

        let lookup = |name: &str| -> Option<String> {
            if let Some(gv) = global_vars.get(name) {
                return Some(gv.clone());
            }
            resolved.get(name).cloned()
        };

        let expanded = expand_variables(raw_value, &lookup, true)?;
        resolved.insert(key.clone(), expanded);
    }

    Ok(resolved)
}

/// Resolve a single field value by expanding variables using the resolved variable map.
pub fn resolve_field(field_value: &str, resolved_vars: &HashMap<String, String>) -> Result<String> {
    let lookup = |name: &str| -> Option<String> { resolved_vars.get(name).cloned() };
    expand_variables(field_value, &lookup, true)
}

/// Split a shell-like string into argv tokens.
///
/// Handles single-quoting, double-quoting, and backslash escapes,
/// matching pkgconf's `pkgconf_argv_split` behaviour.
pub fn argv_split(input: &str) -> Vec<String> {
    let mut args = Vec::new();
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
                        args.push(std::mem::take(&mut current));
                    }
                }
                _ => {
                    current.push(c);
                }
            }
        }
    }

    if !current.is_empty() {
        args.push(current);
    }

    args
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // PcFile parsing
    // -------------------------------------------------------------------------

    #[test]
    fn parse_simple_pc_file() {
        let content = "\
prefix=/usr
libdir=${prefix}/lib
includedir=${prefix}/include

Name: Foo
Description: A test library
Version: 1.2.3
Libs: -L${libdir} -lfoo
Cflags: -I${includedir}
";
        let pc = PcFile::from_str(content, Path::new("foo.pc")).unwrap();

        assert_eq!(pc.get_variable_raw("prefix"), Some("/usr"));
        assert_eq!(pc.get_variable_raw("libdir"), Some("${prefix}/lib"));
        assert_eq!(pc.get_variable_raw("includedir"), Some("${prefix}/include"));

        assert_eq!(pc.name(), Some("Foo"));
        assert_eq!(pc.description(), Some("A test library"));
        assert_eq!(pc.version(), Some("1.2.3"));
        assert_eq!(pc.get_field(Keyword::Libs), Some("-L${libdir} -lfoo"));
        assert_eq!(pc.get_field(Keyword::Cflags), Some("-I${includedir}"));
    }

    #[test]
    fn parse_comments_and_blanks() {
        let content = "\
# This is a comment
prefix=/usr

# Another comment

Name: Bar
Version: 0.1
Description: Bar library
";
        let pc = PcFile::from_str(content, Path::new("bar.pc")).unwrap();
        assert_eq!(pc.name(), Some("Bar"));
        assert_eq!(pc.version(), Some("0.1"));

        // Check that comments and blanks are preserved in directives
        let comment_count = pc
            .directives
            .iter()
            .filter(|d| matches!(d, Directive::Comment(_)))
            .count();
        assert_eq!(comment_count, 2);

        let blank_count = pc
            .directives
            .iter()
            .filter(|d| matches!(d, Directive::Blank))
            .count();
        assert!(blank_count >= 2);
    }

    #[test]
    fn parse_requires() {
        let content = "\
Name: Complex
Description: Complex pkg
Version: 2.0
Requires: glib-2.0 >= 2.50, gio-2.0
Requires.private: zlib
";
        let pc = PcFile::from_str(content, Path::new("complex.pc")).unwrap();
        assert_eq!(
            pc.get_field(Keyword::Requires),
            Some("glib-2.0 >= 2.50, gio-2.0")
        );
        assert_eq!(pc.get_field(Keyword::RequiresPrivate), Some("zlib"));
    }

    #[test]
    fn parse_multiline_continuation() {
        let content = "\
prefix=/usr
libdir=${prefix}/lib

Name: Multi
Description: Multi-line test
Version: 1.0
Libs: -L${libdir} \\
  -lfoo \\
  -lbar
";
        let pc = PcFile::from_str(content, Path::new("multi.pc")).unwrap();
        let libs = pc.get_field(Keyword::Libs).unwrap();
        assert!(libs.contains("-lfoo"));
        assert!(libs.contains("-lbar"));
        assert!(libs.contains("-L${libdir}"));
    }

    #[test]
    fn parse_variable_override_last_wins() {
        let content = "\
prefix=/usr
prefix=/opt

Name: Override
Description: test
Version: 1.0
";
        let pc = PcFile::from_str(content, Path::new("override.pc")).unwrap();
        assert_eq!(pc.get_variable_raw("prefix"), Some("/opt"));
    }

    #[test]
    fn parse_url_with_colon() {
        // URL field values contain colons, and variable values with paths may too.
        // This tests that the parser correctly identifies the field delimiter.
        let content = "\
Name: URLTest
Description: Has URL
Version: 1.0
URL: https://example.com/project
";
        let pc = PcFile::from_str(content, Path::new("url.pc")).unwrap();
        assert_eq!(pc.url(), Some("https://example.com/project"));
    }

    #[test]
    fn parse_empty_values() {
        let content = "\
prefix=
Name: Empty
Description:
Version: 1.0
Libs:
Cflags:
";
        let pc = PcFile::from_str(content, Path::new("empty.pc")).unwrap();
        assert_eq!(pc.get_variable_raw("prefix"), Some(""));
        assert_eq!(pc.description(), Some(""));
        assert_eq!(pc.get_field(Keyword::Libs), Some(""));
        assert_eq!(pc.get_field(Keyword::Cflags), Some(""));
    }

    #[test]
    fn parse_license_and_source() {
        let content = "\
Name: Licensed
Description: Has license info
Version: 1.0
License: MIT
Source: https://example.com/project
";
        let pc = PcFile::from_str(content, Path::new("licensed.pc")).unwrap();
        assert_eq!(pc.license(), Some("MIT"));
        assert_eq!(pc.source(), Some("https://example.com/project"));
    }

    #[test]
    fn parse_variable_names() {
        let content = "\
prefix=/usr
exec_prefix=${prefix}
libdir=${exec_prefix}/lib
includedir=${prefix}/include

Name: VarNames
Description: test
Version: 1.0
";
        let pc = PcFile::from_str(content, Path::new("varnames.pc")).unwrap();
        let names = pc.variable_names();
        assert_eq!(names, vec!["prefix", "exec_prefix", "libdir", "includedir"]);
    }

    // -------------------------------------------------------------------------
    // Variable expansion
    // -------------------------------------------------------------------------

    #[test]
    fn expand_no_variables() {
        let lookup = |_: &str| -> Option<String> { None };
        let result = expand_variables("hello world", &lookup, true).unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    fn expand_simple_variable() {
        let lookup = |name: &str| -> Option<String> {
            if name == "prefix" {
                Some("/usr".to_string())
            } else {
                None
            }
        };
        let result = expand_variables("${prefix}/lib", &lookup, true).unwrap();
        assert_eq!(result, "/usr/lib");
    }

    #[test]
    fn expand_multiple_variables() {
        let lookup = |name: &str| -> Option<String> {
            match name {
                "prefix" => Some("/usr".to_string()),
                "suffix" => Some("64".to_string()),
                _ => None,
            }
        };
        let result = expand_variables("${prefix}/lib${suffix}", &lookup, true).unwrap();
        assert_eq!(result, "/usr/lib64");
    }

    #[test]
    fn expand_nested_variables() {
        let vars: HashMap<String, String> = [
            ("prefix".to_string(), "/usr".to_string()),
            ("libdir".to_string(), "${prefix}/lib".to_string()),
        ]
        .into_iter()
        .collect();

        let lookup = |name: &str| -> Option<String> { vars.get(name).cloned() };
        let result = expand_variables("${libdir}/pkgconfig", &lookup, true).unwrap();
        assert_eq!(result, "/usr/lib/pkgconfig");
    }

    #[test]
    fn expand_dollar_dollar_escape() {
        let lookup = |_: &str| -> Option<String> { None };
        let result = expand_variables("cost is $$5", &lookup, true).unwrap();
        assert_eq!(result, "cost is $5");
    }

    #[test]
    fn expand_undefined_variable_allowed() {
        let lookup = |_: &str| -> Option<String> { None };
        let result = expand_variables("before${missing}after", &lookup, true).unwrap();
        assert_eq!(result, "beforeafter");
    }

    #[test]
    fn expand_undefined_variable_disallowed() {
        let lookup = |_: &str| -> Option<String> { None };
        let result = expand_variables("${missing}", &lookup, false);
        assert!(result.is_err());
    }

    #[test]
    fn expand_unterminated_variable() {
        let lookup = |_: &str| -> Option<String> { None };
        let result = expand_variables("${unclosed", &lookup, true).unwrap();
        assert_eq!(result, "${unclosed");
    }

    #[test]
    fn expand_circular_reference_detected() {
        let vars: HashMap<String, String> = [
            ("a".to_string(), "${b}".to_string()),
            ("b".to_string(), "${a}".to_string()),
        ]
        .into_iter()
        .collect();

        let lookup = |name: &str| -> Option<String> { vars.get(name).cloned() };
        let result = expand_variables("${a}", &lookup, true);
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // resolve_variables
    // -------------------------------------------------------------------------

    #[test]
    fn resolve_all_variables() {
        let content = "\
prefix=/usr
exec_prefix=${prefix}
libdir=${exec_prefix}/lib
includedir=${prefix}/include

Name: Resolve
Description: test
Version: 1.0
";
        let pc = PcFile::from_str(content, Path::new("resolve.pc")).unwrap();
        let global_vars = HashMap::new();
        let resolved = resolve_variables(&pc, &global_vars, None).unwrap();

        assert_eq!(resolved.get("prefix").unwrap(), "/usr");
        assert_eq!(resolved.get("exec_prefix").unwrap(), "/usr");
        assert_eq!(resolved.get("libdir").unwrap(), "/usr/lib");
        assert_eq!(resolved.get("includedir").unwrap(), "/usr/include");
    }

    #[test]
    fn resolve_with_global_override() {
        let content = "\
prefix=/usr
libdir=${prefix}/lib

Name: GlobalOverride
Description: test
Version: 1.0
";
        let pc = PcFile::from_str(content, Path::new("global.pc")).unwrap();
        let mut global_vars = HashMap::new();
        global_vars.insert("prefix".to_string(), "/opt".to_string());

        let resolved = resolve_variables(&pc, &global_vars, None).unwrap();
        assert_eq!(resolved.get("prefix").unwrap(), "/opt");
        assert_eq!(resolved.get("libdir").unwrap(), "/opt/lib");
    }

    #[test]
    fn resolve_with_sysroot() {
        let content = "\
prefix=${pc_sysrootdir}/usr
libdir=${prefix}/lib

Name: Sysroot
Description: test
Version: 1.0
";
        let pc = PcFile::from_str(content, Path::new("sysroot.pc")).unwrap();
        let global_vars = HashMap::new();
        let resolved = resolve_variables(&pc, &global_vars, Some("/mnt/target")).unwrap();

        assert_eq!(resolved.get("prefix").unwrap(), "/mnt/target/usr");
        assert_eq!(resolved.get("libdir").unwrap(), "/mnt/target/usr/lib");
    }

    #[test]
    fn resolve_field_value() {
        let mut resolved_vars = HashMap::new();
        resolved_vars.insert("libdir".to_string(), "/usr/lib".to_string());
        resolved_vars.insert("includedir".to_string(), "/usr/include".to_string());

        let libs = resolve_field("-L${libdir} -lfoo", &resolved_vars).unwrap();
        assert_eq!(libs, "-L/usr/lib -lfoo");

        let cflags = resolve_field("-I${includedir}/foo", &resolved_vars).unwrap();
        assert_eq!(cflags, "-I/usr/include/foo");
    }

    // -------------------------------------------------------------------------
    // argv_split
    // -------------------------------------------------------------------------

    #[test]
    fn argv_split_simple() {
        assert_eq!(
            argv_split("-I/usr/include -lfoo"),
            vec!["-I/usr/include", "-lfoo"]
        );
    }

    #[test]
    fn argv_split_quoted() {
        assert_eq!(
            argv_split(r#"-I"/path with spaces/include" -lfoo"#),
            vec!["-I/path with spaces/include", "-lfoo"]
        );
    }

    #[test]
    fn argv_split_single_quoted() {
        assert_eq!(
            argv_split("-I'/path with spaces/include' -lfoo"),
            vec!["-I/path with spaces/include", "-lfoo"]
        );
    }

    #[test]
    fn argv_split_backslash_escape() {
        assert_eq!(
            argv_split(r"-I/path\ with\ spaces/include -lfoo"),
            vec!["-I/path with spaces/include", "-lfoo"]
        );
    }

    #[test]
    fn argv_split_empty() {
        assert!(argv_split("").is_empty());
        assert!(argv_split("   ").is_empty());
    }

    #[test]
    fn argv_split_multiple_spaces() {
        assert_eq!(argv_split("  -lfoo   -lbar   "), vec!["-lfoo", "-lbar"]);
    }

    #[test]
    fn argv_split_tabs_and_newlines() {
        assert_eq!(
            argv_split("-lfoo\t-lbar\n-lbaz"),
            vec!["-lfoo", "-lbar", "-lbaz"]
        );
    }

    // -------------------------------------------------------------------------
    // Line parsing helpers
    // -------------------------------------------------------------------------

    #[test]
    fn is_valid_variable_name_test() {
        assert!(is_valid_variable_name("prefix"));
        assert!(is_valid_variable_name("exec_prefix"));
        assert!(is_valid_variable_name("_private"));
        assert!(is_valid_variable_name("lib.dir"));
        assert!(is_valid_variable_name("my-var"));

        assert!(!is_valid_variable_name(""));
        assert!(!is_valid_variable_name("123abc"));
        assert!(!is_valid_variable_name("-start"));
    }

    #[test]
    fn find_delimiter_skips_variable_refs() {
        // The `=` inside `${...}` should be ignored
        assert_eq!(find_delimiter("foo=${bar=baz}", '='), Some(3));
        // The `:` inside `${...}` should be ignored
        assert_eq!(find_delimiter("${proto://host}", ':'), None);
    }

    #[test]
    fn logical_lines_no_continuation() {
        let lines: Vec<_> = LogicalLines::new("a\nb\nc").collect();
        assert_eq!(lines, vec!["a", "b", "c"]);
    }

    #[test]
    fn logical_lines_with_continuation() {
        let lines: Vec<_> = LogicalLines::new("a \\\nb \\\nc").collect();
        assert_eq!(lines, vec!["a b c"]);
    }

    #[test]
    fn logical_lines_continuation_at_end() {
        let lines: Vec<_> = LogicalLines::new("a \\").collect();
        assert_eq!(lines, vec!["a "]);
    }

    #[test]
    fn logical_lines_mixed() {
        let lines: Vec<_> = LogicalLines::new("first\nsecond \\\ncontinued\nthird").collect();
        assert_eq!(lines, vec!["first", "second continued", "third"]);
    }

    // -------------------------------------------------------------------------
    // Edge cases
    // -------------------------------------------------------------------------

    #[test]
    fn parse_variable_with_equals_in_value() {
        let content = "\
CFLAGS=-DFOO=BAR -DBAZ=1

Name: EqVal
Description: test
Version: 1.0
";
        let pc = PcFile::from_str(content, Path::new("eqval.pc")).unwrap();
        assert_eq!(pc.get_variable_raw("CFLAGS"), Some("-DFOO=BAR -DBAZ=1"));
    }

    #[test]
    fn parse_provides_and_conflicts() {
        let content = "\
Name: Prov
Description: test
Version: 2.0
Provides: libfoo = 2.0
Conflicts: libbar < 1.0
";
        let pc = PcFile::from_str(content, Path::new("prov.pc")).unwrap();
        assert_eq!(pc.get_field(Keyword::Provides), Some("libfoo = 2.0"));
        assert_eq!(pc.get_field(Keyword::Conflicts), Some("libbar < 1.0"));
    }

    #[test]
    fn parse_libs_private() {
        let content = "\
Name: WithPrivate
Description: test
Version: 1.0
Libs: -lfoo
Libs.private: -lm -lpthread
";
        let pc = PcFile::from_str(content, Path::new("priv.pc")).unwrap();
        assert_eq!(pc.get_field(Keyword::Libs), Some("-lfoo"));
        assert_eq!(pc.get_field(Keyword::LibsPrivate), Some("-lm -lpthread"));
    }
}
