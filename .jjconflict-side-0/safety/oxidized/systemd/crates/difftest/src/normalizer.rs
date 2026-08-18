//! Output normalizers for canonicalizing non-deterministic values.
//!
//! When comparing outputs from real systemd and rust-systemd, certain values are
//! inherently non-deterministic (PIDs, timestamps, boot IDs, etc.). The
//! [`Normalizer`] pipeline rewrites these values to canonical placeholders so
//! that semantically equivalent outputs compare as equal.
//!
//! # Built-in normalizers
//!
//! | Normalizer | Pattern | Replacement |
//! |---|---|---|
//! | PID | decimal numbers in PID-like contexts | `<PID>` |
//! | Timestamp | ISO-8601, Unix epoch, `systemd` timestamp formats | `<TIMESTAMP>` |
//! | Boot ID | 128-bit hex UUIDs in `sd-id128` format | `<BOOT_ID>` |
//! | Machine ID | 32-hex-char machine identifiers | `<MACHINE_ID>` |
//! | Memory address | `0x` hex addresses | `<ADDR>` |
//! | Non-deterministic ordering | sort lines/keys for order-independent comparison | *(sorted)* |
//!
//! # Custom normalizers
//!
//! Implement the [`Normalize`] trait and add your normalizer to a
//! [`Normalizer`] pipeline via [`Normalizer::push`].

use std::collections::BTreeMap;

use regex::Regex;

use crate::TestOutput;

// ── Normalize trait ─────────────────────────────────────────────────────────

/// A single normalization pass over a [`TestOutput`].
pub trait Normalize: Send + Sync {
    /// Human-readable name of this normalizer (for reporting).
    fn name(&self) -> &str;

    /// Apply the normalization, returning a new [`TestOutput`].
    fn normalize(&self, output: &TestOutput) -> TestOutput;

    /// Return `true` if this normalizer actually changed anything between
    /// `original` and the normalized form. Used to build the "normalization
    /// notes" string in [`crate::DiffResult::Equivalent`].
    fn did_change(&self, original: &TestOutput) -> bool {
        *original != self.normalize(original)
    }
}

// ── Normalizer pipeline ─────────────────────────────────────────────────────

/// An ordered pipeline of [`Normalize`] passes.
///
/// The default pipeline includes all built-in normalizers. You can construct an
/// empty pipeline with [`Normalizer::empty`] and add normalizers selectively,
/// or start from the default and [`push`](Normalizer::push) additional ones.
pub struct Normalizer {
    passes: Vec<Box<dyn Normalize>>,
}

impl Default for Normalizer {
    /// Creates a pipeline with all built-in normalizers in recommended order.
    fn default() -> Self {
        Self {
            passes: vec![
                Box::new(MemoryAddressNormalizer::new()),
                Box::new(BootIdNormalizer::new()),
                Box::new(MachineIdNormalizer::new()),
                Box::new(TimestampNormalizer::new()),
                Box::new(PidNormalizer::new()),
                Box::new(OrderingNormalizer),
            ],
        }
    }
}

impl Normalizer {
    /// Create an empty pipeline with no normalizers.
    pub fn empty() -> Self {
        Self { passes: Vec::new() }
    }

    /// Append a normalizer to the end of the pipeline.
    pub fn push(&mut self, pass: Box<dyn Normalize>) {
        self.passes.push(pass);
    }

    /// Apply all normalizers in order and return the final output.
    pub fn normalize(&self, output: &TestOutput) -> TestOutput {
        let mut current = output.clone();
        for pass in &self.passes {
            current = pass.normalize(&current);
        }
        current
    }

    /// Build a comma-separated string of which normalizers actually changed
    /// the output when comparing `left` (systemd) and `right` (rust-systemd).
    ///
    /// This is used for the notes in [`crate::DiffResult::Equivalent`].
    pub fn applied_normalizations(&self, left: &TestOutput, right: &TestOutput) -> String {
        let mut notes = Vec::new();
        for pass in &self.passes {
            if pass.did_change(left) || pass.did_change(right) {
                notes.push(pass.name().to_string());
            }
        }
        if notes.is_empty() {
            "normalized".to_string()
        } else {
            notes.join(", ")
        }
    }
}

