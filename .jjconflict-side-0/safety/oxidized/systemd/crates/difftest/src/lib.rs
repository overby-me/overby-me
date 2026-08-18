//! `difftest` — Differential testing framework for rust-systemd.
//!
//! Provides infrastructure for running identical inputs through both real systemd
//! and rust-systemd, then comparing outputs for behavioral equivalence.
//!
//! # Core Types
//!
//! - [`DiffTest`] — Trait implemented by each differential test case.
//! - [`TestOutput`] — Enum representing the output from either implementation.
//! - [`DiffResult`] — The outcome of comparing two `TestOutput` values.
//! - [`DiffTestRunner`] — Parallel test executor with filtering and reporting.
//! - [`Normalizer`] — Pipeline for canonicalizing non-deterministic output.
//!
//! # Usage
//!
//! ```ignore
//! use difftest::{DiffTest, TestOutput, DiffResult};
//! use difftest_macros::difftest;
//!
//! struct MyDiffTest {
//!     input: String,
//! }
//!
//! impl DiffTest for MyDiffTest {
//!     fn name(&self) -> &str { "my-diff-test" }
//!     fn run_systemd(&self) -> TestOutput {
//!         // Execute against real systemd
//!         TestOutput::RawText("output from systemd".into())
//!     }
//!     fn run_systemd_rs(&self) -> TestOutput {
//!         // Execute against rust-systemd
//!         TestOutput::RawText("output from rust-systemd".into())
//!     }
//!     fn compare(&self, left: &TestOutput, right: &TestOutput) -> DiffResult {
//!         if left == right {
//!             DiffResult::Identical
//!         } else {
//!             DiffResult::Divergent(format!("left: {:?}\nright: {:?}", left, right))
//!         }
//!     }
//! }
//! ```

pub mod normalizer;
pub mod output;
pub mod report;
pub mod runner;
pub mod snapshot;

// Re-export the proc-macro crate so users can do `use difftest::difftest;`
pub use difftest_macros::difftest;

// Re-export inventory so the proc-macro generated code can reference it
pub use inventory;

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

// ── Core trait ──────────────────────────────────────────────────────────────

/// A single differential test case that compares systemd and rust-systemd
/// behavior for identical inputs.
pub trait DiffTest: Send + Sync {
    /// Human-readable test name, used in reports and filtering.
    fn name(&self) -> &str;

    /// Optional category for grouping (e.g. `"unit_parsing"`, `"dbus"`).
    fn category(&self) -> &str {
        "default"
    }

    /// Run the test against **real systemd** and capture output.
    fn run_systemd(&self) -> TestOutput;

    /// Run the test against **rust-systemd** and capture output.
    fn run_systemd_rs(&self) -> TestOutput;

    /// Compare two outputs and produce a [`DiffResult`].
    ///
    /// The default implementation uses [`TestOutput::structural_eq`] with the
    /// global [`Normalizer`](normalizer::Normalizer) pipeline, but tests can
    /// override this for domain-specific comparison logic.
    fn compare(&self, left: &TestOutput, right: &TestOutput) -> DiffResult {
        default_compare(left, right)
    }
}

// ── TestOutput ──────────────────────────────────────────────────────────────

/// Output captured from either the real systemd or rust-systemd implementation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TestOutput {
    /// Structured JSON value (e.g. from `systemctl show -o json`).
    StructuredJson(serde_json::Value),

    /// Raw text output (stdout/stderr from a command).
    RawText(String),

    /// Binary blob (e.g. journal file content, credential ciphertext).
    BinaryBlob(Vec<u8>),

    /// Process exit code.
    ExitCode(i32),

    /// File-tree snapshot: sorted map of `relative_path → content_hash`.
    FileTreeSnapshot(BTreeMap<String, String>),

    /// D-Bus property map: sorted map of `property_name → value_string`.
    DBusPropertyMap(BTreeMap<String, String>),

    /// Composite output containing multiple facets of a single test run.
    Composite(Vec<(String, TestOutput)>),

    /// The implementation is not available (e.g. systemd not installed).
    Unavailable(String),
}

impl TestOutput {
    /// Returns `true` if the output represents an unavailable implementation.
    pub fn is_unavailable(&self) -> bool {
        matches!(self, TestOutput::Unavailable(_))
    }

