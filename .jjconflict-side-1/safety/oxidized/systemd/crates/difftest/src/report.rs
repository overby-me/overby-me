//! Report generation for differential test results.
//!
//! Produces structured reports from a collection of [`TestRecord`] results,
//! enabling integration with CI systems and human review workflows.
//!
//! # Output formats
//!
//! - **JUnit XML** — Standard CI format consumed by GitHub Actions, GitLab CI,
//!   Jenkins, and most CI platforms. Produced by [`JunitReport`].
//! - **JSON** — Machine-readable format for custom tooling, dashboards, and
//!   PR comment generation. Produced by [`JsonReport`].
//! - **Summary text** — Human-readable terminal output with ANSI colors.
//!   Produced by [`SummaryReport`].
//!
//! # Usage
//!
//! ```ignore
//! use difftest::report::{JunitReport, JsonReport, SummaryReport, ReportGenerator};
//! use difftest::TestRecord;
//!
//! let records: Vec<TestRecord> = run_all_tests();
//!
//! // Write JUnit XML for CI
//! let junit = JunitReport::new(&records);
//! junit.write_to_file("test-results.xml").unwrap();
//!
//! // Write JSON for tooling
//! let json = JsonReport::new(&records);
//! json.write_to_file("test-results.json").unwrap();
//!
//! // Print summary to terminal
//! let summary = SummaryReport::new(&records);
//! println!("{summary}");
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{DiffResult, DiffTestError, TestRecord};

// ── ReportGenerator trait ───────────────────────────────────────────────────

/// Common interface for all report generators.
pub trait ReportGenerator {
    /// Render the report as a byte vector.
    fn render(&self) -> Result<Vec<u8>, DiffTestError>;

    /// Write the report to a file, creating parent directories as needed.
    fn write_to_file(&self, path: impl AsRef<Path>) -> Result<(), DiffTestError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DiffTestError::Io(format!(
                    "creating report directory {}: {e}",
                    parent.display()
                ))
            })?;
        }
        let content = self.render()?;
        std::fs::write(path, &content)
            .map_err(|e| DiffTestError::Io(format!("writing report to {}: {e}", path.display())))?;
        Ok(())
    }

    /// Write the report to an arbitrary [`Write`] sink.
    fn write_to(&self, mut writer: impl Write) -> Result<(), DiffTestError> {
        let content = self.render()?;
        writer
            .write_all(&content)
            .map_err(|e| DiffTestError::Io(format!("writing report: {e}")))?;
        Ok(())
    }
}

// ── Aggregate statistics ────────────────────────────────────────────────────

/// Aggregate statistics computed from a collection of test records.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReportStats {
    /// Total number of tests executed.
    pub total: usize,
    /// Number of tests with identical outputs.
    pub identical: usize,
    /// Number of tests with equivalent (normalized) outputs.
    pub equivalent: usize,
    /// Number of tests with divergent outputs.
    pub divergent: usize,
    /// Number of skipped tests.
    pub skipped: usize,
    /// Number of divergences that were previously known.
    pub known_divergences: usize,
    /// Number of **new** divergences (not previously known).
    pub new_divergences: usize,
    /// Total wall-clock duration in milliseconds.
    pub total_duration_ms: u64,
    /// Per-category breakdown: `category → (pass_count, fail_count, skip_count)`.
    pub by_category: BTreeMap<String, CategoryStats>,
}

/// Per-category statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CategoryStats {
    pub total: usize,
    pub pass: usize,
    pub fail: usize,
    pub skip: usize,
    pub duration_ms: u64,
}

impl ReportStats {
    /// Compute statistics from a slice of test records.
    pub fn from_records(records: &[TestRecord]) -> Self {
        let mut stats = Self {
            total: records.len(),
            ..Self::default()
        };

        for rec in records {
            stats.total_duration_ms += rec.duration_ms;

            match &rec.result {
                DiffResult::Identical => stats.identical += 1,
                DiffResult::Equivalent(_) => stats.equivalent += 1,
                DiffResult::Divergent(_) => {
                    stats.divergent += 1;
                    if rec.known_divergence {
                        stats.known_divergences += 1;
                    } else {
                        stats.new_divergences += 1;
                    }
                }
                DiffResult::Skipped(_) => stats.skipped += 1,
            }

            let cat = stats.by_category.entry(rec.category.clone()).or_default();
            cat.total += 1;
            cat.duration_ms += rec.duration_ms;
            if rec.result.is_pass() {
                cat.pass += 1;
            } else if rec.result.is_skipped() {
                cat.skip += 1;
            } else {
                cat.fail += 1;
            }
        }

        stats
    }

    /// The number of passing tests (identical + equivalent).
    pub fn pass_count(&self) -> usize {
        self.identical + self.equivalent
    }

