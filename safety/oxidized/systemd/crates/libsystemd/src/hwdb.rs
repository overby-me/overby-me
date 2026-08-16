//! Hardware database (hwdb) binary format reader.
//!
//! Reads the compiled hardware database (`hwdb.bin`) produced by
//! `systemd-hwdb update` or `udevadm hwdb --update`.  The binary
//! format is a compressed prefix trie mapping device modalias strings
//! to key=value property pairs.
//!
//! # Binary format (on‐disk)
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │  trie_header_f   (80 bytes, signature   │
//! │                    "KSLPHHRH")           │
//! ├─────────────────────────────────────────┤
//! │  trie nodes, child arrays, value arrays │
//! │  (nodes_len bytes)                      │
//! ├─────────────────────────────────────────┤
//! │  string table  (strings_len bytes)      │
//! │  null‐terminated, shared via offsets    │
//! └─────────────────────────────────────────┘
//! ```
//!
//! Each trie node has a prefix (path‐compressed characters shared by
//! all children), followed by an array of child entries (sorted by
//! character for binary search) and an array of value entries.
//!
//! Value entry keys that start with `' '` (space) are property names;
//! the space is stripped before returning the key to callers.
//!
//! # Lookup algorithm
//!
//! The lookup walks the trie character‐by‐character.  If a node's
//! prefix or a child edge contains glob characters (`*`, `?`, `[`),
//! the code switches to an `fnmatch`‐style recursive search that
//! reconstructs the full pattern from the trie path and matches it
//! against the query string.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// On‐disk structures (all little‐endian)
// ---------------------------------------------------------------------------

const HWDB_SIG: [u8; 8] = *b"KSLPHHRH";

/// Minimum file size: header must be fully readable.
const MIN_FILE_SIZE: usize = 80;

/// Standard search paths for hwdb.bin, checked in order.
const HWDB_BIN_PATHS: &[&str] = &[
    "/etc/systemd/hwdb/hwdb.bin",
    "/etc/udev/hwdb.bin",
    "/usr/lib/systemd/hwdb/hwdb.bin",
    "/usr/lib/udev/hwdb.bin",
    "/lib/udev/hwdb.bin",
];

// ---------------------------------------------------------------------------
// Helper: read little‐endian integers from a byte slice
// ---------------------------------------------------------------------------

#[inline]
fn read_u8(data: &[u8], off: usize) -> Option<u8> {
    data.get(off).copied()
}

#[inline]
#[allow(dead_code)]
fn read_u16_le(data: &[u8], off: usize) -> Option<u16> {
    let b = data.get(off..off + 2)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

#[inline]
#[allow(dead_code)]
fn read_u32_le(data: &[u8], off: usize) -> Option<u32> {
    let b = data.get(off..off + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[inline]
fn read_u64_le(data: &[u8], off: usize) -> Option<u64> {
    let b = data.get(off..off + 8)?;
    Some(u64::from_le_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}

/// Read a NUL‐terminated string starting at `off`.
fn read_cstr(data: &[u8], off: usize) -> Option<&str> {
    if off >= data.len() {
        return None;
    }
    let rest = &data[off..];
    let nul = rest.iter().position(|&b| b == 0)?;
    std::str::from_utf8(&rest[..nul]).ok()
}

// ---------------------------------------------------------------------------
// Parsed header
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct HwdbHeader {
    #[allow(dead_code)]
    tool_version: u64,
    file_size: u64,
    #[allow(dead_code)]
    header_size: u64,
    node_size: u64,
    child_entry_size: u64,
    value_entry_size: u64,
    nodes_root_off: u64,
    #[allow(dead_code)]
    nodes_len: u64,
    #[allow(dead_code)]
    strings_len: u64,
}

impl HwdbHeader {
    fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < MIN_FILE_SIZE {
            return None;
        }
        if data[..8] != HWDB_SIG {
            return None;
        }
        Some(HwdbHeader {
            tool_version: read_u64_le(data, 8)?,
            file_size: read_u64_le(data, 16)?,
            header_size: read_u64_le(data, 24)?,
            node_size: read_u64_le(data, 32)?,
            child_entry_size: read_u64_le(data, 40)?,
            value_entry_size: read_u64_le(data, 48)?,
            nodes_root_off: read_u64_le(data, 56)?,
            nodes_len: read_u64_le(data, 64)?,
            strings_len: read_u64_le(data, 72)?,
        })
    }
}

// ---------------------------------------------------------------------------
// Trie node helpers
// ---------------------------------------------------------------------------

/// Read a trie node's fields at the given file offset.
struct TrieNode {
    prefix_off: u64,
    children_count: u8,
    values_count: u64,
}

impl TrieNode {
    fn read(data: &[u8], off: usize) -> Option<Self> {
        if off + 24 > data.len() {
            return None;
        }
        Some(TrieNode {
            prefix_off: read_u64_le(data, off)?,
            children_count: read_u8(data, off + 8)?,
            values_count: read_u64_le(data, off + 16)?,
        })
    }
}

/// A child entry: the edge character and the offset of the child node.
#[derive(Clone, Copy)]
struct ChildEntry {
    c: u8,
    child_off: u64,
}

/// A value entry: offsets into the string table for the key and value.
struct ValueEntry {
    key_off: u64,
    value_off: u64,
}

// ---------------------------------------------------------------------------
// Hwdb — the public API
// ---------------------------------------------------------------------------

/// An opened hwdb binary database.
#[derive(Clone)]
pub struct Hwdb {
    data: Vec<u8>,
    header: HwdbHeader,
    /// Path from which the database was loaded.
    pub path: PathBuf,
}

impl std::fmt::Debug for Hwdb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hwdb")
            .field("path", &self.path)
            .field("file_size", &self.header.file_size)
            .field("tool_version", &self.header.tool_version)
            .finish()
    }
}

impl Hwdb {
    // -- Construction -------------------------------------------------------

    /// Open the hwdb from a specific file path.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref();
        let data = fs::read(path)?;
        Self::from_bytes(data, path.to_path_buf())
    }

    /// Open the first hwdb.bin found on the standard search paths.
    ///
    /// Also checks a NixOS package‐relative path derived from the
    /// running executable's location.
    pub fn open_default() -> io::Result<Self> {
        // Try NixOS package-relative path first (exe is in $out/lib/systemd/...)
        if let Ok(exe) = std::env::current_exe()
            && let Some(parent) = exe.parent()
        {
            for rel in &[
                "../hwdb/hwdb.bin",
                "../../hwdb/hwdb.bin",
                "../../../lib/udev/hwdb.bin",
            ] {
                let p = parent.join(rel);
                if p.is_file() {
                    match Self::open(&p) {
                        Ok(h) => return Ok(h),
                        Err(_) => continue,
                    }
                }
            }
        }

        for p in HWDB_BIN_PATHS {
            let path = Path::new(p);
            if path.is_file() {
                match Self::open(path) {
                    Ok(h) => return Ok(h),
                    Err(_) => continue,
                }
            }
        }

        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "hwdb.bin not found; run 'systemd-hwdb update'",
        ))
    }

    /// Construct from raw bytes (useful for testing).
    pub fn from_bytes(data: Vec<u8>, path: PathBuf) -> io::Result<Self> {
        let header = HwdbHeader::parse(&data)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid hwdb.bin header"))?;

        if data.len() != header.file_size as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "hwdb.bin file size mismatch: header says {} but file is {} bytes",
                    header.file_size,
                    data.len()
                ),
            ));
        }

        Ok(Hwdb { data, header, path })
    }

    // -- Lookup -------------------------------------------------------------

    /// Look up all properties matching a modalias string.
    ///
    /// Returns a map of property‐name → property‐value.  Property names
    /// have the leading space stripped (only entries whose raw key starts
    /// with `' '` are included, per the hwdb convention).
    pub fn lookup(&self, modalias: &str) -> BTreeMap<String, String> {
        let mut props: BTreeMap<String, String> = BTreeMap::new();
        let root_off = self.header.nodes_root_off as usize;
        let mut buf = String::with_capacity(256);
        self.trie_search(root_off, modalias.as_bytes(), 0, &mut buf, &mut props);
        props
    }

    /// Look up a single property value for the given modalias and key.
    pub fn get(&self, modalias: &str, key: &str) -> Option<String> {
        self.lookup(modalias).remove(key)
    }

    // -- Internal trie traversal --------------------------------------------

    /// The main trie search function, mirroring systemd's `trie_search_f`.
    ///
    /// Walks the trie character‐by‐character.  When a glob meta‐character
    /// is encountered (in a node prefix or as a child edge), delegates to
    /// `trie_fnmatch` for recursive pattern matching.
    fn trie_search(
        &self,
        mut node_off: usize,
        search: &[u8],
        mut search_idx: usize,
        buf: &mut String,
        props: &mut BTreeMap<String, String>,
    ) {
        loop {
            let node = match TrieNode::read(&self.data, node_off) {
                Some(n) => n,
                None => return,
            };

            // Match the node's prefix against the search string.
            if node.prefix_off != 0
                && let Some(prefix) = read_cstr(&self.data, node.prefix_off as usize)
            {
                for (pi, pc) in prefix.bytes().enumerate() {
                    if is_glob_char(pc) {
                        // Switch to fnmatch mode for the remainder.
                        self.trie_fnmatch(node_off, pi, buf, search, search_idx, props);
                        return;
                    }
                    if search_idx >= search.len() || pc != search[search_idx] {
                        return;
                    }
                    search_idx += 1;
                }
            }

            // At this node, also probe wildcard children *, ?, [
            for &wc in b"*?[" {
                if let Some(child) = self.child_lookup(node_off, &node, wc) {
                    let saved_len = buf.len();
                    buf.push(wc as char);
                    self.trie_fnmatch(child.child_off as usize, 0, buf, search, search_idx, props);
                    buf.truncate(saved_len);
                }
            }

            // If the search string is exhausted, collect values and return.
            if search_idx >= search.len() {
                self.collect_values(node_off, &node, props);
                return;
            }

            // Navigate to the child that matches the next search character.
            let ch = search[search_idx];
            match self.child_lookup(node_off, &node, ch) {
                Some(child) => {
                    node_off = child.child_off as usize;
                    search_idx += 1;
                }
                None => return,
            }
        }
    }

    /// Recursive fnmatch‐style trie traversal.
    ///
    /// Builds up the full pattern string in `buf` by appending the
    /// remaining prefix and child edges, then tests it against the
    /// search string with `fnmatch`.  Matching leaf nodes' values are
    /// collected.
    fn trie_fnmatch(
        &self,
        node_off: usize,
        prefix_start: usize,
        buf: &mut String,
        search: &[u8],
        search_idx: usize,
        props: &mut BTreeMap<String, String>,
    ) {
        let node = match TrieNode::read(&self.data, node_off) {
            Some(n) => n,
            None => return,
        };

        // Append the rest of this node's prefix to the pattern buffer.
        let prefix_len = if node.prefix_off != 0 {
            if let Some(prefix) = read_cstr(&self.data, node.prefix_off as usize) {
                let tail = &prefix[prefix_start..];
                buf.push_str(tail);
                tail.len()
            } else {
                0
            }
        } else {
            0
        };

        // Recurse into all children.
        let children = self.read_children(node_off, &node);
        for child in &children {
            let saved = buf.len();
            buf.push(child.c as char);
            self.trie_fnmatch(child.child_off as usize, 0, buf, search, search_idx, props);
            buf.truncate(saved);
        }

        // If this node has values, check whether the accumulated pattern
        // matches the (remaining) search string.
        if node.values_count > 0 {
            let search_tail = if search_idx <= search.len() {
                std::str::from_utf8(&search[search_idx..]).unwrap_or("")
            } else {
                ""
            };
            if fnmatch(buf, search_tail) {
                self.collect_values(node_off, &node, props);
            }
        }

        // Remove what we appended.
        let new_len = buf.len().saturating_sub(prefix_len);
        buf.truncate(new_len);
    }

    /// Collect key=value properties from a trie node.
    fn collect_values(
        &self,
        node_off: usize,
        node: &TrieNode,
        props: &mut BTreeMap<String, String>,
    ) {
        let child_size = self.header.child_entry_size as usize;
        let value_size = self.header.value_entry_size as usize;
        let node_size = self.header.node_size as usize;

        let values_base = node_off + node_size + (node.children_count as usize) * child_size;

        for vi in 0..node.values_count as usize {
            let voff = values_base + vi * value_size;
            if let Some(ve) = self.read_value_entry(voff)
                && let Some(raw_key) = read_cstr(&self.data, ve.key_off as usize)
                // Only include keys that start with a space.
                && let Some(key) = raw_key.strip_prefix(' ')
                && let Some(val) = read_cstr(&self.data, ve.value_off as usize)
            {
                props.insert(key.to_string(), val.to_string());
            }
        }
    }

    // -- Child / value access -----------------------------------------------

    /// Binary search for a child edge with character `c`.
    fn child_lookup(&self, node_off: usize, node: &TrieNode, c: u8) -> Option<ChildEntry> {
        let children = self.read_children(node_off, node);
        // Children are sorted by `c`, so we can binary search.
        children
            .binary_search_by_key(&c, |e| e.c)
            .ok()
            .map(|idx| children[idx])
    }

    /// Read all child entries for a node.
    fn read_children(&self, node_off: usize, node: &TrieNode) -> Vec<ChildEntry> {
        let node_size = self.header.node_size as usize;
        let child_size = self.header.child_entry_size as usize;
        let base = node_off + node_size;
        let count = node.children_count as usize;

        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let off = base + i * child_size;
            if let Some(ce) = self.read_child_entry(off) {
                out.push(ce);
            }
        }
        out
    }

    fn read_child_entry(&self, off: usize) -> Option<ChildEntry> {
        Some(ChildEntry {
            c: read_u8(&self.data, off)?,
            child_off: read_u64_le(&self.data, off + 8)?,
        })
    }

    fn read_value_entry(&self, off: usize) -> Option<ValueEntry> {
        Some(ValueEntry {
            key_off: read_u64_le(&self.data, off)?,
            value_off: read_u64_le(&self.data, off + 8)?,
        })
    }
}