    /// Attempt to extract the inner text, if this is a `RawText` variant.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            TestOutput::RawText(s) => Some(s),
            _ => None,
        }
    }

    /// Attempt to extract the inner JSON value.
    pub fn as_json(&self) -> Option<&serde_json::Value> {
        match self {
            TestOutput::StructuredJson(v) => Some(v),
            _ => None,
        }
    }

    /// Attempt to extract the inner exit code.
    pub fn as_exit_code(&self) -> Option<i32> {
        match self {
            TestOutput::ExitCode(c) => Some(*c),
            _ => None,
        }
    }
}

impl fmt::Display for TestOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestOutput::StructuredJson(v) => {
                write!(f, "{}", serde_json::to_string_pretty(v).unwrap_or_default())
            }
            TestOutput::RawText(s) => write!(f, "{s}"),
            TestOutput::BinaryBlob(b) => write!(f, "<binary {} bytes>", b.len()),
            TestOutput::ExitCode(c) => write!(f, "exit code {c}"),
            TestOutput::FileTreeSnapshot(m) => {
                for (path, hash) in m {
                    writeln!(f, "{path}  {hash}")?;
                }
                Ok(())
            }
            TestOutput::DBusPropertyMap(m) => {
                for (k, v) in m {
                    writeln!(f, "{k}={v}")?;
                }
                Ok(())
            }
            TestOutput::Composite(parts) => {
                for (label, output) in parts {
                    writeln!(f, "--- {label} ---")?;
                    writeln!(f, "{output}")?;
                }
                Ok(())
            }
            TestOutput::Unavailable(reason) => write!(f, "<unavailable: {reason}>"),
        }
    }
}

// ── DiffResult ──────────────────────────────────────────────────────────────

/// The result of comparing outputs from systemd and rust-systemd.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiffResult {
    /// Outputs are byte-for-byte identical.
    Identical,

    /// Outputs are semantically equivalent after normalization.
    ///
    /// The `String` contains notes about what normalization was applied
    /// (e.g. "PID values differed", "timestamps normalized").
    Equivalent(String),

    /// Outputs are meaningfully different.
    ///
    /// The `String` contains a human-readable explanation of the divergence.
    Divergent(String),

    /// Comparison was skipped because one or both sides were unavailable.
    Skipped(String),
}

impl DiffResult {
    /// Returns `true` if the result represents a passing test.
    pub fn is_pass(&self) -> bool {
        matches!(self, DiffResult::Identical | DiffResult::Equivalent(_))
    }

    /// Returns `true` if the result is a divergence (failure).
    pub fn is_divergent(&self) -> bool {
        matches!(self, DiffResult::Divergent(_))
    }

    /// Returns `true` if the test was skipped.
    pub fn is_skipped(&self) -> bool {
        matches!(self, DiffResult::Skipped(_))
    }
}

impl fmt::Display for DiffResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiffResult::Identical => write!(f, "IDENTICAL"),
            DiffResult::Equivalent(notes) => write!(f, "EQUIVALENT ({notes})"),
            DiffResult::Divergent(explanation) => write!(f, "DIVERGENT: {explanation}"),
            DiffResult::Skipped(reason) => write!(f, "SKIPPED: {reason}"),
        }
    }
}

// ── Default comparison ──────────────────────────────────────────────────────

/// Default comparison function used when [`DiffTest::compare`] is not overridden.
///
/// Applies the standard normalizer pipeline and then checks for equality.
pub fn default_compare(left: &TestOutput, right: &TestOutput) -> DiffResult {
    // If either side is unavailable, skip
    if let TestOutput::Unavailable(reason) = left {
        return DiffResult::Skipped(format!("systemd unavailable: {reason}"));
    }
    if let TestOutput::Unavailable(reason) = right {
        return DiffResult::Skipped(format!("rust-systemd unavailable: {reason}"));
    }

    // Fast path: identical without normalization
    if left == right {
        return DiffResult::Identical;
    }

    // Apply normalizers and try again
    let norm = normalizer::Normalizer::default();
    let left_norm = norm.normalize(left);
    let right_norm = norm.normalize(right);

    if left_norm == right_norm {
        let notes = norm.applied_normalizations(left, right);
        DiffResult::Equivalent(notes)
    } else {
        let explanation = build_divergence_explanation(&left_norm, &right_norm);
        DiffResult::Divergent(explanation)
    }
}