    /// Returns `true` if there are no new (unknown) divergences.
    pub fn is_success(&self) -> bool {
        self.new_divergences == 0
    }
}

impl fmt::Display for ReportStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Total:       {}", self.total)?;
        writeln!(f, "Identical:   {}", self.identical)?;
        writeln!(f, "Equivalent:  {}", self.equivalent)?;
        writeln!(f, "Divergent:   {}", self.divergent)?;
        writeln!(f, "  Known:     {}", self.known_divergences)?;
        writeln!(f, "  New:       {}", self.new_divergences)?;
        writeln!(f, "Skipped:     {}", self.skipped)?;
        writeln!(f, "Duration:    {}ms", self.total_duration_ms)?;
        Ok(())
    }
}

// ── JUnit XML Report ────────────────────────────────────────────────────────

/// Generates a JUnit XML report compatible with CI systems.
///
/// The report structure follows the JUnit XML schema:
///
/// ```xml
/// <?xml version="1.0" encoding="UTF-8"?>
/// <testsuites name="difftest" tests="N" failures="N" errors="0" skipped="N" time="T">
///   <testsuite name="category_name" tests="N" failures="N" skipped="N" time="T">
///     <testcase name="test_name" classname="category_name" time="T">
///       <!-- for failures: -->
///       <failure message="...">detailed explanation</failure>
///       <!-- for skipped: -->
///       <skipped message="reason"/>
///     </testcase>
///   </testsuite>
/// </testsuites>
/// ```
pub struct JunitReport<'a> {
    records: &'a [TestRecord],
    suite_name: String,
}

impl<'a> JunitReport<'a> {
    /// Create a new JUnit report from test records.
    pub fn new(records: &'a [TestRecord]) -> Self {
        Self {
            records,
            suite_name: "difftest".to_string(),
        }
    }

    /// Override the top-level test suite name (default: `"difftest"`).
    pub fn with_suite_name(mut self, name: impl Into<String>) -> Self {
        self.suite_name = name.into();
        self
    }

    /// Group records by category.
    fn by_category(&self) -> BTreeMap<String, Vec<&TestRecord>> {
        let mut groups: BTreeMap<String, Vec<&TestRecord>> = BTreeMap::new();
        for rec in self.records {
            groups.entry(rec.category.clone()).or_default().push(rec);
        }
        groups
    }
}

impl<'a> ReportGenerator for JunitReport<'a> {
    fn render(&self) -> Result<Vec<u8>, DiffTestError> {
        let stats = ReportStats::from_records(self.records);
        let groups = self.by_category();
        let total_time_secs = stats.total_duration_ms as f64 / 1000.0;

        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str(&format!(
            "<testsuites name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"0\" skipped=\"{}\" time=\"{:.3}\">\n",
            xml_escape(&self.suite_name),
            stats.total,
            stats.new_divergences,
            stats.skipped,
            total_time_secs,
        ));

        for (category, records) in &groups {
            let cat_stats = stats.by_category.get(category);
            let cat_failures = cat_stats.map(|c| c.fail).unwrap_or(0);
            let cat_skipped = cat_stats.map(|c| c.skip).unwrap_or(0);
            let cat_time = cat_stats
                .map(|c| c.duration_ms as f64 / 1000.0)
                .unwrap_or(0.0);

            xml.push_str(&format!(
                "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" skipped=\"{}\" time=\"{:.3}\">\n",
                xml_escape(category),
                records.len(),
                cat_failures,
                cat_skipped,
                cat_time,
            ));

            for rec in records {
                let test_time = rec.duration_ms as f64 / 1000.0;
                xml.push_str(&format!(
                    "    <testcase name=\"{}\" classname=\"{}\" time=\"{:.3}\"",
                    xml_escape(&rec.name),
                    xml_escape(category),
                    test_time,
                ));

                match &rec.result {
                    DiffResult::Identical | DiffResult::Equivalent(_) => {
                        xml.push_str("/>\n");
                    }
                    DiffResult::Divergent(explanation) => {
                        if rec.known_divergence {
                            // Known divergences are reported as passing with a system-out note
                            xml.push_str(">\n");
                            xml.push_str("      <system-out>");
                            xml.push_str(&xml_escape(&format!(
                                "Known divergence: {}\nReason: {}",
                                explanation,
                                rec.known_reason
                                    .as_deref()
                                    .unwrap_or("(no reason recorded)")
                            )));
                            xml.push_str("</system-out>\n");
                            xml.push_str("    </testcase>\n");
                        } else {
                            xml.push_str(">\n");
                            let message = truncate_for_xml(explanation, 200);
                            xml.push_str(&format!(
                                "      <failure message=\"{}\">{}</failure>\n",
                                xml_escape(&message),
                                xml_escape(explanation),
                            ));
                            xml.push_str("    </testcase>\n");
                        }
                    }
                    DiffResult::Skipped(reason) => {
                        xml.push_str(">\n");
                        xml.push_str(&format!(
                            "      <skipped message=\"{}\"/>\n",
                            xml_escape(reason),
                        ));
                        xml.push_str("    </testcase>\n");
                    }
                }
            }

            xml.push_str("  </testsuite>\n");
        }

        xml.push_str("</testsuites>\n");
        Ok(xml.into_bytes())
    }
}