// ---------------------------------------------------------------------------
// Glob / fnmatch implementation
// ---------------------------------------------------------------------------

/// Returns `true` if `c` is a glob meta‐character.
#[inline]
fn is_glob_char(c: u8) -> bool {
    matches!(c, b'*' | b'?' | b'[')
}

/// Simple `fnmatch(3)`‐compatible glob matching (no `FNM_NOESCAPE`).
///
/// Supports `*` (any sequence), `?` (any single char), and `[…]`
/// character classes (including negation with `!` or `^`).
pub fn fnmatch(pattern: &str, string: &str) -> bool {
    fnmatch_bytes(pattern.as_bytes(), string.as_bytes())
}

fn fnmatch_bytes(pat: &[u8], s: &[u8]) -> bool {
    let mut pi = 0usize;
    let mut si = 0usize;
    // For backtracking on `*`.
    let mut star_pi: Option<usize> = None;
    let mut star_si: usize = 0;

    while si < s.len() {
        if pi < pat.len() && pat[pi] == b'?' {
            pi += 1;
            si += 1;
            continue;
        }

        if pi < pat.len() && pat[pi] == b'[' {
            if let Some((matched, end)) = match_bracket(&pat[pi..], s[si])
                && matched
            {
                pi += end;
                si += 1;
                continue;
            }
            // Bracket didn't match — try backtrack
            if let Some(sp) = star_pi {
                pi = sp;
                star_si += 1;
                si = star_si;
                continue;
            }
            return false;
        }

        if pi < pat.len() && pat[pi] == b'*' {
            star_pi = Some(pi);
            star_si = si;
            pi += 1;
            continue;
        }

        if pi < pat.len() && pat[pi] == s[si] {
            pi += 1;
            si += 1;
            continue;
        }

        // Mismatch — backtrack to last `*`
        if let Some(sp) = star_pi {
            pi = sp + 1;
            star_si += 1;
            si = star_si;
            continue;
        }

        return false;
    }

    // Consume trailing `*`s
    while pi < pat.len() && pat[pi] == b'*' {
        pi += 1;
    }

    pi == pat.len()
}

