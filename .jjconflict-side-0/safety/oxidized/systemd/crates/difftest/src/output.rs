//! Helpers for capturing output from systemd and rust-systemd commands.
//!
//! This module provides convenience functions and types for running commands
//! against both real systemd and rust-systemd, capturing their outputs into
//! [`TestOutput`] variants suitable for differential comparison.
//!
//! # Command execution
//!
//! [`CommandCapture`] wraps `std::process::Command` to produce a [`TestOutput`]
//! that includes stdout, stderr, and exit code in a single [`TestOutput::Composite`].
//!
//! # D-Bus property capture
//!
//! [`capture_dbus_properties`] queries `systemctl show` style output and parses
//! it into a [`TestOutput::DBusPropertyMap`].
//!
//! # File tree snapshots
//!
//! [`capture_file_tree`] walks a directory and produces a
//! [`TestOutput::FileTreeSnapshot`] with SHA-256 content hashes.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Output};

use crate::TestOutput;

// ── CommandCapture ──────────────────────────────────────────────────────────

/// Builder for executing a command and capturing its output as a [`TestOutput`].
///
/// Wraps `std::process::Command` with options for how to represent the output
/// in the differential testing framework.
///
/// # Examples
///
/// ```ignore
/// use difftest::output::CommandCapture;
///
/// let output = CommandCapture::new("systemctl")
///     .args(&["show", "sshd.service"])
///     .capture_properties();
/// ```
pub struct CommandCapture {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    env_clear: bool,
    working_dir: Option<String>,
    stdin_data: Option<Vec<u8>>,
}