// ── JSON Report ─────────────────────────────────────────────────────────────

/// Generates a JSON report for machine consumption.
///
/// The JSON structure:
///
/// ```json
/// {
///   "generated_at": "2024-06-01T12:00:00Z",
///   "suite_name": "difftest",
///   "stats": { ... },
///   "records": [ ... ],
///   "new_divergences": [ ... ],
///   "known_divergences": [ ... ]
/// }
/// ```
pub struct JsonReport<'a> {
    records: &'a [TestRecord],
    suite_name: String,
}

impl<'a> JsonReport<'a> {
    /// Create a new JSON report from test records.
    pub fn new(records: &'a [TestRecord]) -> Self {
        Self {
            records,
            suite_name: "difftest".to_string(),
        }
    }

    /// Override the suite name in the report.
    pub fn with_suite_name(mut self, name: impl Into<String>) -> Self {
        self.suite_name = name.into();
        self
    }
}

/// Serializable JSON report structure.
#[derive(Debug, Serialize, Deserialize)]
struct JsonReportData {
    generated_at: String,
    suite_name: String,
    stats: ReportStats,
    records: Vec<JsonTestRecord>,
    new_divergences: Vec<JsonTestRecord>,
    known_divergences: Vec<JsonTestRecord>,
}

/// A single test record serialized for JSON output.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonTestRecord {
    name: String,
    category: String,
    result: String,
    result_detail: Option<String>,
    duration_ms: u64,
    known_divergence: bool,
    known_reason: Option<String>,
}

impl From<&TestRecord> for JsonTestRecord {
    fn from(rec: &TestRecord) -> Self {
        let (result, result_detail) = match &rec.result {
            DiffResult::Identical => ("identical".to_string(), None),
            DiffResult::Equivalent(notes) => ("equivalent".to_string(), Some(notes.clone())),
            DiffResult::Divergent(explanation) => {
                ("divergent".to_string(), Some(explanation.clone()))
            }
            DiffResult::Skipped(reason) => ("skipped".to_string(), Some(reason.clone())),
        };

        Self {
            name: rec.name.clone(),
            category: rec.category.clone(),
            result,
            result_detail,
            duration_ms: rec.duration_ms,
            known_divergence: rec.known_divergence,
            known_reason: rec.known_reason.clone(),
        }
    }
}

impl<'a> ReportGenerator for JsonReport<'a> {
    fn render(&self) -> Result<Vec<u8>, DiffTestError> {
        let stats = ReportStats::from_records(self.records);

        let all_records: Vec<JsonTestRecord> = self.records.iter().map(Into::into).collect();

        let new_divergences: Vec<JsonTestRecord> = self
            .records
            .iter()
            .filter(|r| r.is_new_failure())
            .map(Into::into)
            .collect();

        let known_divergences: Vec<JsonTestRecord> = self
            .records
            .iter()
            .filter(|r| r.result.is_divergent() && r.known_divergence)
            .map(Into::into)
            .collect();

        let report = JsonReportData {
            generated_at: Utc::now().to_rfc3339(),
            suite_name: self.suite_name.clone(),
            stats,
            records: all_records,
            new_divergences,
            known_divergences,
        };

        serde_json::to_vec_pretty(&report)
            .map_err(|e| DiffTestError::Serialization(format!("JSON report: {e}")))
    }
}

// ── Summary text report ─────────────────────────────────────────────────────

/// Generates a human-readable summary for terminal output.
///
/// Includes:
/// - Overall pass/fail/skip counts
/// - Per-category breakdown
/// - List of new divergences
/// - List of known divergences
/// - Total duration
pub struct SummaryReport<'a> {
    records: &'a [TestRecord],
}

impl<'a> SummaryReport<'a> {
    /// Create a new summary report from test records.
    pub fn new(records: &'a [TestRecord]) -> Self {
        Self { records }
    }
}