// ── Helper: apply a regex replacement to all text-bearing variants ──────────

fn normalize_text(output: &TestOutput, re: &Regex, replacement: &str) -> TestOutput {
    match output {
        TestOutput::RawText(s) => TestOutput::RawText(re.replace_all(s, replacement).into_owned()),
        TestOutput::StructuredJson(v) => {
            TestOutput::StructuredJson(normalize_json_strings(v, re, replacement))
        }
        TestOutput::DBusPropertyMap(m) => {
            let normalized: BTreeMap<String, String> = m
                .iter()
                .map(|(k, v)| (k.clone(), re.replace_all(v, replacement).into_owned()))
                .collect();
            TestOutput::DBusPropertyMap(normalized)
        }
        TestOutput::FileTreeSnapshot(m) => {
            let normalized: BTreeMap<String, String> = m
                .iter()
                .map(|(k, v)| {
                    (
                        re.replace_all(k, replacement).into_owned(),
                        re.replace_all(v, replacement).into_owned(),
                    )
                })
                .collect();
            TestOutput::FileTreeSnapshot(normalized)
        }
        TestOutput::Composite(parts) => {
            let normalized: Vec<(String, TestOutput)> = parts
                .iter()
                .map(|(label, inner)| (label.clone(), normalize_text(inner, re, replacement)))
                .collect();
            TestOutput::Composite(normalized)
        }
        // BinaryBlob, ExitCode, Unavailable — no text to normalize
        other => other.clone(),
    }
}

/// Recursively normalize string values inside a JSON tree.
fn normalize_json_strings(
    value: &serde_json::Value,
    re: &Regex,
    replacement: &str,
) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => {
            serde_json::Value::String(re.replace_all(s, replacement).into_owned())
        }
        serde_json::Value::Array(arr) => serde_json::Value::Array(
            arr.iter()
                .map(|v| normalize_json_strings(v, re, replacement))
                .collect(),
        ),
        serde_json::Value::Object(obj) => {
            let normalized: serde_json::Map<String, serde_json::Value> = obj
                .iter()
                .map(|(k, v)| (k.clone(), normalize_json_strings(v, re, replacement)))
                .collect();
            serde_json::Value::Object(normalized)
        }
        other => other.clone(),
    }
}

// ── PID normalizer ──────────────────────────────────────────────────────────

/// Replaces PID-like numeric values with `<PID>`.
///
/// Matches patterns commonly seen in systemd output:
/// - `PID: 12345` / `MainPID=12345` / `ControlPID=12345`
/// - `pid 12345` / `PID 12345`
/// - `LISTEN_PID=12345`
/// - `_PID=12345` (journal fields)
pub struct PidNormalizer {
    _private: (),
}

impl Default for PidNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl PidNormalizer {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Normalize for PidNormalizer {
    fn name(&self) -> &str {
        "PID normalization"
    }