/// Build a human-readable explanation of how two outputs differ.
fn build_divergence_explanation(left: &TestOutput, right: &TestOutput) -> String {
    match (left, right) {
        (TestOutput::RawText(l), TestOutput::RawText(r)) => {
            let mut out = String::new();
            out.push_str("Text output differs:\n");
            // Simple line-by-line diff
            let left_lines: Vec<&str> = l.lines().collect();
            let right_lines: Vec<&str> = r.lines().collect();
            let max = left_lines.len().max(right_lines.len());
            for i in 0..max {
                let ll = left_lines.get(i).unwrap_or(&"<missing>");
                let rl = right_lines.get(i).unwrap_or(&"<missing>");
                if ll != rl {
                    out.push_str(&format!("  line {}: systemd:    {ll}\n", i + 1));
                    out.push_str(&format!("  line {}: rust-systemd: {rl}\n", i + 1));
                }
            }
            if left_lines.len() != right_lines.len() {
                out.push_str(&format!(
                    "  (systemd: {} lines, rust-systemd: {} lines)\n",
                    left_lines.len(),
                    right_lines.len()
                ));
            }
            out
        }
        (TestOutput::ExitCode(l), TestOutput::ExitCode(r)) => {
            format!("Exit code differs: systemd={l}, rust-systemd={r}")
        }
        (TestOutput::StructuredJson(l), TestOutput::StructuredJson(r)) => {
            format!(
                "JSON output differs:\n  systemd:    {}\n  rust-systemd: {}",
                serde_json::to_string(l).unwrap_or_default(),
                serde_json::to_string(r).unwrap_or_default(),
            )
        }
        (TestOutput::DBusPropertyMap(l), TestOutput::DBusPropertyMap(r)) => {
            let mut out = String::from("D-Bus property map differs:\n");
            // Find keys that differ
            let all_keys: std::collections::BTreeSet<&String> = l.keys().chain(r.keys()).collect();
            for key in all_keys {
                let lv = l.get(key);
                let rv = r.get(key);
                if lv != rv {
                    out.push_str(&format!(
                        "  {key}: systemd={}, rust-systemd={}\n",
                        lv.map(|s| s.as_str()).unwrap_or("<missing>"),
                        rv.map(|s| s.as_str()).unwrap_or("<missing>"),
                    ));
                }
            }
            out
        }
        (TestOutput::FileTreeSnapshot(l), TestOutput::FileTreeSnapshot(r)) => {
            let mut out = String::from("File tree snapshot differs:\n");
            let all_keys: std::collections::BTreeSet<&String> = l.keys().chain(r.keys()).collect();
            for key in all_keys {
                let lv = l.get(key);
                let rv = r.get(key);
                if lv != rv {
                    out.push_str(&format!(
                        "  {key}: systemd={}, rust-systemd={}\n",
                        lv.map(|s| s.as_str()).unwrap_or("<missing>"),
                        rv.map(|s| s.as_str()).unwrap_or("<missing>"),
                    ));
                }
            }
            out
        }
        _ => {
            format!(
                "Output type or content differs:\n  systemd:    {left}\n  rust-systemd: {right}"
            )
        }
    }
}

// ── DiffTestRegistration (for inventory) ────────────────────────────────────

/// A registration entry created by the `#[difftest]` proc-macro.
///
/// These are collected at link-time via the `inventory` crate and discovered
/// by [`DiffTestRunner`](runner::DiffTestRunner).
pub struct DiffTestRegistration {
    /// Test name (matches the function name).
    pub name: &'static str,
    /// Category string for grouping.
    pub category: &'static str,
    /// Per-test timeout in milliseconds.
    pub timeout_ms: u64,
    /// Tags for filtering.
    pub tags: &'static [String],
    /// Whether the test is ignored by default.
    pub ignored: bool,
    /// Factory function that constructs the `DiffTest` impl.
    pub constructor: fn() -> Box<dyn DiffTest>,
}