impl<'a> fmt::Display for SummaryReport<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let stats = ReportStats::from_records(self.records);

        writeln!(f, "═══ Differential Test Results ═══")?;
        writeln!(f)?;

        // Overall stats
        let status = if stats.is_success() { "PASS" } else { "FAIL" };
        writeln!(f, "Status: {status}")?;
        writeln!(f)?;
        write!(f, "{stats}")?;
        writeln!(f)?;

        // Per-category breakdown
        if !stats.by_category.is_empty() {
            writeln!(f, "By category:")?;
            for (cat, cat_stats) in &stats.by_category {
                writeln!(
                    f,
                    "  {cat}: {} total, {} pass, {} fail, {} skip ({}ms)",
                    cat_stats.total,
                    cat_stats.pass,
                    cat_stats.fail,
                    cat_stats.skip,
                    cat_stats.duration_ms,
                )?;
            }
            writeln!(f)?;
        }

        // New divergences (the important stuff)
        let new_failures: Vec<&TestRecord> =
            self.records.iter().filter(|r| r.is_new_failure()).collect();

        if !new_failures.is_empty() {
            writeln!(f, "New divergences ({}):", new_failures.len())?;
            for rec in &new_failures {
                writeln!(f, "  ✗ {} [{}]", rec.name, rec.category)?;
                if let DiffResult::Divergent(explanation) = &rec.result {
                    // Indent the explanation
                    for line in explanation.lines().take(5) {
                        writeln!(f, "      {line}")?;
                    }
                    let total_lines = explanation.lines().count();
                    if total_lines > 5 {
                        writeln!(f, "      ... ({} more lines)", total_lines - 5)?;
                    }
                }
            }
            writeln!(f)?;
        }

        // Known divergences
        let known: Vec<&TestRecord> = self
            .records
            .iter()
            .filter(|r| r.result.is_divergent() && r.known_divergence)
            .collect();

        if !known.is_empty() {
            writeln!(f, "Known divergences ({}):", known.len())?;
            for rec in &known {
                let reason = rec
                    .known_reason
                    .as_deref()
                    .unwrap_or("(no reason recorded)");
                writeln!(f, "  ⚠ {} [{}]: {reason}", rec.name, rec.category)?;
            }
            writeln!(f)?;
        }

        // Skipped
        let skipped: Vec<&TestRecord> = self
            .records
            .iter()
            .filter(|r| r.result.is_skipped())
            .collect();

        if !skipped.is_empty() {
            writeln!(f, "Skipped ({}):", skipped.len())?;
            for rec in &skipped {
                if let DiffResult::Skipped(reason) = &rec.result {
                    writeln!(f, "  ○ {} [{}]: {reason}", rec.name, rec.category)?;
                }
            }
            writeln!(f)?;
        }

        writeln!(f, "═══ End of Report ═══")?;
        Ok(())
    }
}

impl<'a> ReportGenerator for SummaryReport<'a> {
    fn render(&self) -> Result<Vec<u8>, DiffTestError> {
        Ok(self.to_string().into_bytes())
    }
}

// ── Markdown Report (for PR comments) ───────────────────────────────────────

/// Generates a Markdown-formatted report suitable for posting as a PR comment.
pub struct MarkdownReport<'a> {
    records: &'a [TestRecord],
    suite_name: String,
}

impl<'a> MarkdownReport<'a> {
    /// Create a new Markdown report.
    pub fn new(records: &'a [TestRecord]) -> Self {
        Self {
            records,
            suite_name: "difftest".to_string(),
        }
    }

    /// Override the suite name used in the report heading.
    pub fn with_suite_name(mut self, name: impl Into<String>) -> Self {
        self.suite_name = name.into();
        self
    }
}