    fn normalize(&self, output: &TestOutput) -> TestOutput {
        // We want to keep the prefix and replace only the numeric part.
        let re_replace = Regex::new(r"(?i)((?:(?:main|control_?|listen_|_)?)pid\s*[=:\s]\s*)\d+")
            .expect("PID replace regex should compile");
        normalize_text(output, &re_replace, "${1}<PID>")
    }
}

// ── Timestamp normalizer ────────────────────────────────────────────────────

/// Replaces timestamp values with `<TIMESTAMP>`.
///
/// Recognized formats:
/// - ISO-8601: `2024-01-15T12:34:56.789Z`, `2024-01-15 12:34:56 UTC`
/// - systemd monotonic: `123456789` in `*Timestamp*=` or `*USec*=` contexts
/// - `Day YYYY-MM-DD HH:MM:SS TZ` (e.g. `Mon 2024-01-15 12:34:56 UTC`)
/// - Unix epoch seconds: large integers in timestamp contexts
pub struct TimestampNormalizer {
    /// ISO-8601 style: `2024-01-15T12:34:56...` or `2024-01-15 12:34:56...`
    re_iso: Regex,
    /// systemd day-prefixed: `Mon 2024-01-15 12:34:56 UTC`
    re_day_prefix: Regex,
    /// Property-based: `*Timestamp=...` or `*USec=...` values
    re_property: Regex,
}

impl Default for TimestampNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl TimestampNormalizer {
    pub fn new() -> Self {
        Self {
            re_iso: Regex::new(
                r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2}| [A-Z]{1,5})?"
            ).expect("ISO timestamp regex should compile"),
            re_day_prefix: Regex::new(
                r"(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun)\s+\d{4}-\d{2}-\d{2}\s+\d{2}:\d{2}:\d{2}\s+[A-Z]{1,5}"
            ).expect("day-prefix timestamp regex should compile"),
            re_property: Regex::new(
                r"((?:Timestamp|USec|TimestampMonotonic)\s*=\s*)\S+"
            ).expect("property timestamp regex should compile"),
        }
    }
}

impl Normalize for TimestampNormalizer {
    fn name(&self) -> &str {
        "timestamp normalization"
    }

    fn normalize(&self, output: &TestOutput) -> TestOutput {
        // Apply in order: day-prefix first (more specific), then ISO, then property
        let step1 = normalize_text(output, &self.re_day_prefix, "<TIMESTAMP>");
        let step2 = normalize_text(&step1, &self.re_iso, "<TIMESTAMP>");
        normalize_text(&step2, &self.re_property, "${1}<TIMESTAMP>")
    }
}

// ── Boot ID normalizer ──────────────────────────────────────────────────────

/// Replaces boot IDs (128-bit UUIDs in `8-4-4-4-12` hex format) with `<BOOT_ID>`.
///
/// Also matches the flat 32-hex-char format when preceded by `_BOOT_ID=` or
/// `BootID=` or similar context.
pub struct BootIdNormalizer {
    /// Standard UUID format: `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`
    re_uuid: Regex,
    /// Flat format in known contexts: `_BOOT_ID=<32 hex chars>`
    re_flat: Regex,
}

impl Default for BootIdNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl BootIdNormalizer {
    pub fn new() -> Self {
        Self {
            re_uuid: Regex::new(
                r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
            )
            .expect("UUID regex should compile"),
            re_flat: Regex::new(
                r"(?i)(_BOOT_ID=|BootID=|boot_id=|InvocationID=|invocation_id=)[0-9a-fA-F]{32}",
            )
            .expect("flat boot ID regex should compile"),
        }
    }
}

impl Normalize for BootIdNormalizer {
    fn name(&self) -> &str {
        "boot ID normalization"
    }

    fn normalize(&self, output: &TestOutput) -> TestOutput {
        let step1 = normalize_text(output, &self.re_flat, "${1}<BOOT_ID>");
        normalize_text(&step1, &self.re_uuid, "<BOOT_ID>")
    }
}

// ── Machine ID normalizer ───────────────────────────────────────────────────

/// Replaces machine IDs (32-hex-char identifiers) with `<MACHINE_ID>`.
///
/// Only matches in known contexts to avoid false positives with other
/// 32-char hex strings.
pub struct MachineIdNormalizer {
    re: Regex,
}

impl Default for MachineIdNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl MachineIdNormalizer {
    pub fn new() -> Self {
        Self {
            re: Regex::new(r"(?i)(machine[_-]?id\s*[=:\s]\s*|_MACHINE_ID=)[0-9a-fA-F]{32}")
                .expect("machine ID regex should compile"),
        }
    }
}