/// Try to match a `[…]` bracket expression against a single byte.
///
/// Returns `Some((matched, bracket_len))` where `bracket_len` is the
/// number of bytes consumed from `pat` (including the closing `]`), or
/// `None` if the bracket expression is malformed.
fn match_bracket(pat: &[u8], ch: u8) -> Option<(bool, usize)> {
    debug_assert!(pat[0] == b'[');
    let mut i = 1;
    let negate = if i < pat.len() && (pat[i] == b'!' || pat[i] == b'^') {
        i += 1;
        true
    } else {
        false
    };

    let mut matched = false;
    let mut first = true;

    loop {
        if i >= pat.len() {
            return None; // unterminated bracket
        }
        if pat[i] == b']' && !first {
            i += 1;
            break;
        }
        first = false;

        let lo = pat[i];
        i += 1;

        // Range: [a-z]
        if i + 1 < pat.len() && pat[i] == b'-' && pat[i + 1] != b']' {
            let hi = pat[i + 1];
            i += 2;
            if ch >= lo && ch <= hi {
                matched = true;
            }
        } else if ch == lo {
            matched = true;
        }
    }

    Some((matched ^ negate, i))
}

// ---------------------------------------------------------------------------
// Hwdb builtin argument parser
// ---------------------------------------------------------------------------

/// Parsed arguments for the hwdb udev builtin.
#[derive(Debug, Clone, Default)]
pub struct HwdbBuiltinArgs {
    /// `--subsystem=SUB` — only consider parent devices in this subsystem.
    pub subsystem: Option<String>,
    /// `--filter=PATTERN` — only include properties whose key matches this glob.
    pub filter: Option<String>,
    /// `--device=DEVID` — look up a different source device.
    pub device: Option<String>,
    /// `--lookup-prefix=PFX` — prepend this prefix to the modalias.
    pub prefix: Option<String>,
    /// Positional argument — explicit modalias to look up.
    pub modalias: Option<String>,
}