impl<'a> ReportGenerator for MarkdownReport<'a> {
    fn render(&self) -> Result<Vec<u8>, DiffTestError> {
        let stats = ReportStats::from_records(self.records);
        let mut md = String::new();

        // Header with status badge
        let status_emoji = if stats.is_success() { "✅" } else { "❌" };
        md.push_str(&format!(
            "## {status_emoji} Differential Test Results — {}\n\n",
            self.suite_name
        ));

        // Summary table
        md.push_str("| Metric | Count |\n");
        md.push_str("|--------|-------|\n");
        md.push_str(&format!("| Total | {} |\n", stats.total));
        md.push_str(&format!("| Identical | {} |\n", stats.identical));
        md.push_str(&format!("| Equivalent | {} |\n", stats.equivalent));
        md.push_str(&format!("| Divergent | {} |\n", stats.divergent));
        md.push_str(&format!("| ↳ Known | {} |\n", stats.known_divergences));
        md.push_str(&format!("| ↳ **New** | **{}** |\n", stats.new_divergences));
        md.push_str(&format!("| Skipped | {} |\n", stats.skipped));
        md.push_str(&format!(
            "| Duration | {:.1}s |\n",
            stats.total_duration_ms as f64 / 1000.0
        ));
        md.push('\n');

        // New divergences
        let new_failures: Vec<&TestRecord> =
            self.records.iter().filter(|r| r.is_new_failure()).collect();

        if !new_failures.is_empty() {
            md.push_str(&format!(
                "### ❌ New Divergences ({})\n\n",
                new_failures.len()
            ));
            for rec in &new_failures {
                md.push_str(&format!(
                    "<details><summary><code>{}</code> [{}]</summary>\n\n",
                    rec.name, rec.category
                ));
                if let DiffResult::Divergent(explanation) = &rec.result {
                    md.push_str("```\n");
                    // Limit to first 50 lines to keep PR comments manageable
                    let lines: Vec<&str> = explanation.lines().take(50).collect();
                    md.push_str(&lines.join("\n"));
                    let total_lines = explanation.lines().count();
                    if total_lines > 50 {
                        md.push_str(&format!("\n... ({} more lines)", total_lines - 50));
                    }
                    md.push_str("\n```\n");
                }
                md.push_str("\n</details>\n\n");
            }
        }

        // Category breakdown
        if stats.by_category.len() > 1 {
            md.push_str("### Per-Category Breakdown\n\n");
            md.push_str("| Category | Total | Pass | Fail | Skip |\n");
            md.push_str("|----------|-------|------|------|------|\n");
            for (cat, cat_stats) in &stats.by_category {
                md.push_str(&format!(
                    "| {} | {} | {} | {} | {} |\n",
                    cat, cat_stats.total, cat_stats.pass, cat_stats.fail, cat_stats.skip,
                ));
            }
            md.push('\n');
        }

        Ok(md.into_bytes())
    }
}

// ── XML helpers ─────────────────────────────────────────────────────────────