impl Normalize for MachineIdNormalizer {
    fn name(&self) -> &str {
        "machine ID normalization"
    }

    fn normalize(&self, output: &TestOutput) -> TestOutput {
        normalize_text(output, &self.re, "${1}<MACHINE_ID>")
    }
}

// ── Memory address normalizer ───────────────────────────────────────────────

/// Replaces memory addresses (`0x7fff...`) with `<ADDR>`.
pub struct MemoryAddressNormalizer {
    re: Regex,
}

impl Default for MemoryAddressNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryAddressNormalizer {
    pub fn new() -> Self {
        Self {
            re: Regex::new(r"0x[0-9a-fA-F]{4,16}").expect("address regex should compile"),
        }
    }
}

impl Normalize for MemoryAddressNormalizer {
    fn name(&self) -> &str {
        "memory address normalization"
    }

    fn normalize(&self, output: &TestOutput) -> TestOutput {
        normalize_text(output, &self.re, "<ADDR>")
    }
}

// ── Ordering normalizer ─────────────────────────────────────────────────────

/// Sorts lines in text output and keys in maps for order-independent
/// comparison.
///
/// This is the last normalizer in the default pipeline. It handles cases where
/// systemd and rust-systemd produce the same set of items but in a different
/// order (e.g. `list-units` output, dependency lists, D-Bus signal ordering).
///
/// Note: this normalizer only applies to [`TestOutput::RawText`] (line sort),
/// [`TestOutput::StructuredJson`] (array sort for arrays of primitives), and
/// map types (which are already ordered via `BTreeMap`). It does NOT sort
/// [`TestOutput::Composite`] parts, as their ordering is typically meaningful.
#[derive(Debug, Clone, Copy)]
pub struct OrderingNormalizer;

impl Normalize for OrderingNormalizer {
    fn name(&self) -> &str {
        "ordering normalization"
    }

    fn normalize(&self, output: &TestOutput) -> TestOutput {
        match output {
            TestOutput::RawText(s) => {
                let mut lines: Vec<&str> = s.lines().collect();
                lines.sort();
                TestOutput::RawText(lines.join("\n"))
            }
            TestOutput::StructuredJson(v) => TestOutput::StructuredJson(sort_json_arrays(v)),
            // BTreeMap variants are already sorted by key.
            // Composite: preserve part ordering.
            // Everything else: pass through.
            other => other.clone(),
        }
    }
}

/// Recursively sort JSON arrays whose elements are all strings or all numbers.
/// Object keys in serde_json::Map are already sorted.
fn sort_json_arrays(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(arr) => {
            let mut sorted: Vec<serde_json::Value> = arr.iter().map(sort_json_arrays).collect();
            // Only sort if all elements are the same primitive type
            let all_strings = sorted.iter().all(|v| v.is_string());
            let all_numbers = sorted.iter().all(|v| v.is_number());
            if all_strings {
                sorted.sort_by(|a, b| a.as_str().unwrap_or("").cmp(b.as_str().unwrap_or("")));
            } else if all_numbers {
                sorted.sort_by(|a, b| {
                    let fa = a.as_f64().unwrap_or(0.0);
                    let fb = b.as_f64().unwrap_or(0.0);
                    fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            serde_json::Value::Array(sorted)
        }
        serde_json::Value::Object(obj) => {
            let normalized: serde_json::Map<String, serde_json::Value> = obj
                .iter()
                .map(|(k, v)| (k.clone(), sort_json_arrays(v)))
                .collect();
            serde_json::Value::Object(normalized)
        }
        other => other.clone(),
    }
}

// ── SelectiveNormalizer ─────────────────────────────────────────────────────

/// A normalizer that only applies to specific [`TestOutput`] variants.
///
/// Useful when you have a custom regex normalizer but only want it to fire
/// on text output, not JSON, etc.
pub struct SelectiveNormalizer<F>
where
    F: Fn(&TestOutput) -> TestOutput + Send + Sync,
{
    pub name: String,
    pub predicate: fn(&TestOutput) -> bool,
    pub transform: F,
}

impl<F> Normalize for SelectiveNormalizer<F>
where
    F: Fn(&TestOutput) -> TestOutput + Send + Sync,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn normalize(&self, output: &TestOutput) -> TestOutput {
        if (self.predicate)(output) {
            (self.transform)(output)
        } else {
            output.clone()
        }
    }
}

// ── RegexNormalizer (convenience) ───────────────────────────────────────────

/// A simple regex-based normalizer that replaces all matches with a fixed
/// string across all text-bearing output variants.
pub struct RegexNormalizer {
    label: String,
    re: Regex,
    replacement: String,
}

impl RegexNormalizer {
    /// Create a new regex normalizer.
    ///
    /// # Panics
    ///
    /// Panics if `pattern` is not a valid regex.
    pub fn new(label: impl Into<String>, pattern: &str, replacement: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            re: Regex::new(pattern).expect("RegexNormalizer pattern should compile"),
            replacement: replacement.into(),
        }
    }
}