impl HwdbBuiltinArgs {
    /// Parse the argument string as passed to `IMPORT{builtin}="hwdb …"`.
    ///
    /// The string starts with `"hwdb"` followed by optional flags.
    pub fn parse(args: &str) -> Self {
        let mut result = HwdbBuiltinArgs::default();

        // Skip the "hwdb" command name itself.
        let mut parts = args.split_whitespace().peekable();
        if let Some(first) = parts.peek()
            && *first == "hwdb"
        {
            parts.next();
        }

        while let Some(arg) = parts.next() {
            if let Some(val) = arg.strip_prefix("--subsystem=") {
                result.subsystem = Some(val.to_string());
            } else if arg == "--subsystem" || arg == "-s" {
                if let Some(val) = parts.next() {
                    result.subsystem = Some(val.to_string());
                }
            } else if let Some(val) = arg.strip_prefix("--filter=") {
                result.filter = Some(val.to_string());
            } else if arg == "--filter" || arg == "-f" {
                if let Some(val) = parts.next() {
                    result.filter = Some(val.to_string());
                }
            } else if let Some(val) = arg.strip_prefix("--device=") {
                result.device = Some(val.to_string());
            } else if arg == "--device" || arg == "-d" {
                if let Some(val) = parts.next() {
                    result.device = Some(val.to_string());
                }
            } else if let Some(val) = arg.strip_prefix("--lookup-prefix=") {
                result.prefix = Some(val.to_string());
            } else if arg == "--lookup-prefix" || arg == "-p" {
                if let Some(val) = parts.next() {
                    result.prefix = Some(val.to_string());
                }
            } else if !arg.starts_with('-') {
                // Positional: explicit modalias
                result.modalias = Some(arg.to_string());
            }
        }

        result
    }
}