/// Escape special characters for XML content and attributes.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Truncate a string for use in an XML attribute `message`, appending `...` if
/// truncated.
fn truncate_for_xml(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_records() -> Vec<TestRecord> {
        vec![
            TestRecord {
                name: "ini_parser_basic".into(),
                category: "unit_parsing".into(),
                result: DiffResult::Identical,
                duration_ms: 50,
                known_divergence: false,
                known_reason: None,
            },
            TestRecord {
                name: "specifier_expansion".into(),
                category: "unit_parsing".into(),
                result: DiffResult::Equivalent("PID normalization".into()),
                duration_ms: 120,
                known_divergence: false,
                known_reason: None,
            },
            TestRecord {
                name: "service_start_stop".into(),
                category: "service_lifecycle".into(),
                result: DiffResult::Divergent("MainPID property differs".into()),
                duration_ms: 500,
                known_divergence: false,
                known_reason: None,
            },
            TestRecord {
                name: "calendar_minutely".into(),
                category: "timer".into(),
                result: DiffResult::Divergent("known ordering issue".into()),
                duration_ms: 30,
                known_divergence: true,
                known_reason: Some("tracked in #42".into()),
            },
            TestRecord {
                name: "dbus_query".into(),
                category: "dbus".into(),
                result: DiffResult::Skipped("no systemd available".into()),
                duration_ms: 1,
                known_divergence: false,
                known_reason: None,
            },
        ]
    }

    #[test]
    fn test_report_stats_from_records() {
        let records = sample_records();
        let stats = ReportStats::from_records(&records);

        assert_eq!(stats.total, 5);
        assert_eq!(stats.identical, 1);
        assert_eq!(stats.equivalent, 1);
        assert_eq!(stats.divergent, 2);
        assert_eq!(stats.skipped, 1);
        assert_eq!(stats.known_divergences, 1);
        assert_eq!(stats.new_divergences, 1);
        assert_eq!(stats.pass_count(), 2);
        assert!(!stats.is_success()); // has a new divergence
    }

    #[test]
    fn test_report_stats_is_success_all_pass() {
        let records = vec![
            TestRecord {
                name: "test1".into(),
                category: "cat".into(),
                result: DiffResult::Identical,
                duration_ms: 10,
                known_divergence: false,
                known_reason: None,
            },
            TestRecord {
                name: "test2".into(),
                category: "cat".into(),
                result: DiffResult::Equivalent("normalized".into()),
                duration_ms: 20,
                known_divergence: false,
                known_reason: None,
            },
        ];
        let stats = ReportStats::from_records(&records);
        assert!(stats.is_success());
    }

    #[test]
    fn test_report_stats_is_success_known_divergence_only() {
        let records = vec![TestRecord {
            name: "test1".into(),
            category: "cat".into(),
            result: DiffResult::Divergent("known issue".into()),
            duration_ms: 10,
            known_divergence: true,
            known_reason: Some("tracked".into()),
        }];
        let stats = ReportStats::from_records(&records);
        assert!(stats.is_success());
    }

    #[test]
    fn test_report_stats_by_category() {
        let records = sample_records();
        let stats = ReportStats::from_records(&records);

        assert!(stats.by_category.contains_key("unit_parsing"));
        assert!(stats.by_category.contains_key("service_lifecycle"));
        assert!(stats.by_category.contains_key("timer"));
        assert!(stats.by_category.contains_key("dbus"));

        let up = stats.by_category.get("unit_parsing").unwrap();
        assert_eq!(up.total, 2);
        assert_eq!(up.pass, 2);
        assert_eq!(up.fail, 0);
        assert_eq!(up.skip, 0);

        let sl = stats.by_category.get("service_lifecycle").unwrap();
        assert_eq!(sl.total, 1);
        assert_eq!(sl.pass, 0);
        assert_eq!(sl.fail, 1);
    }

    #[test]
    fn test_report_stats_display() {
        let records = sample_records();
        let stats = ReportStats::from_records(&records);
        let display = stats.to_string();

        assert!(display.contains("Total:"));
        assert!(display.contains("Identical:"));
        assert!(display.contains("Divergent:"));
        assert!(display.contains("Duration:"));
    }

    #[test]
    fn test_report_stats_empty_records() {
        let stats = ReportStats::from_records(&[]);
        assert_eq!(stats.total, 0);
        assert_eq!(stats.pass_count(), 0);
        assert!(stats.is_success());
        assert!(stats.by_category.is_empty());
    }

    #[test]
    fn test_junit_report_render() {
        let records = sample_records();
        let report = JunitReport::new(&records);
        let bytes = report.render().unwrap();
        let xml = String::from_utf8(bytes).unwrap();

        assert!(xml.starts_with("<?xml version=\"1.0\""));
        assert!(xml.contains("<testsuites name=\"difftest\""));
        assert!(xml.contains("tests=\"5\""));
        assert!(xml.contains("<testsuite name=\"unit_parsing\""));
        assert!(xml.contains("<testcase name=\"ini_parser_basic\""));
        assert!(xml.contains("<failure"));
        assert!(xml.contains("<skipped"));
        assert!(xml.contains("</testsuites>"));
    }

    #[test]
    fn test_junit_report_custom_suite_name() {
        let records = sample_records();
        let report = JunitReport::new(&records).with_suite_name("my-suite");
        let bytes = report.render().unwrap();
        let xml = String::from_utf8(bytes).unwrap();

        assert!(xml.contains("name=\"my-suite\""));
    }

    #[test]
    fn test_junit_report_known_divergence_not_failure() {
        let records = vec![TestRecord {
            name: "known_test".into(),
            category: "cat".into(),
            result: DiffResult::Divergent("known issue".into()),
            duration_ms: 10,
            known_divergence: true,
            known_reason: Some("tracked".into()),
        }];
        let report = JunitReport::new(&records);
        let bytes = report.render().unwrap();
        let xml = String::from_utf8(bytes).unwrap();

        // Known divergences should NOT be <failure> elements — they get system-out
        assert!(!xml.contains("<failure"));
        assert!(xml.contains("<system-out>"));
        assert!(xml.contains("Known divergence"));
    }

    #[test]
    fn test_junit_report_identical_test_self_closing() {
        let records = vec![TestRecord {
            name: "passing_test".into(),
            category: "cat".into(),
            result: DiffResult::Identical,
            duration_ms: 10,
            known_divergence: false,
            known_reason: None,
        }];
        let report = JunitReport::new(&records);
        let bytes = report.render().unwrap();
        let xml = String::from_utf8(bytes).unwrap();

        // Passing tests should be self-closing <testcase ... />
        assert!(xml.contains("name=\"passing_test\""));
        // Should NOT contain a closing </testcase> for this test
        // (it's self-closing with />)
        assert!(xml.contains("/>"));
    }

    #[test]
    fn test_junit_report_empty_records() {
        let records: Vec<TestRecord> = vec![];
        let report = JunitReport::new(&records);
        let bytes = report.render().unwrap();
        let xml = String::from_utf8(bytes).unwrap();

        assert!(xml.contains("tests=\"0\""));
        assert!(xml.contains("failures=\"0\""));
    }

    #[test]
    fn test_junit_report_xml_escaping() {
        let records = vec![TestRecord {
            name: "test<with>&special\"chars'".into(),
            category: "cat&esc".into(),
            result: DiffResult::Divergent("error: a < b && c > d".into()),
            duration_ms: 10,
            known_divergence: false,
            known_reason: None,
        }];
        let report = JunitReport::new(&records);
        let bytes = report.render().unwrap();
        let xml = String::from_utf8(bytes).unwrap();

        // Verify no raw special characters appear unescaped
        assert!(xml.contains("&amp;"));
        assert!(xml.contains("&lt;"));
        assert!(xml.contains("&gt;"));
        assert!(xml.contains("&quot;"));
        assert!(xml.contains("&apos;"));
    }

    #[test]
    fn test_json_report_render() {
        let records = sample_records();
        let report = JsonReport::new(&records);
        let bytes = report.render().unwrap();
        let data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert!(data["generated_at"].is_string());
        assert_eq!(data["suite_name"], "difftest");
        assert_eq!(data["stats"]["total"], 5);
        assert_eq!(data["records"].as_array().unwrap().len(), 5);

        // New divergences: just service_start_stop
        assert_eq!(data["new_divergences"].as_array().unwrap().len(), 1);
        assert_eq!(data["new_divergences"][0]["name"], "service_start_stop");

        // Known divergences: calendar_minutely
        assert_eq!(data["known_divergences"].as_array().unwrap().len(), 1);
        assert_eq!(data["known_divergences"][0]["name"], "calendar_minutely");
    }

    #[test]
    fn test_json_report_custom_suite_name() {
        let records = sample_records();
        let report = JsonReport::new(&records).with_suite_name("custom");
        let bytes = report.render().unwrap();
        let data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(data["suite_name"], "custom");
    }

    #[test]
    fn test_json_report_empty_records() {
        let records: Vec<TestRecord> = vec![];
        let report = JsonReport::new(&records);
        let bytes = report.render().unwrap();
        let data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(data["stats"]["total"], 0);
        assert!(data["records"].as_array().unwrap().is_empty());
        assert!(data["new_divergences"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_json_report_record_fields() {
        let records = vec![TestRecord {
            name: "test1".into(),
            category: "cat".into(),
            result: DiffResult::Equivalent("PIDs normalized".into()),
            duration_ms: 42,
            known_divergence: false,
            known_reason: None,
        }];
        let report = JsonReport::new(&records);
        let bytes = report.render().unwrap();
        let data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        let rec = &data["records"][0];
        assert_eq!(rec["name"], "test1");
        assert_eq!(rec["category"], "cat");
        assert_eq!(rec["result"], "equivalent");
        assert_eq!(rec["result_detail"], "PIDs normalized");
        assert_eq!(rec["duration_ms"], 42);
        assert_eq!(rec["known_divergence"], false);
        assert!(rec["known_reason"].is_null());
    }

    #[test]
    fn test_summary_report_display() {
        let records = sample_records();
        let report = SummaryReport::new(&records);
        let text = report.to_string();

        assert!(text.contains("Differential Test Results"));
        assert!(text.contains("Status: FAIL"));
        assert!(text.contains("New divergences (1):"));
        assert!(text.contains("service_start_stop"));
        assert!(text.contains("Known divergences (1):"));
        assert!(text.contains("calendar_minutely"));
        assert!(text.contains("tracked in #42"));
        assert!(text.contains("Skipped (1):"));
        assert!(text.contains("dbus_query"));
        assert!(text.contains("End of Report"));
    }

    #[test]
    fn test_summary_report_all_pass() {
        let records = vec![
            TestRecord {
                name: "test1".into(),
                category: "cat".into(),
                result: DiffResult::Identical,
                duration_ms: 10,
                known_divergence: false,
                known_reason: None,
            },
            TestRecord {
                name: "test2".into(),
                category: "cat".into(),
                result: DiffResult::Equivalent("x".into()),
                duration_ms: 20,
                known_divergence: false,
                known_reason: None,
            },
        ];
        let report = SummaryReport::new(&records);
        let text = report.to_string();

        assert!(text.contains("Status: PASS"));
        assert!(!text.contains("New divergences ("));
        assert!(!text.contains("Known divergences ("));
        assert!(!text.contains("Skipped ("));
    }

    #[test]
    fn test_summary_report_render() {
        let records = sample_records();
        let report = SummaryReport::new(&records);
        let bytes = report.render().unwrap();
        let text = String::from_utf8(bytes).unwrap();

        // render() should produce the same output as Display
        assert_eq!(text, report.to_string());
    }

    #[test]
    fn test_markdown_report_render() {
        let records = sample_records();
        let report = MarkdownReport::new(&records);
        let bytes = report.render().unwrap();
        let md = String::from_utf8(bytes).unwrap();

        assert!(md.contains("## ❌ Differential Test Results"));
        assert!(md.contains("| Metric | Count |"));
        assert!(md.contains("| Total | 5 |"));
        assert!(md.contains("**New**"));
        assert!(md.contains("**1**"));
        assert!(md.contains("### ❌ New Divergences (1)"));
        assert!(md.contains("<code>service_start_stop</code>"));
        assert!(md.contains("Per-Category Breakdown"));
    }

    #[test]
    fn test_markdown_report_all_pass() {
        let records = vec![TestRecord {
            name: "test1".into(),
            category: "cat".into(),
            result: DiffResult::Identical,
            duration_ms: 10,
            known_divergence: false,
            known_reason: None,
        }];
        let report = MarkdownReport::new(&records);
        let bytes = report.render().unwrap();
        let md = String::from_utf8(bytes).unwrap();

        assert!(md.contains("## ✅ Differential Test Results"));
        assert!(!md.contains("New Divergences"));
    }

    #[test]
    fn test_markdown_report_custom_suite_name() {
        let records = sample_records();
        let report = MarkdownReport::new(&records).with_suite_name("nightly run");
        let bytes = report.render().unwrap();
        let md = String::from_utf8(bytes).unwrap();

        assert!(md.contains("nightly run"));
    }

    #[test]
    fn test_xml_escape() {
        assert_eq!(xml_escape("a&b"), "a&amp;b");
        assert_eq!(xml_escape("a<b"), "a&lt;b");
        assert_eq!(xml_escape("a>b"), "a&gt;b");
        assert_eq!(xml_escape("a\"b"), "a&quot;b");
        assert_eq!(xml_escape("a'b"), "a&apos;b");
        assert_eq!(xml_escape("clean"), "clean");
        assert_eq!(xml_escape(""), "");
    }

    #[test]
    fn test_xml_escape_combined() {
        assert_eq!(xml_escape("a < b && c > d"), "a &lt; b &amp;&amp; c &gt; d");
    }

    #[test]
    fn test_truncate_for_xml_short() {
        assert_eq!(truncate_for_xml("short", 100), "short");
    }

    #[test]
    fn test_truncate_for_xml_exact() {
        assert_eq!(truncate_for_xml("12345", 5), "12345");
    }

    #[test]
    fn test_truncate_for_xml_long() {
        let long = "a".repeat(300);
        let result = truncate_for_xml(&long, 200);
        assert!(result.len() <= 200);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_report_generator_write_to_file() {
        let records = sample_records();
        let report = JsonReport::new(&records);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subdir/report.json");

        report.write_to_file(&path).unwrap();
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        let data: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(data["stats"]["total"], 5);
    }

    #[test]
    fn test_report_generator_write_to() {
        let records = sample_records();
        let report = SummaryReport::new(&records);

        let mut buf = Vec::new();
        report.write_to(&mut buf).unwrap();

        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("Differential Test Results"));
    }

    #[test]
    fn test_category_stats_default() {
        let stats = CategoryStats::default();
        assert_eq!(stats.total, 0);
        assert_eq!(stats.pass, 0);
        assert_eq!(stats.fail, 0);
        assert_eq!(stats.skip, 0);
        assert_eq!(stats.duration_ms, 0);
    }

    #[test]
    fn test_report_stats_total_duration() {
        let records = sample_records();
        let stats = ReportStats::from_records(&records);
        // Sum of 50 + 120 + 500 + 30 + 1 = 701
        assert_eq!(stats.total_duration_ms, 701);
    }

    #[test]
    fn test_junit_report_equivalent_is_passing() {
        let records = vec![TestRecord {
            name: "equiv_test".into(),
            category: "cat".into(),
            result: DiffResult::Equivalent("normalized".into()),
            duration_ms: 10,
            known_divergence: false,
            known_reason: None,
        }];
        let report = JunitReport::new(&records);
        let bytes = report.render().unwrap();
        let xml = String::from_utf8(bytes).unwrap();

        // Equivalent tests should be self-closing (passing)
        assert!(!xml.contains("<failure"));
        assert!(!xml.contains("<skipped"));
    }

    #[test]
    fn test_json_test_record_from_all_result_types() {
        let cases = vec![
            (DiffResult::Identical, "identical", None),
            (
                DiffResult::Equivalent("notes".into()),
                "equivalent",
                Some("notes".to_string()),
            ),
            (
                DiffResult::Divergent("bad".into()),
                "divergent",
                Some("bad".to_string()),
            ),
            (
                DiffResult::Skipped("skip".into()),
                "skipped",
                Some("skip".to_string()),
            ),
        ];

        for (result, expected_result, expected_detail) in cases {
            let rec = TestRecord {
                name: "test".into(),
                category: "cat".into(),
                result,
                duration_ms: 10,
                known_divergence: false,
                known_reason: None,
            };
            let json_rec: JsonTestRecord = (&rec).into();
            assert_eq!(json_rec.result, expected_result);
            assert_eq!(json_rec.result_detail, expected_detail);
        }
    }
}