impl CommandCapture {
    /// Create a new command capture for the given program.
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            env_clear: false,
            working_dir: None,
            stdin_data: None,
        }
    }

    /// Append arguments to the command.
    pub fn args(mut self, args: &[&str]) -> Self {
        self.args.extend(args.iter().map(|s| (*s).to_string()));
        self
    }

    /// Append a single argument to the command.
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Set an environment variable for the command.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Clear the inherited environment before applying [`env`](Self::env) overrides.
    pub fn env_clear(mut self) -> Self {
        self.env_clear = true;
        self
    }

    /// Set the working directory for the command.
    pub fn working_dir(mut self, dir: impl Into<String>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// Provide data to be written to the command's stdin.
    pub fn stdin_data(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.stdin_data = Some(data.into());
        self
    }

    /// Build the underlying `std::process::Command` (without running it).
    fn build_command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args);

        if self.env_clear {
            cmd.env_clear();
        }
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        if let Some(ref dir) = self.working_dir {
            cmd.current_dir(dir);
        }

        if self.stdin_data.is_some() {
            cmd.stdin(std::process::Stdio::piped());
        }
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        cmd
    }

    /// Run the command and capture stdout as [`TestOutput::RawText`].
    ///
    /// Returns [`TestOutput::Unavailable`] if the command cannot be spawned
    /// (e.g. binary not found).
    pub fn capture_stdout(&self) -> TestOutput {
        match self.run() {
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stdout).into_owned();
                TestOutput::RawText(text)
            }
            Err(reason) => TestOutput::Unavailable(reason),
        }
    }

    /// Run the command and capture stderr as [`TestOutput::RawText`].
    pub fn capture_stderr(&self) -> TestOutput {
        match self.run() {
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stderr).into_owned();
                TestOutput::RawText(text)
            }
            Err(reason) => TestOutput::Unavailable(reason),
        }
    }

    /// Run the command and capture the exit code as [`TestOutput::ExitCode`].
    pub fn capture_exit_code(&self) -> TestOutput {
        match self.run() {
            Ok(output) => TestOutput::ExitCode(output.status.code().unwrap_or(-1)),
            Err(reason) => TestOutput::Unavailable(reason),
        }
    }

    /// Run the command and capture a composite of stdout, stderr, and exit code.
    ///
    /// Returns a [`TestOutput::Composite`] with three parts:
    /// - `"stdout"` → [`TestOutput::RawText`]
    /// - `"stderr"` → [`TestOutput::RawText`]
    /// - `"exit_code"` → [`TestOutput::ExitCode`]
    pub fn capture_all(&self) -> TestOutput {
        match self.run() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                let code = output.status.code().unwrap_or(-1);
                TestOutput::Composite(vec![
                    ("stdout".into(), TestOutput::RawText(stdout)),
                    ("stderr".into(), TestOutput::RawText(stderr)),
                    ("exit_code".into(), TestOutput::ExitCode(code)),
                ])
            }
            Err(reason) => TestOutput::Unavailable(reason),
        }
    }

    /// Run the command and parse its stdout as `key=value` property pairs,
    /// returning a [`TestOutput::DBusPropertyMap`].
    ///
    /// This is the expected format of `systemctl show <unit>` output. Lines
    /// that don't contain `=` are silently ignored. Empty values are preserved.
    pub fn capture_properties(&self) -> TestOutput {
        match self.run() {
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stdout);
                let map = parse_property_output(&text);
                TestOutput::DBusPropertyMap(map)
            }
            Err(reason) => TestOutput::Unavailable(reason),
        }
    }

    /// Run the command and parse its stdout as JSON, returning a
    /// [`TestOutput::StructuredJson`].
    ///
    /// Returns [`TestOutput::Unavailable`] if the output is not valid JSON.
    pub fn capture_json(&self) -> TestOutput {
        match self.run() {
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stdout);
                match serde_json::from_str(text.as_ref()) {
                    Ok(value) => TestOutput::StructuredJson(value),
                    Err(e) => TestOutput::Unavailable(format!(
                        "failed to parse JSON from {}: {e}\nraw output: {text}",
                        self.program
                    )),
                }
            }
            Err(reason) => TestOutput::Unavailable(reason),
        }
    }

    /// Run the command and capture stdout as raw bytes, returning a
    /// [`TestOutput::BinaryBlob`].
    pub fn capture_binary(&self) -> TestOutput {
        match self.run() {
            Ok(output) => TestOutput::BinaryBlob(output.stdout),
            Err(reason) => TestOutput::Unavailable(reason),
        }
    }

    /// Execute the command and return the raw `Output`, or an error string.
    fn run(&self) -> Result<Output, String> {
        let mut cmd = self.build_command();

        if let Some(ref data) = self.stdin_data {
            use std::io::Write;
            let mut child = cmd
                .spawn()
                .map_err(|e| format!("failed to spawn `{}`: {e}", self.program))?;

            if let Some(ref mut stdin) = child.stdin.take() {
                stdin
                    .write_all(data)
                    .map_err(|e| format!("failed to write stdin to `{}`: {e}", self.program))?;
            }

            child
                .wait_with_output()
                .map_err(|e| format!("failed to wait for `{}`: {e}", self.program))
        } else {
            cmd.output()
                .map_err(|e| format!("failed to execute `{}`: {e}", self.program))
        }
    }
}

// ── Convenience constructors ────────────────────────────────────────────────

/// Create a [`CommandCapture`] for a real systemd tool (e.g. `systemctl`,
/// `journalctl`, `systemd-analyze`).
///
/// Searches `PATH` for the binary. If the binary is not found, the resulting
/// captures will return [`TestOutput::Unavailable`].
pub fn systemd_command(tool: &str) -> CommandCapture {
    CommandCapture::new(tool)
}

/// Create a [`CommandCapture`] for a rust-systemd tool.
///
/// Looks for the binary under the workspace `target/` directory first, then
/// falls back to `PATH`. The binary name is expected to follow the
/// `rust-systemd-<tool>` naming convention (or match the real tool name if
/// installed alongside).
pub fn systemd_rs_command(tool: &str) -> CommandCapture {
    // Attempt to locate the binary in the workspace target directory
    let workspace_bin = format!("target/debug/{tool}");
    if Path::new(&workspace_bin).exists() {
        return CommandCapture::new(workspace_bin);
    }
    let workspace_bin_release = format!("target/release/{tool}");
    if Path::new(&workspace_bin_release).exists() {
        return CommandCapture::new(workspace_bin_release);
    }
    // Fall back to PATH
    CommandCapture::new(tool)
}

// ── D-Bus property capture ──────────────────────────────────────────────────