/// Filter properties by a glob pattern on the key.
pub fn filter_properties(
    props: &BTreeMap<String, String>,
    filter: Option<&str>,
) -> BTreeMap<String, String> {
    match filter {
        None => props.clone(),
        Some(pat) => props
            .iter()
            .filter(|(k, _)| fnmatch(pat, k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- fnmatch tests ------------------------------------------------------

    #[test]
    fn test_fnmatch_exact() {
        assert!(fnmatch("hello", "hello"));
        assert!(!fnmatch("hello", "world"));
    }

    #[test]
    fn test_fnmatch_star() {
        assert!(fnmatch("*", "anything"));
        assert!(fnmatch("*", ""));
        assert!(fnmatch("he*lo", "hello"));
        assert!(fnmatch("ab*cd", "abcd"));
        assert!(fnmatch("he*lo", "heXXXlo"));
        assert!(!fnmatch("he*lo", "heXXXla"));
        assert!(fnmatch("*foo*", "barfoobar"));
    }

    #[test]
    fn test_fnmatch_question() {
        assert!(fnmatch("h?llo", "hello"));
        assert!(!fnmatch("h?llo", "hllo"));
        assert!(!fnmatch("h?llo", "heello"));
    }

    #[test]
    fn test_fnmatch_bracket() {
        assert!(fnmatch("[abc]", "a"));
        assert!(fnmatch("[abc]", "b"));
        assert!(!fnmatch("[abc]", "d"));
        assert!(fnmatch("[a-z]", "m"));
        assert!(!fnmatch("[a-z]", "A"));
    }

    #[test]
    fn test_fnmatch_bracket_negate() {
        assert!(!fnmatch("[!abc]", "a"));
        assert!(fnmatch("[!abc]", "d"));
        assert!(!fnmatch("[^abc]", "b"));
        assert!(fnmatch("[^abc]", "z"));
    }

    #[test]
    fn test_fnmatch_combined() {
        assert!(fnmatch("usb:v04D9p*", "usb:v04D9p1234"));
        assert!(fnmatch("usb:v04D9p*", "usb:v04D9p"));
        assert!(!fnmatch("usb:v04D9p*", "usb:v04DAp1234"));
        assert!(fnmatch(
            "evdev:input:b0003v????p????*",
            "evdev:input:b0003v04D9p0024abc"
        ));
        assert!(fnmatch(
            "mouse:*:name:*Logitech*:",
            "mouse:usb:name:My Logitech Mouse:"
        ));
    }

    #[test]
    fn test_fnmatch_trailing_star() {
        assert!(fnmatch("abc*", "abc"));
        assert!(fnmatch("abc*", "abcdef"));
        assert!(!fnmatch("abc*", "ab"));
    }

    #[test]
    fn test_fnmatch_empty() {
        assert!(fnmatch("", ""));
        assert!(!fnmatch("", "x"));
        assert!(fnmatch("*", ""));
    }

    #[test]
    fn test_fnmatch_multiple_stars() {
        assert!(fnmatch("*a*b*c*", "xaxbxcx"));
        assert!(fnmatch("*a*b*c*", "abc"));
        assert!(!fnmatch("*a*b*c*", "xaxbx"));
    }

    #[test]
    fn test_fnmatch_bracket_range_edge() {
        assert!(fnmatch("[0-9]", "0"));
        assert!(fnmatch("[0-9]", "9"));
        assert!(!fnmatch("[0-9]", "a"));
        assert!(fnmatch("[A-Z]", "M"));
        assert!(!fnmatch("[A-Z]", "m"));
    }

    // -- match_bracket tests ------------------------------------------------

    #[test]
    fn test_match_bracket_simple() {
        assert_eq!(match_bracket(b"[abc]", b'a'), Some((true, 5)));
        assert_eq!(match_bracket(b"[abc]", b'd'), Some((false, 5)));
    }

    #[test]
    fn test_match_bracket_range() {
        assert_eq!(match_bracket(b"[a-z]", b'm'), Some((true, 5)));
        assert_eq!(match_bracket(b"[a-z]", b'A'), Some((false, 5)));
    }

    #[test]
    fn test_match_bracket_negate() {
        assert_eq!(match_bracket(b"[!a]", b'a'), Some((false, 4)));
        assert_eq!(match_bracket(b"[!a]", b'b'), Some((true, 4)));
    }

    #[test]
    fn test_match_bracket_unterminated() {
        assert_eq!(match_bracket(b"[abc", b'a'), None);
    }

    // -- HwdbBuiltinArgs tests ----------------------------------------------

    #[test]
    fn test_parse_args_empty() {
        let a = HwdbBuiltinArgs::parse("hwdb");
        assert!(a.subsystem.is_none());
        assert!(a.filter.is_none());
        assert!(a.device.is_none());
        assert!(a.prefix.is_none());
        assert!(a.modalias.is_none());
    }

    #[test]
    fn test_parse_args_subsystem_equals() {
        let a = HwdbBuiltinArgs::parse("hwdb --subsystem=usb");
        assert_eq!(a.subsystem.as_deref(), Some("usb"));
    }

    #[test]
    fn test_parse_args_subsystem_space() {
        let a = HwdbBuiltinArgs::parse("hwdb --subsystem input");
        assert_eq!(a.subsystem.as_deref(), Some("input"));
    }

    #[test]
    fn test_parse_args_short_subsystem() {
        let a = HwdbBuiltinArgs::parse("hwdb -s pci");
        assert_eq!(a.subsystem.as_deref(), Some("pci"));
    }

    #[test]
    fn test_parse_args_filter() {
        let a = HwdbBuiltinArgs::parse("hwdb --filter=ID_INPUT*");
        assert_eq!(a.filter.as_deref(), Some("ID_INPUT*"));
    }

    #[test]
    fn test_parse_args_device() {
        let a = HwdbBuiltinArgs::parse("hwdb --device=c13:0");
        assert_eq!(a.device.as_deref(), Some("c13:0"));
    }

    #[test]
    fn test_parse_args_prefix() {
        let a = HwdbBuiltinArgs::parse("hwdb --lookup-prefix=evdev:");
        assert_eq!(a.prefix.as_deref(), Some("evdev:"));
    }

    #[test]
    fn test_parse_args_positional() {
        let a = HwdbBuiltinArgs::parse("hwdb usb:v04D9p1234");
        assert_eq!(a.modalias.as_deref(), Some("usb:v04D9p1234"));
    }

    #[test]
    fn test_parse_args_combined() {
        let a =
            HwdbBuiltinArgs::parse("hwdb --subsystem=usb --filter=ID_MODEL* --lookup-prefix=usb:");
        assert_eq!(a.subsystem.as_deref(), Some("usb"));
        assert_eq!(a.filter.as_deref(), Some("ID_MODEL*"));
        assert_eq!(a.prefix.as_deref(), Some("usb:"));
    }

    #[test]
    fn test_parse_args_without_hwdb_prefix() {
        // If someone passes just the args without "hwdb" as first word.
        let a = HwdbBuiltinArgs::parse("--subsystem=input");
        assert_eq!(a.subsystem.as_deref(), Some("input"));
    }

    // -- filter_properties tests --------------------------------------------

    #[test]
    fn test_filter_properties_none() {
        let mut m = BTreeMap::new();
        m.insert("A".into(), "1".into());
        m.insert("B".into(), "2".into());
        let f = filter_properties(&m, None);
        assert_eq!(f.len(), 2);
    }

    #[test]
    fn test_filter_properties_glob() {
        let mut m = BTreeMap::new();
        m.insert("ID_INPUT".into(), "1".into());
        m.insert("ID_INPUT_KEY".into(), "1".into());
        m.insert("ID_VENDOR".into(), "Foo".into());
        let f = filter_properties(&m, Some("ID_INPUT*"));
        assert_eq!(f.len(), 2);
        assert!(f.contains_key("ID_INPUT"));
        assert!(f.contains_key("ID_INPUT_KEY"));
        assert!(!f.contains_key("ID_VENDOR"));
    }

    #[test]
    fn test_filter_properties_no_match() {
        let mut m = BTreeMap::new();
        m.insert("FOO".into(), "1".into());
        let f = filter_properties(&m, Some("BAR*"));
        assert!(f.is_empty());
    }

    // -- Header parsing tests -----------------------------------------------

    #[test]
    fn test_header_parse_too_short() {
        assert!(HwdbHeader::parse(&[0u8; 10]).is_none());
    }

    #[test]
    fn test_header_parse_bad_sig() {
        let mut data = vec![0u8; 80];
        data[..8].copy_from_slice(b"BADMAGIC");
        assert!(HwdbHeader::parse(&data).is_none());
    }

    #[test]
    fn test_header_parse_valid() {
        let data = build_minimal_hwdb(&[]);
        let h = HwdbHeader::parse(&data).unwrap();
        assert_eq!(h.header_size, 80);
        assert_eq!(h.node_size, 24);
        assert_eq!(h.child_entry_size, 16);
        assert_eq!(h.value_entry_size, 16);
    }

    // -- Trie reading tests -------------------------------------------------

    #[test]
    fn test_read_cstr_basic() {
        let data = b"hello\0world\0";
        assert_eq!(read_cstr(data, 0), Some("hello"));
        assert_eq!(read_cstr(data, 6), Some("world"));
    }

    #[test]
    fn test_read_cstr_out_of_bounds() {
        let data = b"hi\0";
        assert_eq!(read_cstr(data, 100), None);
    }

    #[test]
    fn test_read_cstr_empty() {
        let data = b"\0rest";
        assert_eq!(read_cstr(data, 0), Some(""));
    }

    // -- Synthetic hwdb for integration tests --------------------------------

    /// Build a minimal hwdb.bin in memory with the given (pattern, key, value) triples.
    ///
    /// This constructs a trivially simple trie: a root node with one child per
    /// distinct first character, leading to a chain of character nodes.
    /// Each full pattern path terminates with a node that holds values.
    ///
    /// This is not the most efficient trie layout but it correctly exercises
    /// the reader.
    fn build_minimal_hwdb(entries: &[(&str, &str, &str)]) -> Vec<u8> {
        // We build:
        //   - Header (80 bytes)
        //   - Nodes section
        //   - Strings section
        //
        // For simplicity, each "entry" is a full pattern → value.
        // We create a naive trie: the root node, with one child chain per entry.

        let header_size: u64 = 80;
        let node_size: u64 = 24;
        let child_entry_size: u64 = 16;
        let value_entry_size: u64 = 16;

        // String table: collect all strings we need.
        let mut strings = Vec::<u8>::new();
        let mut string_offsets = std::collections::HashMap::<String, u64>::new();

        // We need a "base" for string offsets.  Strings will be appended
        // at the end.  We'll figure out the exact offset after we know the
        // nodes section size.  For now, record relative positions.
        let mut add_string = |strings: &mut Vec<u8>, s: &str| -> usize {
            if let Some(&off) = string_offsets.get(s) {
                return off as usize;
            }
            let rel = strings.len();
            strings.extend_from_slice(s.as_bytes());
            strings.push(0);
            string_offsets.insert(s.to_string(), rel as u64);
            rel
        };

        // Pre-add the empty prefix string.
        let _empty_prefix_rel = add_string(&mut strings, "");

        // Plan the trie structure.
        //
        // Approach: build a simple recursive trie in memory, then serialize.

        struct TrieBuilder {
            // maps edge-char → child index
            children: Vec<(u8, usize)>,
            // values: (key_str, val_str) — raw with leading space on key
            values: Vec<(String, String)>,
            // prefix string (for path compression)
            prefix: String,
        }

        let mut nodes: Vec<TrieBuilder> = Vec::new();

        // Create root node.
        nodes.push(TrieBuilder {
            children: Vec::new(),
            values: Vec::new(),
            prefix: String::new(),
        });

        // Insert each entry as a chain from root.
        for &(pattern, key, value) in entries {
            // Walk/create nodes for each character.
            let mut cur = 0usize; // root

            // We store the pattern as individual character edges (no prefix compression
            // in this simple builder).
            for ch in pattern.bytes() {
                let existing = nodes[cur].children.iter().find(|&&(c, _)| c == ch);
                if let Some(&(_, child_idx)) = existing {
                    cur = child_idx;
                } else {
                    let new_idx = nodes.len();
                    nodes.push(TrieBuilder {
                        children: Vec::new(),
                        values: Vec::new(),
                        prefix: String::new(),
                    });
                    nodes[cur].children.push((ch, new_idx));
                    // Keep children sorted.
                    nodes[cur].children.sort_by_key(|&(c, _)| c);
                    cur = new_idx;
                }
            }

            // Add the value at the terminal node.
            // Key must start with a space (hwdb convention).
            nodes[cur]
                .values
                .push((format!(" {}", key), value.to_string()));
        }

        // Now serialize.  First pass: compute sizes to determine string base offset.
        // Each node: node_size + children_count * child_entry_size + values_count * value_entry_size
        let mut total_nodes_bytes: usize = 0;
        for n in &nodes {
            total_nodes_bytes += node_size as usize
                + n.children.len() * child_entry_size as usize
                + n.values.len() * value_entry_size as usize;
        }

        let strings_base = header_size as usize + total_nodes_bytes;

        // Fixup string offsets: add strings_base.
        // Re-add all strings with the correct base.
        // Actually, our `string_offsets` are relative.  We need to add strings_base.
        // Let's rebuild the string table with absolute offsets.

        let mut strings_final = Vec::<u8>::new();
        let mut string_offsets_abs = std::collections::HashMap::<String, u64>::new();

        let add_string_abs = |strings: &mut Vec<u8>,
                              offsets: &mut std::collections::HashMap<String, u64>,
                              s: &str|
         -> u64 {
            if let Some(&off) = offsets.get(s) {
                return off;
            }
            let off = strings_base as u64 + strings.len() as u64;
            strings.extend_from_slice(s.as_bytes());
            strings.push(0);
            offsets.insert(s.to_string(), off);
            off
        };

        // Pre-add empty string.
        let _empty_off = add_string_abs(&mut strings_final, &mut string_offsets_abs, "");

        // Add all prefixes and value strings.
        for n in &nodes {
            if !n.prefix.is_empty() {
                add_string_abs(&mut strings_final, &mut string_offsets_abs, &n.prefix);
            }
            for (k, v) in &n.values {
                add_string_abs(&mut strings_final, &mut string_offsets_abs, k);
                add_string_abs(&mut strings_final, &mut string_offsets_abs, v);
            }
        }

        // Second pass: assign offsets to nodes.
        let mut node_offsets = Vec::<usize>::new();
        let mut off = header_size as usize;
        for n in &nodes {
            node_offsets.push(off);
            off += node_size as usize
                + n.children.len() * child_entry_size as usize
                + n.values.len() * value_entry_size as usize;
        }

        let file_size = strings_base + strings_final.len();
        let root_off = node_offsets[0];

        // Write header.
        let mut out = Vec::<u8>::with_capacity(file_size);

        // signature
        out.extend_from_slice(&HWDB_SIG);
        // tool_version
        out.extend_from_slice(&1u64.to_le_bytes());
        // file_size
        out.extend_from_slice(&(file_size as u64).to_le_bytes());
        // header_size
        out.extend_from_slice(&header_size.to_le_bytes());
        // node_size
        out.extend_from_slice(&node_size.to_le_bytes());
        // child_entry_size
        out.extend_from_slice(&child_entry_size.to_le_bytes());
        // value_entry_size
        out.extend_from_slice(&value_entry_size.to_le_bytes());
        // nodes_root_off
        out.extend_from_slice(&(root_off as u64).to_le_bytes());
        // nodes_len
        out.extend_from_slice(&(total_nodes_bytes as u64).to_le_bytes());
        // strings_len
        out.extend_from_slice(&(strings_final.len() as u64).to_le_bytes());

        assert_eq!(out.len(), header_size as usize);

        // Write nodes.
        for n in nodes.iter() {
            let prefix_off = if n.prefix.is_empty() {
                0u64
            } else {
                *string_offsets_abs.get(&n.prefix).unwrap()
            };

            // trie_node_f
            out.extend_from_slice(&prefix_off.to_le_bytes()); // prefix_off
            out.push(n.children.len() as u8); // children_count
            out.extend_from_slice(&[0u8; 7]); // padding
            out.extend_from_slice(&(n.values.len() as u64).to_le_bytes()); // values_count

            // child entries
            for &(c, child_idx) in &n.children {
                out.push(c); // c
                out.extend_from_slice(&[0u8; 7]); // padding
                out.extend_from_slice(&(node_offsets[child_idx] as u64).to_le_bytes()); // child_off
            }

            // value entries
            for (k, v) in &n.values {
                let key_off = *string_offsets_abs.get(k.as_str()).unwrap();
                let val_off = *string_offsets_abs.get(v.as_str()).unwrap();
                out.extend_from_slice(&key_off.to_le_bytes());
                out.extend_from_slice(&val_off.to_le_bytes());
            }
        }

        assert_eq!(out.len(), strings_base);

        // Write strings.
        out.extend_from_slice(&strings_final);

        assert_eq!(out.len(), file_size);

        out
    }

    #[test]
    fn test_hwdb_empty() {
        let data = build_minimal_hwdb(&[]);
        let hwdb = Hwdb::from_bytes(data, PathBuf::from("test.bin")).unwrap();
        let props = hwdb.lookup("anything");
        assert!(props.is_empty());
    }

    #[test]
    fn test_hwdb_exact_match() {
        let data = build_minimal_hwdb(&[("mouse:usb:v1234p5678", "ID_INPUT_MOUSE", "1")]);
        let hwdb = Hwdb::from_bytes(data, PathBuf::from("test.bin")).unwrap();

        let props = hwdb.lookup("mouse:usb:v1234p5678");
        assert_eq!(props.get("ID_INPUT_MOUSE"), Some(&"1".to_string()));
    }

    #[test]
    fn test_hwdb_no_match() {
        let data = build_minimal_hwdb(&[("mouse:usb:v1234p5678", "ID_INPUT_MOUSE", "1")]);
        let hwdb = Hwdb::from_bytes(data, PathBuf::from("test.bin")).unwrap();

        let props = hwdb.lookup("mouse:usb:vAAAApBBBB");
        assert!(props.is_empty());
    }

    #[test]
    fn test_hwdb_prefix_match() {
        // A pattern without `*` only matches exactly — a longer search
        // string does NOT match.  To match longer strings the hwdb entry
        // must end with `*`.
        let data = build_minimal_hwdb(&[("mouse:usb", "ID_INPUT", "1")]);
        let hwdb = Hwdb::from_bytes(data, PathBuf::from("test.bin")).unwrap();

        // Exact match
        let props = hwdb.lookup("mouse:usb");
        assert_eq!(props.get("ID_INPUT"), Some(&"1".to_string()));

        // Longer string does NOT match (no trailing wildcard in pattern)
        let props2 = hwdb.lookup("mouse:usb:v1234");
        assert!(props2.is_empty());

        // With a wildcard suffix, longer strings match
        let data2 = build_minimal_hwdb(&[("mouse:usb*", "ID_INPUT", "1")]);
        let hwdb2 = Hwdb::from_bytes(data2, PathBuf::from("test.bin")).unwrap();
        let props3 = hwdb2.lookup("mouse:usb:v1234");
        assert_eq!(props3.get("ID_INPUT"), Some(&"1".to_string()));
    }

    #[test]
    fn test_hwdb_wildcard_match() {
        let data = build_minimal_hwdb(&[("mouse:*", "ID_INPUT_MOUSE", "1")]);
        let hwdb = Hwdb::from_bytes(data, PathBuf::from("test.bin")).unwrap();

        // The * child at the end should match via fnmatch
        let props = hwdb.lookup("mouse:usb:v1234");
        assert_eq!(props.get("ID_INPUT_MOUSE"), Some(&"1".to_string()));

        // Also matches mouse: with nothing after
        let props2 = hwdb.lookup("mouse:");
        assert_eq!(props2.get("ID_INPUT_MOUSE"), Some(&"1".to_string()));
    }

    #[test]
    fn test_hwdb_multiple_entries() {
        let data = build_minimal_hwdb(&[
            ("mouse:usb:v1234", "ID_VENDOR", "Acme"),
            ("mouse:usb:v1234", "ID_MODEL", "Mouse"),
            ("keyboard:usb:v5678", "ID_INPUT_KEYBOARD", "1"),
        ]);
        let hwdb = Hwdb::from_bytes(data, PathBuf::from("test.bin")).unwrap();

        let props = hwdb.lookup("mouse:usb:v1234");
        assert_eq!(props.get("ID_VENDOR"), Some(&"Acme".to_string()));
        assert_eq!(props.get("ID_MODEL"), Some(&"Mouse".to_string()));
        assert!(!props.contains_key("ID_INPUT_KEYBOARD"));

        let props2 = hwdb.lookup("keyboard:usb:v5678");
        assert_eq!(props2.get("ID_INPUT_KEYBOARD"), Some(&"1".to_string()));
        assert!(!props2.contains_key("ID_VENDOR"));
    }

    #[test]
    fn test_hwdb_get_single() {
        let data = build_minimal_hwdb(&[
            ("usb:v04D9p0024", "ID_VENDOR", "Holtek"),
            ("usb:v04D9p0024", "ID_MODEL", "Keyboard"),
        ]);
        let hwdb = Hwdb::from_bytes(data, PathBuf::from("test.bin")).unwrap();

        assert_eq!(
            hwdb.get("usb:v04D9p0024", "ID_VENDOR"),
            Some("Holtek".into())
        );
        assert_eq!(hwdb.get("usb:v04D9p0024", "NONEXISTENT"), None);
    }

    #[test]
    fn test_hwdb_shared_prefix() {
        // Two entries with a shared prefix: "usb:v04D9p0024" and "usb:v04D9p0025"
        let data = build_minimal_hwdb(&[
            ("usb:v04D9p0024", "ID_MODEL", "Keyboard24"),
            ("usb:v04D9p0025", "ID_MODEL", "Keyboard25"),
        ]);
        let hwdb = Hwdb::from_bytes(data, PathBuf::from("test.bin")).unwrap();

        assert_eq!(
            hwdb.get("usb:v04D9p0024", "ID_MODEL"),
            Some("Keyboard24".into())
        );
        assert_eq!(
            hwdb.get("usb:v04D9p0025", "ID_MODEL"),
            Some("Keyboard25".into())
        );
    }

    #[test]
    fn test_hwdb_value_override() {
        // Later entries for the same pattern + key should override earlier ones
        // (the trie builder puts them on the same node, and our collect_values
        //  inserts into a BTreeMap where last-write-wins for duplicate keys).
        let data = build_minimal_hwdb(&[("usb:v1234", "KEY", "old"), ("usb:v1234", "KEY", "new")]);
        let hwdb = Hwdb::from_bytes(data, PathBuf::from("test.bin")).unwrap();
        assert_eq!(hwdb.get("usb:v1234", "KEY"), Some("new".into()));
    }

    #[test]
    fn test_hwdb_file_size_mismatch() {
        let mut data = build_minimal_hwdb(&[]);
        // Truncate to cause a mismatch.
        data.pop();
        let result = Hwdb::from_bytes(data, PathBuf::from("test.bin"));
        assert!(result.is_err());
    }

    #[test]
    fn test_hwdb_open_nonexistent() {
        let result = Hwdb::open("/nonexistent/path/hwdb.bin");
        assert!(result.is_err());
    }

    // -- Integration with real hwdb.bin (if available) ----------------------

    #[test]
    fn test_hwdb_open_default_no_crash() {
        // This test just ensures we don't panic when trying to open.
        // It's OK if the file doesn't exist (CI etc.).
        let _ = Hwdb::open_default();
    }

    #[test]
    fn test_hwdb_real_lookup_no_crash() {
        if let Ok(hwdb) = Hwdb::open_default() {
            // Query something common — we don't assert the result because
            // it depends on the installed hwdb, but it must not panic.
            let _ = hwdb.lookup("usb:v1D6Bp0001");
            let _ = hwdb.lookup("evdev:input:b0003v04D9p0024");
            let _ = hwdb.lookup("nonexistent:device");
        }
    }

    // -- LE integer helpers -------------------------------------------------

    #[test]
    fn test_read_u8_basic() {
        assert_eq!(read_u8(&[0x42], 0), Some(0x42));
        assert_eq!(read_u8(&[0x42], 1), None);
    }

    #[test]
    fn test_read_u16_le_basic() {
        assert_eq!(read_u16_le(&[0x34, 0x12], 0), Some(0x1234));
    }

    #[test]
    fn test_read_u32_le_basic() {
        assert_eq!(read_u32_le(&[0x78, 0x56, 0x34, 0x12], 0), Some(0x12345678));
    }

    #[test]
    fn test_read_u64_le_basic() {
        assert_eq!(
            read_u64_le(&[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], 0),
            Some(1)
        );
    }

    // -- TrieNode reading ---------------------------------------------------

    #[test]
    fn test_trie_node_read_too_short() {
        assert!(TrieNode::read(&[0u8; 10], 0).is_none());
    }

    #[test]
    fn test_trie_node_read_valid() {
        // 24 bytes: prefix_off(8) + children_count(1) + padding(7) + values_count(8)
        let mut buf = vec![0u8; 24];
        // prefix_off = 100
        buf[0..8].copy_from_slice(&100u64.to_le_bytes());
        // children_count = 3
        buf[8] = 3;
        // values_count = 5
        buf[16..24].copy_from_slice(&5u64.to_le_bytes());

        let node = TrieNode::read(&buf, 0).unwrap();
        assert_eq!(node.prefix_off, 100);
        assert_eq!(node.children_count, 3);
        assert_eq!(node.values_count, 5);
    }

    // -- Debug/Display checks -----------------------------------------------

    #[test]
    fn test_hwdb_debug() {
        let data = build_minimal_hwdb(&[]);
        let hwdb = Hwdb::from_bytes(data, PathBuf::from("my.bin")).unwrap();
        let dbg = format!("{:?}", hwdb);
        assert!(dbg.contains("my.bin"));
        assert!(dbg.contains("file_size"));
    }

    // -- Wildcard child in middle of pattern --------------------------------

    #[test]
    fn test_hwdb_question_mark_match() {
        // The ? child in the trie should match any single character.
        let data = build_minimal_hwdb(&[("usb:v12?4", "ID_FOUND", "yes")]);
        let hwdb = Hwdb::from_bytes(data, PathBuf::from("test.bin")).unwrap();

        let props = hwdb.lookup("usb:v1234");
        assert_eq!(props.get("ID_FOUND"), Some(&"yes".to_string()));

        let props2 = hwdb.lookup("usb:v12X4");
        assert_eq!(props2.get("ID_FOUND"), Some(&"yes".to_string()));
    }

    #[test]
    fn test_hwdb_bracket_match() {
        let data = build_minimal_hwdb(&[("type:[abc]end", "MATCHED", "1")]);
        let hwdb = Hwdb::from_bytes(data, PathBuf::from("test.bin")).unwrap();

        let props = hwdb.lookup("type:aend");
        assert_eq!(props.get("MATCHED"), Some(&"1".to_string()));

        let props2 = hwdb.lookup("type:dend");
        assert!(props2.is_empty());
    }

    // -- Empty search string ------------------------------------------------

    #[test]
    fn test_hwdb_empty_modalias() {
        let data = build_minimal_hwdb(&[("", "ROOT_PROP", "yes")]);
        let hwdb = Hwdb::from_bytes(data, PathBuf::from("test.bin")).unwrap();
        let props = hwdb.lookup("");
        assert_eq!(props.get("ROOT_PROP"), Some(&"yes".to_string()));
    }
}