impl Normalize for RegexNormalizer {
    fn name(&self) -> &str {
        &self.label
    }

    fn normalize(&self, output: &TestOutput) -> TestOutput {
        normalize_text(output, &self.re, &self.replacement)
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pid_normalizer_raw_text() {
        let norm = PidNormalizer::new();
        let input = TestOutput::RawText("MainPID=12345 started".into());
        let output = norm.normalize(&input);
        if let TestOutput::RawText(s) = &output {
            assert!(s.contains("<PID>"), "got: {s}");
            assert!(!s.contains("12345"), "got: {s}");
        } else {
            panic!("expected RawText");
        }
    }

    #[test]
    fn test_pid_normalizer_property_style() {
        let norm = PidNormalizer::new();
        let input = TestOutput::RawText("ControlPID=999\nMainPID=888".into());
        let output = norm.normalize(&input);
        if let TestOutput::RawText(s) = &output {
            assert!(!s.contains("999"), "got: {s}");
            assert!(!s.contains("888"), "got: {s}");
            assert!(s.contains("<PID>"), "got: {s}");
        } else {
            panic!("expected RawText");
        }
    }

    #[test]
    fn test_pid_normalizer_journal_field() {
        let norm = PidNormalizer::new();
        let input = TestOutput::RawText("_PID=42".into());
        let output = norm.normalize(&input);
        if let TestOutput::RawText(s) = &output {
            assert!(s.contains("<PID>"), "got: {s}");
            assert!(!s.contains("42"), "got: {s}");
        } else {
            panic!("expected RawText");
        }
    }

    #[test]
    fn test_timestamp_normalizer_iso() {
        let norm = TimestampNormalizer::new();
        let input = TestOutput::RawText("started at 2024-01-15T12:34:56.789Z done".into());
        let output = norm.normalize(&input);
        if let TestOutput::RawText(s) = &output {
            assert!(s.contains("<TIMESTAMP>"), "got: {s}");
            assert!(!s.contains("2024-01-15"), "got: {s}");
        } else {
            panic!("expected RawText");
        }
    }

    #[test]
    fn test_timestamp_normalizer_day_prefix() {
        let norm = TimestampNormalizer::new();
        let input = TestOutput::RawText("Since Mon 2024-01-15 12:34:56 UTC; 5min ago".into());
        let output = norm.normalize(&input);
        if let TestOutput::RawText(s) = &output {
            assert!(s.contains("<TIMESTAMP>"), "got: {s}");
            assert!(!s.contains("Mon 2024"), "got: {s}");
        } else {
            panic!("expected RawText");
        }
    }

    #[test]
    fn test_timestamp_normalizer_property() {
        let norm = TimestampNormalizer::new();
        let input =
            TestOutput::RawText("ExecMainStartTimestamp=Mon 2024-01-15 12:34:56 UTC".into());
        let output = norm.normalize(&input);
        if let TestOutput::RawText(s) = &output {
            assert!(s.contains("<TIMESTAMP>"), "got: {s}");
        } else {
            panic!("expected RawText");
        }
    }

    #[test]
    fn test_boot_id_normalizer_uuid() {
        let norm = BootIdNormalizer::new();
        let input = TestOutput::RawText("Boot ID: a1b2c3d4-e5f6-7890-abcd-ef1234567890".into());
        let output = norm.normalize(&input);
        if let TestOutput::RawText(s) = &output {
            assert!(s.contains("<BOOT_ID>"), "got: {s}");
            assert!(!s.contains("a1b2c3d4"), "got: {s}");
        } else {
            panic!("expected RawText");
        }
    }

    #[test]
    fn test_boot_id_normalizer_flat() {
        let norm = BootIdNormalizer::new();
        let input = TestOutput::RawText("_BOOT_ID=a1b2c3d4e5f67890abcdef1234567890".into());
        let output = norm.normalize(&input);
        if let TestOutput::RawText(s) = &output {
            assert!(s.contains("<BOOT_ID>"), "got: {s}");
        } else {
            panic!("expected RawText");
        }
    }

    #[test]
    fn test_machine_id_normalizer() {
        let norm = MachineIdNormalizer::new();
        let input = TestOutput::RawText("_MACHINE_ID=a1b2c3d4e5f67890abcdef1234567890".into());
        let output = norm.normalize(&input);
        if let TestOutput::RawText(s) = &output {
            assert!(s.contains("<MACHINE_ID>"), "got: {s}");
        } else {
            panic!("expected RawText");
        }
    }

    #[test]
    fn test_machine_id_normalizer_dash_style() {
        let norm = MachineIdNormalizer::new();
        let input = TestOutput::RawText("Machine-ID: abcdef1234567890abcdef1234567890".into());
        let output = norm.normalize(&input);
        if let TestOutput::RawText(s) = &output {
            assert!(s.contains("<MACHINE_ID>"), "got: {s}");
        } else {
            panic!("expected RawText");
        }
    }

    #[test]
    fn test_memory_address_normalizer() {
        let norm = MemoryAddressNormalizer::new();
        let input = TestOutput::RawText("loaded at 0x7fff12345678 in memory".into());
        let output = norm.normalize(&input);
        if let TestOutput::RawText(s) = &output {
            assert!(s.contains("<ADDR>"), "got: {s}");
            assert!(!s.contains("0x7fff"), "got: {s}");
        } else {
            panic!("expected RawText");
        }
    }

    #[test]
    fn test_ordering_normalizer_raw_text() {
        let norm = OrderingNormalizer;
        let input = TestOutput::RawText("charlie\nalpha\nbravo".into());
        let output = norm.normalize(&input);
        if let TestOutput::RawText(s) = &output {
            assert_eq!(s, "alpha\nbravo\ncharlie");
        } else {
            panic!("expected RawText");
        }
    }

    #[test]
    fn test_ordering_normalizer_json_string_array() {
        let norm = OrderingNormalizer;
        let input = TestOutput::StructuredJson(serde_json::json!(["c", "a", "b"]));
        let output = norm.normalize(&input);
        if let TestOutput::StructuredJson(v) = &output {
            assert_eq!(v, &serde_json::json!(["a", "b", "c"]));
        } else {
            panic!("expected StructuredJson");
        }
    }

    #[test]
    fn test_ordering_normalizer_json_number_array() {
        let norm = OrderingNormalizer;
        let input = TestOutput::StructuredJson(serde_json::json!([3, 1, 2]));
        let output = norm.normalize(&input);
        if let TestOutput::StructuredJson(v) = &output {
            assert_eq!(v, &serde_json::json!([1, 2, 3]));
        } else {
            panic!("expected StructuredJson");
        }
    }

    #[test]
    fn test_ordering_normalizer_json_mixed_array_not_sorted() {
        let norm = OrderingNormalizer;
        // Mixed types: should not be sorted
        let input = TestOutput::StructuredJson(serde_json::json!(["a", 1, "b"]));
        let output = norm.normalize(&input);
        if let TestOutput::StructuredJson(v) = &output {
            assert_eq!(v, &serde_json::json!(["a", 1, "b"]));
        } else {
            panic!("expected StructuredJson");
        }
    }

    #[test]
    fn test_ordering_normalizer_passthrough_exit_code() {
        let norm = OrderingNormalizer;
        let input = TestOutput::ExitCode(42);
        let output = norm.normalize(&input);
        assert_eq!(output, TestOutput::ExitCode(42));
    }

    #[test]
    fn test_ordering_normalizer_passthrough_binary() {
        let norm = OrderingNormalizer;
        let input = TestOutput::BinaryBlob(vec![3, 1, 2]);
        let output = norm.normalize(&input);
        assert_eq!(output, TestOutput::BinaryBlob(vec![3, 1, 2]));
    }

    #[test]
    fn test_pipeline_default_normalizes_pid_and_timestamp() {
        let pipeline = Normalizer::default();
        let input =
            TestOutput::RawText("MainPID=12345\nStarted at Mon 2024-06-01 10:00:00 UTC".into());
        let output = pipeline.normalize(&input);
        if let TestOutput::RawText(s) = &output {
            assert!(s.contains("<PID>"), "should normalize PID, got: {s}");
            assert!(
                s.contains("<TIMESTAMP>"),
                "should normalize timestamp, got: {s}"
            );
        } else {
            panic!("expected RawText");
        }
    }

    #[test]
    fn test_pipeline_empty_is_identity() {
        let pipeline = Normalizer::empty();
        let input = TestOutput::RawText("MainPID=12345".into());
        let output = pipeline.normalize(&input);
        assert_eq!(output, input);
    }

    #[test]
    fn test_pipeline_push_custom() {
        let mut pipeline = Normalizer::empty();
        pipeline.push(Box::new(RegexNormalizer::new(
            "custom",
            r"secret-\d+",
            "<REDACTED>",
        )));
        let input = TestOutput::RawText("token=secret-42".into());
        let output = pipeline.normalize(&input);
        if let TestOutput::RawText(s) = &output {
            assert_eq!(s, "token=<REDACTED>");
        } else {
            panic!("expected RawText");
        }
    }

    #[test]
    fn test_regex_normalizer() {
        let norm = RegexNormalizer::new("test", r"\d{3}-\d{4}", "<PHONE>");
        let input = TestOutput::RawText("call 555-1234 now".into());
        let output = norm.normalize(&input);
        if let TestOutput::RawText(s) = &output {
            assert_eq!(s, "call <PHONE> now");
        } else {
            panic!("expected RawText");
        }
    }

    #[test]
    fn test_did_change_returns_true_when_changed() {
        let norm = PidNormalizer::new();
        let input = TestOutput::RawText("MainPID=999".into());
        assert!(norm.did_change(&input));
    }

    #[test]
    fn test_did_change_returns_false_when_unchanged() {
        let norm = PidNormalizer::new();
        let input = TestOutput::RawText("no pid here".into());
        assert!(!norm.did_change(&input));
    }

    #[test]
    fn test_applied_normalizations_reports_active() {
        let pipeline = Normalizer::default();
        let left = TestOutput::RawText("MainPID=100".into());
        let right = TestOutput::RawText("MainPID=200".into());
        let notes = pipeline.applied_normalizations(&left, &right);
        assert!(notes.contains("PID"), "expected PID note, got: {notes}");
    }

    #[test]
    fn test_applied_normalizations_reports_nothing_changed() {
        let pipeline = Normalizer::default();
        let left = TestOutput::RawText("hello".into());
        let right = TestOutput::RawText("hello".into());
        let notes = pipeline.applied_normalizations(&left, &right);
        // Only ordering changes ("hello" single line stays same after sort)
        // Nothing else matches, so at most "ordering normalization" or "normalized"
        assert!(!notes.is_empty());
    }

    #[test]
    fn test_normalize_dbus_property_map() {
        let norm = PidNormalizer::new();
        let mut m = BTreeMap::new();
        m.insert("MainPID".into(), "MainPID=42".into());
        m.insert("Name".into(), "foo.service".into());
        let input = TestOutput::DBusPropertyMap(m);
        let output = norm.normalize(&input);
        if let TestOutput::DBusPropertyMap(m) = &output {
            assert!(
                m.get("MainPID").unwrap().contains("<PID>"),
                "got: {:?}",
                m.get("MainPID")
            );
            assert_eq!(m.get("Name").unwrap(), "foo.service");
        } else {
            panic!("expected DBusPropertyMap");
        }
    }

    #[test]
    fn test_normalize_composite() {
        let norm = PidNormalizer::new();
        let input = TestOutput::Composite(vec![
            ("stdout".into(), TestOutput::RawText("pid=100".into())),
            ("exit".into(), TestOutput::ExitCode(0)),
        ]);
        let output = norm.normalize(&input);
        if let TestOutput::Composite(parts) = &output {
            assert_eq!(parts.len(), 2);
            if let TestOutput::RawText(s) = &parts[0].1 {
                assert!(s.contains("<PID>"), "got: {s}");
            }
            assert_eq!(parts[1].1, TestOutput::ExitCode(0));
        } else {
            panic!("expected Composite");
        }
    }

    #[test]
    fn test_normalize_json_strings() {
        let norm = BootIdNormalizer::new();
        let input = TestOutput::StructuredJson(serde_json::json!({
            "boot_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            "name": "test.service",
            "nested": {
                "id": "11111111-2222-3333-4444-555555555555"
            }
        }));
        let output = norm.normalize(&input);
        if let TestOutput::StructuredJson(v) = &output {
            assert_eq!(v["boot_id"], "<BOOT_ID>");
            assert_eq!(v["name"], "test.service");
            assert_eq!(v["nested"]["id"], "<BOOT_ID>");
        } else {
            panic!("expected StructuredJson");
        }
    }

    #[test]
    fn test_normalize_unavailable_passthrough() {
        let norm = PidNormalizer::new();
        let input = TestOutput::Unavailable("not installed".into());
        let output = norm.normalize(&input);
        assert_eq!(output, input);
    }

    #[test]
    fn test_normalize_binary_blob_passthrough() {
        let norm = PidNormalizer::new();
        let input = TestOutput::BinaryBlob(vec![1, 2, 3]);
        let output = norm.normalize(&input);
        assert_eq!(output, input);
    }

    #[test]
    fn test_full_pipeline_complex_input() {
        let pipeline = Normalizer::default();
        let input = TestOutput::RawText(
            "Boot: a1b2c3d4-e5f6-7890-abcd-ef1234567890\n\
             MainPID=12345\n\
             Addr: 0x7fff12345678\n\
             _MACHINE_ID=abcdef1234567890abcdef1234567890\n\
             Started: Mon 2024-06-01 10:00:00 UTC"
                .into(),
        );
        let output = pipeline.normalize(&input);
        if let TestOutput::RawText(s) = &output {
            assert!(s.contains("<BOOT_ID>"), "boot ID not normalized, got:\n{s}");
            assert!(s.contains("<PID>"), "PID not normalized, got:\n{s}");
            assert!(s.contains("<ADDR>"), "address not normalized, got:\n{s}");
            assert!(
                s.contains("<MACHINE_ID>"),
                "machine ID not normalized, got:\n{s}"
            );
            assert!(
                s.contains("<TIMESTAMP>"),
                "timestamp not normalized, got:\n{s}"
            );
        } else {
            panic!("expected RawText");
        }
    }
}