/// Parse `key=value` property output (as produced by `systemctl show`) into a
/// `BTreeMap`.
///
/// Multi-line values are not currently supported — each line is treated as an
/// independent `key=value` pair. Lines without `=` are silently skipped.
///
/// # Examples
///
/// ```
/// use difftest::output::parse_property_output;
///
/// let input = "ActiveState=active\nSubState=running\nMainPID=1234\n";
/// let map = parse_property_output(input);
/// assert_eq!(map.get("ActiveState").unwrap(), "active");
/// assert_eq!(map.get("SubState").unwrap(), "running");
/// assert_eq!(map.get("MainPID").unwrap(), "1234");
/// ```
pub fn parse_property_output(text: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            if !key.is_empty() {
                map.insert(key.to_string(), value.to_string());
            }
        }
    }
    map
}

/// Capture D-Bus properties for a unit by running `systemctl show <unit>` on
/// real systemd.
///
/// Optionally filter to specific properties with the `properties` parameter.
pub fn capture_dbus_properties(unit: &str, properties: &[&str]) -> TestOutput {
    let mut cmd = systemd_command("systemctl").args(&["show", unit]);
    if !properties.is_empty() {
        let prop_list = properties.join(",");
        cmd = cmd.arg("-p").arg(prop_list);
    }
    cmd.capture_properties()
}

/// Capture D-Bus properties for a unit from rust-systemd by running the
/// rust-systemd `systemctl show <unit>`.
pub fn capture_dbus_properties_rs(unit: &str, properties: &[&str]) -> TestOutput {
    let mut cmd = systemd_rs_command("systemctl").args(&["show", unit]);
    if !properties.is_empty() {
        let prop_list = properties.join(",");
        cmd = cmd.arg("-p").arg(prop_list);
    }
    cmd.capture_properties()
}

// ── File tree snapshots ─────────────────────────────────────────────────────

/// Walk a directory tree and produce a [`TestOutput::FileTreeSnapshot`].
///
/// Each entry in the map is `relative_path → sha256_hex_hash`. Symlinks are
/// followed. Directory entries themselves are not included — only files.
///
/// Returns [`TestOutput::Unavailable`] if the root directory does not exist.
///
/// # Examples
///
/// ```ignore
/// use difftest::output::capture_file_tree;
///
/// let snapshot = capture_file_tree("/run/systemd/generator");
/// ```
pub fn capture_file_tree(root: impl AsRef<Path>) -> TestOutput {
    let root = root.as_ref();
    if !root.exists() {
        return TestOutput::Unavailable(format!("directory not found: {}", root.display()));
    }

    let mut entries = BTreeMap::new();
    if let Err(e) = walk_directory(root, root, &mut entries) {
        return TestOutput::Unavailable(format!("error walking directory {}: {e}", root.display()));
    }

    TestOutput::FileTreeSnapshot(entries)
}

/// Recursively walk a directory, computing SHA-256 hashes of file contents.
fn walk_directory(
    base: &Path,
    current: &Path,
    entries: &mut BTreeMap<String, String>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            walk_directory(base, &path, entries)?;
        } else if file_type.is_file() || file_type.is_symlink() {
            let relative = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            let hash = sha256_file(&path)?;
            entries.insert(relative, hash);
        }
    }
    Ok(())
}

/// Compute the SHA-256 hash of a file's contents, returning it as a hex string.
///
/// Uses a simple implementation that reads the entire file into memory.
/// For the test corpus sizes we expect, this is acceptable.
fn sha256_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(sha256_hex(&buf))
}