// Safety: the constructor fn pointer is Send+Sync, and the static data is too.
unsafe impl Send for DiffTestRegistration {}
unsafe impl Sync for DiffTestRegistration {}

inventory::collect!(DiffTestRegistration);

// ── Known divergences ───────────────────────────────────────────────────────

/// A known divergence entry loaded from `tests/difftest/known-divergences.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownDivergence {
    /// The test name that is known to diverge.
    pub test: String,
    /// Reason / tracking issue for the divergence.
    pub reason: String,
    /// Optional systemd version constraint (e.g. ">=255").
    pub systemd_version: Option<String>,
}

/// Load known divergences from a TOML file.
///
/// Expected format:
/// ```toml
/// [[divergence]]
/// test = "ini_parser_trailing_backslash"
/// reason = "systemd silently discards trailing backslash at EOF, we error"
///
/// [[divergence]]
/// test = "dbus_manager_list_units_ordering"
/// reason = "non-deterministic ordering, tracked in #42"
/// systemd_version = ">=256"
/// ```
pub fn load_known_divergences(
    path: &std::path::Path,
) -> Result<Vec<KnownDivergence>, DiffTestError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| DiffTestError::Io(format!("reading {}: {e}", path.display())))?;

    #[derive(Deserialize)]
    struct KnownDivergencesFile {
        #[serde(default)]
        divergence: Vec<KnownDivergence>,
    }

    let file: KnownDivergencesFile = toml::from_str(&content)
        .map_err(|e| DiffTestError::Config(format!("parsing {}: {e}", path.display())))?;
    Ok(file.divergence)
}

// ── Convenience runner for proc-macro generated tests ───────────────────────

/// Run a single differential test with a timeout, returning the [`DiffResult`].
///
/// This is called by the `#[test]` function generated by `#[difftest]`.
pub fn run_single_difftest(test: &dyn DiffTest, _timeout_ms: u64) -> DiffResult {
    // TODO: enforce timeout via thread::spawn + recv_timeout when running in
    // VM-backed mode. For now, run synchronously.
    let left = test.run_systemd();
    let right = test.run_systemd_rs();
    test.compare(&left, &right)
}

// ── Errors ──────────────────────────────────────────────────────────────────

/// Errors that can occur in the difftest framework.
#[derive(Debug, thiserror::Error)]
pub enum DiffTestError {
    #[error("I/O error: {0}")]
    Io(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("VM communication error: {0}")]
    VmComm(String),

    #[error("Test timeout after {0}ms")]
    Timeout(u64),

    #[error("Serialization error: {0}")]
    Serialization(String),
}

// ── Test result record ──────────────────────────────────────────────────────

/// Complete record of a single test execution, used for reporting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRecord {
    /// Test name.
    pub name: String,
    /// Test category.
    pub category: String,
    /// The comparison result.
    pub result: DiffResult,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Whether this divergence was previously known.
    pub known_divergence: bool,
    /// Optional reason from known-divergences file.
    pub known_reason: Option<String>,
}