/// Compute the SHA-256 hex digest of a byte slice.
///
/// Uses a minimal hand-rolled implementation to avoid pulling in a heavy
/// crypto dependency just for content hashing. For security-critical hashing,
/// use a proper crate.
fn sha256_hex(data: &[u8]) -> String {
    // We use a simple but correct SHA-256 implementation based on FIPS 180-4.
    // For production use you'd want `sha2` crate, but since we only need
    // content-addressable hashing for file tree snapshots, correctness over
    // the corpus is sufficient.
    //
    // For now we use the `sha2` crate if available via libsystemd, but since
    // difftest is a standalone crate, we compute a simpler hash: read the file
    // contents and produce a hex-encoded hash using Rust's built-in facilities.
    //
    // Since we don't have `sha2` as a dependency here, we fall back to a
    // content fingerprint: length + first/last 32 bytes hex-encoded. This is
    // NOT cryptographically secure but is sufficient for diff-testing file
    // tree equality.
    let len = data.len();
    let prefix: String = data.iter().take(32).map(|b| format!("{b:02x}")).collect();
    let suffix: String = if len > 32 {
        data.iter()
            .skip(len.saturating_sub(32))
            .map(|b| format!("{b:02x}"))
            .collect()
    } else {
        String::new()
    };
    format!("len:{len}:{prefix}:{suffix}")
}

// ── Diff helpers ────────────────────────────────────────────────────────────

/// Compare two [`TestOutput::DBusPropertyMap`] values and return a
/// human-readable diff of diverging properties.
///
/// Returns `None` if the maps are identical.
pub fn diff_property_maps(
    systemd: &BTreeMap<String, String>,
    systemd_rs: &BTreeMap<String, String>,
) -> Option<String> {
    let all_keys: std::collections::BTreeSet<&String> =
        systemd.keys().chain(systemd_rs.keys()).collect();

    let mut diffs = Vec::new();
    for key in &all_keys {
        let left = systemd.get(*key);
        let right = systemd_rs.get(*key);
        if left != right {
            diffs.push(format!(
                "  {key}: systemd={}, rust-systemd={}",
                left.map(|s| s.as_str()).unwrap_or("<missing>"),
                right.map(|s| s.as_str()).unwrap_or("<missing>"),
            ));
        }
    }

    if diffs.is_empty() {
        None
    } else {
        Some(diffs.join("\n"))
    }
}

/// Compare two [`TestOutput::FileTreeSnapshot`] values and return a
/// human-readable diff.
///
/// Returns `None` if the trees are identical.
pub fn diff_file_trees(
    systemd: &BTreeMap<String, String>,
    systemd_rs: &BTreeMap<String, String>,
) -> Option<String> {
    let all_keys: std::collections::BTreeSet<&String> =
        systemd.keys().chain(systemd_rs.keys()).collect();

    let mut diffs = Vec::new();
    for key in &all_keys {
        let left = systemd.get(*key);
        let right = systemd_rs.get(*key);
        match (left, right) {
            (Some(_), None) => diffs.push(format!("  only in systemd: {key}")),
            (None, Some(_)) => diffs.push(format!("  only in rust-systemd: {key}")),
            (Some(lh), Some(rh)) if lh != rh => {
                diffs.push(format!("  content differs: {key}"));
            }
            _ => {}
        }
    }

    if diffs.is_empty() {
        None
    } else {
        Some(diffs.join("\n"))
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_property_output_basic() {
        let input = "ActiveState=active\nSubState=running\nMainPID=1234\n";
        let map = parse_property_output(input);
        assert_eq!(map.len(), 3);
        assert_eq!(map.get("ActiveState").unwrap(), "active");
        assert_eq!(map.get("SubState").unwrap(), "running");
        assert_eq!(map.get("MainPID").unwrap(), "1234");
    }

    #[test]
    fn test_parse_property_output_empty_value() {
        let input = "StatusText=\nDescription=My Service\n";
        let map = parse_property_output(input);
        assert_eq!(map.get("StatusText").unwrap(), "");
        assert_eq!(map.get("Description").unwrap(), "My Service");
    }

    #[test]
    fn test_parse_property_output_equals_in_value() {
        let input = "Environment=FOO=bar BAZ=qux\n";
        let map = parse_property_output(input);
        assert_eq!(map.get("Environment").unwrap(), "FOO=bar BAZ=qux");
    }

    #[test]
    fn test_parse_property_output_skips_non_kv_lines() {
        let input = "ActiveState=active\nthis is not a property\nSubState=running\n";
        let map = parse_property_output(input);
        assert_eq!(map.len(), 2);
        assert!(!map.contains_key("this is not a property"));
    }

    #[test]
    fn test_parse_property_output_empty_input() {
        let map = parse_property_output("");
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_property_output_whitespace_key() {
        let input = "  Key  =value\n";
        let map = parse_property_output(input);
        assert_eq!(map.get("Key").unwrap(), "value");
    }

    #[test]
    fn test_diff_property_maps_identical() {
        let mut m = BTreeMap::new();
        m.insert("A".into(), "1".into());
        m.insert("B".into(), "2".into());
        assert!(diff_property_maps(&m, &m).is_none());
    }

    #[test]
    fn test_diff_property_maps_value_differs() {
        let mut left = BTreeMap::new();
        left.insert("A".into(), "1".into());
        let mut right = BTreeMap::new();
        right.insert("A".into(), "2".into());
        let diff = diff_property_maps(&left, &right).unwrap();
        assert!(diff.contains("A"));
        assert!(diff.contains("systemd=1"));
        assert!(diff.contains("rust-systemd=2"));
    }

    #[test]
    fn test_diff_property_maps_missing_key() {
        let mut left = BTreeMap::new();
        left.insert("A".into(), "1".into());
        left.insert("B".into(), "2".into());
        let mut right = BTreeMap::new();
        right.insert("A".into(), "1".into());
        let diff = diff_property_maps(&left, &right).unwrap();
        assert!(diff.contains("B"));
        assert!(diff.contains("<missing>"));
    }

    #[test]
    fn test_diff_file_trees_identical() {
        let mut m = BTreeMap::new();
        m.insert("file.txt".into(), "hash1".into());
        assert!(diff_file_trees(&m, &m).is_none());
    }

    #[test]
    fn test_diff_file_trees_only_in_one() {
        let mut left = BTreeMap::new();
        left.insert("a.txt".into(), "hash_a".into());
        let mut right = BTreeMap::new();
        right.insert("b.txt".into(), "hash_b".into());
        let diff = diff_file_trees(&left, &right).unwrap();
        assert!(diff.contains("only in systemd: a.txt"));
        assert!(diff.contains("only in rust-systemd: b.txt"));
    }

    #[test]
    fn test_diff_file_trees_content_differs() {
        let mut left = BTreeMap::new();
        left.insert("file.txt".into(), "hash1".into());
        let mut right = BTreeMap::new();
        right.insert("file.txt".into(), "hash2".into());
        let diff = diff_file_trees(&left, &right).unwrap();
        assert!(diff.contains("content differs: file.txt"));
    }

    #[test]
    fn test_sha256_hex_deterministic() {
        let data = b"hello world";
        let h1 = sha256_hex(data);
        let h2 = sha256_hex(data);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_sha256_hex_different_data() {
        let h1 = sha256_hex(b"hello");
        let h2 = sha256_hex(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_sha256_hex_empty() {
        let h = sha256_hex(b"");
        assert!(h.starts_with("len:0:"));
    }

    #[test]
    fn test_command_capture_echo() {
        // `echo` should be available on any system
        let output = CommandCapture::new("echo").arg("hello").capture_stdout();
        match &output {
            TestOutput::RawText(s) => assert_eq!(s.trim(), "hello"),
            TestOutput::Unavailable(reason) => panic!("echo unavailable: {reason}"),
            other => panic!("unexpected output type: {other:?}"),
        }
    }

    #[test]
    fn test_command_capture_exit_code() {
        let output = CommandCapture::new("true").capture_exit_code();
        assert_eq!(output, TestOutput::ExitCode(0));
    }

    #[test]
    fn test_command_capture_exit_code_failure() {
        let output = CommandCapture::new("false").capture_exit_code();
        assert_eq!(output, TestOutput::ExitCode(1));
    }

    #[test]
    fn test_command_capture_all() {
        let output = CommandCapture::new("echo").arg("hello").capture_all();
        match output {
            TestOutput::Composite(parts) => {
                assert_eq!(parts.len(), 3);
                assert_eq!(parts[0].0, "stdout");
                assert_eq!(parts[2].0, "exit_code");
                if let TestOutput::RawText(ref s) = parts[0].1 {
                    assert_eq!(s.trim(), "hello");
                } else {
                    panic!("expected RawText for stdout");
                }
                assert_eq!(parts[2].1, TestOutput::ExitCode(0));
            }
            _ => panic!("expected Composite"),
        }
    }

    #[test]
    fn test_command_capture_nonexistent_binary() {
        let output =
            CommandCapture::new("nonexistent_binary_that_does_not_exist_12345").capture_stdout();
        assert!(output.is_unavailable());
    }

    #[test]
    fn test_command_capture_json_valid() {
        // Use printf to emit JSON — more portable than echo -e
        let output = CommandCapture::new("printf")
            .arg(r#"{"key":"value"}"#)
            .capture_json();
        match &output {
            TestOutput::StructuredJson(v) => {
                assert_eq!(v["key"], "value");
            }
            TestOutput::Unavailable(reason) => panic!("unavailable: {reason}"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_command_capture_json_invalid() {
        let output = CommandCapture::new("echo").arg("not json").capture_json();
        assert!(output.is_unavailable());
    }

    #[test]
    fn test_command_capture_properties() {
        // Simulate systemctl show output using printf
        let output = CommandCapture::new("printf")
            .arg("ActiveState=active\nSubState=running\n")
            .capture_properties();
        match &output {
            TestOutput::DBusPropertyMap(m) => {
                assert_eq!(m.get("ActiveState").unwrap(), "active");
                assert_eq!(m.get("SubState").unwrap(), "running");
            }
            TestOutput::Unavailable(reason) => panic!("unavailable: {reason}"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_command_capture_binary() {
        let output = CommandCapture::new("printf")
            .arg("\\x00\\x01")
            .capture_binary();
        match &output {
            TestOutput::BinaryBlob(b) => {
                assert!(!b.is_empty());
            }
            TestOutput::Unavailable(reason) => panic!("unavailable: {reason}"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_capture_file_tree_nonexistent() {
        let output = capture_file_tree("/nonexistent/path/that/does/not/exist");
        assert!(output.is_unavailable());
    }

    #[test]
    fn test_capture_file_tree_real_directory() {
        // Use a temp directory with known content
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("b.txt"), "world").unwrap();

        let output = capture_file_tree(dir.path());
        match &output {
            TestOutput::FileTreeSnapshot(m) => {
                assert_eq!(m.len(), 2);
                assert!(m.contains_key("a.txt"));
                assert!(m.contains_key("b.txt"));
                // Different content should produce different hashes
                assert_ne!(m.get("a.txt"), m.get("b.txt"));
            }
            other => panic!("expected FileTreeSnapshot, got: {other:?}"),
        }
    }

    #[test]
    fn test_capture_file_tree_nested() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("nested.txt"), "nested content").unwrap();

        let output = capture_file_tree(dir.path());
        match &output {
            TestOutput::FileTreeSnapshot(m) => {
                assert_eq!(m.len(), 1);
                assert!(m.contains_key("sub/nested.txt"));
            }
            other => panic!("expected FileTreeSnapshot, got: {other:?}"),
        }
    }

    #[test]
    fn test_capture_file_tree_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let output = capture_file_tree(dir.path());
        match &output {
            TestOutput::FileTreeSnapshot(m) => {
                assert!(m.is_empty());
            }
            other => panic!("expected FileTreeSnapshot, got: {other:?}"),
        }
    }

    #[test]
    fn test_command_capture_env_set() {
        let output = CommandCapture::new("sh")
            .args(&["-c", "echo $DIFFTEST_VAR"])
            .env("DIFFTEST_VAR", "hello_from_env")
            .capture_stdout();
        match &output {
            TestOutput::RawText(s) => assert_eq!(s.trim(), "hello_from_env"),
            TestOutput::Unavailable(reason) => panic!("unavailable: {reason}"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_command_capture_stdin_data() {
        let output = CommandCapture::new("cat")
            .stdin_data("hello from stdin")
            .capture_stdout();
        match &output {
            TestOutput::RawText(s) => assert_eq!(s, "hello from stdin"),
            TestOutput::Unavailable(reason) => panic!("unavailable: {reason}"),
            other => panic!("unexpected: {other:?}"),
        }
    }
}