impl TestRecord {
    /// Returns `true` if this test should be treated as a failure for CI
    /// purposes (divergent and not a known divergence).
    pub fn is_new_failure(&self) -> bool {
        self.result.is_divergent() && !self.known_divergence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_raw_text() {
        let a = TestOutput::RawText("hello".into());
        let b = TestOutput::RawText("hello".into());
        assert_eq!(default_compare(&a, &b), DiffResult::Identical);
    }

    #[test]
    fn test_divergent_raw_text() {
        let a = TestOutput::RawText("hello".into());
        let b = TestOutput::RawText("world".into());
        assert!(default_compare(&a, &b).is_divergent());
    }

    #[test]
    fn test_divergent_exit_code() {
        let a = TestOutput::ExitCode(0);
        let b = TestOutput::ExitCode(1);
        let result = default_compare(&a, &b);
        assert!(result.is_divergent());
        if let DiffResult::Divergent(msg) = &result {
            assert!(msg.contains("Exit code differs"));
        }
    }

    #[test]
    fn test_skipped_when_unavailable() {
        let a = TestOutput::Unavailable("no systemd".into());
        let b = TestOutput::RawText("output".into());
        assert!(default_compare(&a, &b).is_skipped());
    }

    #[test]
    fn test_identical_exit_codes() {
        let a = TestOutput::ExitCode(0);
        let b = TestOutput::ExitCode(0);
        assert_eq!(default_compare(&a, &b), DiffResult::Identical);
    }

    #[test]
    fn test_identical_dbus_property_map() {
        let mut m = BTreeMap::new();
        m.insert("ActiveState".into(), "active".into());
        m.insert("SubState".into(), "running".into());
        let a = TestOutput::DBusPropertyMap(m.clone());
        let b = TestOutput::DBusPropertyMap(m);
        assert_eq!(default_compare(&a, &b), DiffResult::Identical);
    }

    #[test]
    fn test_divergent_dbus_property_map() {
        let mut m1 = BTreeMap::new();
        m1.insert("ActiveState".into(), "active".into());
        let mut m2 = BTreeMap::new();
        m2.insert("ActiveState".into(), "inactive".into());
        let result = default_compare(
            &TestOutput::DBusPropertyMap(m1),
            &TestOutput::DBusPropertyMap(m2),
        );
        assert!(result.is_divergent());
    }

    #[test]
    fn test_diff_result_display() {
        assert_eq!(DiffResult::Identical.to_string(), "IDENTICAL");
        assert_eq!(
            DiffResult::Equivalent("PIDs normalized".into()).to_string(),
            "EQUIVALENT (PIDs normalized)"
        );
        assert!(
            DiffResult::Divergent("mismatch".into())
                .to_string()
                .starts_with("DIVERGENT:")
        );
    }

    #[test]
    fn test_test_output_display() {
        assert_eq!(TestOutput::ExitCode(42).to_string(), "exit code 42");
        assert_eq!(TestOutput::RawText("hi".into()).to_string(), "hi");
        assert_eq!(
            TestOutput::BinaryBlob(vec![1, 2, 3]).to_string(),
            "<binary 3 bytes>"
        );
        assert_eq!(
            TestOutput::Unavailable("nope".into()).to_string(),
            "<unavailable: nope>"
        );
    }

    #[test]
    fn test_test_output_accessors() {
        let text = TestOutput::RawText("hello".into());
        assert_eq!(text.as_text(), Some("hello"));
        assert_eq!(text.as_json(), None);
        assert_eq!(text.as_exit_code(), None);
        assert!(!text.is_unavailable());

        let code = TestOutput::ExitCode(0);
        assert_eq!(code.as_exit_code(), Some(0));
        assert_eq!(code.as_text(), None);

        let unavail = TestOutput::Unavailable("x".into());
        assert!(unavail.is_unavailable());
    }

    #[test]
    fn test_test_record_is_new_failure() {
        let rec = TestRecord {
            name: "test1".into(),
            category: "default".into(),
            result: DiffResult::Divergent("bad".into()),
            duration_ms: 100,
            known_divergence: false,
            known_reason: None,
        };
        assert!(rec.is_new_failure());

        let rec_known = TestRecord {
            known_divergence: true,
            known_reason: Some("tracked".into()),
            ..rec
        };
        assert!(!rec_known.is_new_failure());
    }

    #[test]
    fn test_load_known_divergences_missing_file() {
        let path = std::path::Path::new("/nonexistent/known-divergences.toml");
        let result = load_known_divergences(path).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_composite_output_display() {
        let composite = TestOutput::Composite(vec![
            ("stdout".into(), TestOutput::RawText("hello".into())),
            ("exit".into(), TestOutput::ExitCode(0)),
        ]);
        let display = composite.to_string();
        assert!(display.contains("--- stdout ---"));
        assert!(display.contains("hello"));
        assert!(display.contains("--- exit ---"));
        assert!(display.contains("exit code 0"));
    }

    #[test]
    fn test_diff_result_predicates() {
        assert!(DiffResult::Identical.is_pass());
        assert!(DiffResult::Equivalent("x".into()).is_pass());
        assert!(!DiffResult::Divergent("x".into()).is_pass());
        assert!(!DiffResult::Skipped("x".into()).is_pass());

        assert!(!DiffResult::Identical.is_divergent());
        assert!(DiffResult::Divergent("x".into()).is_divergent());

        assert!(!DiffResult::Identical.is_skipped());
        assert!(DiffResult::Skipped("x".into()).is_skipped());
    }
}
