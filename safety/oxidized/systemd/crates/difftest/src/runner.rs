//! Parallel test runner for differential tests.
//!
//! The [`DiffTestRunner`] discovers tests registered via the `#[difftest]`
//! proc-macro (through the `inventory` crate), applies filtering, executes
//! them in parallel, and collects results into [`TestRecord`] values suitable
//! for report generation.
//!
//! # Discovery
//!
//! Tests are discovered at link-time via [`inventory::iter`] over
//! [`DiffTestRegistration`] entries. Each registration contains a factory
//! function that produces a `Box<dyn DiffTest>`.
//!
//! # Filtering
//!
//! The runner supports filtering by:
//! - Test name substring or glob
//! - Category name
//! - Tags
//! - Ignored/non-ignored status
//!
//! # Execution
//!
//! Tests are executed in parallel using [`rayon`]'s thread pool. The
//! concurrency level defaults to the number of available CPUs but can be
//! overridden via [`DiffTestRunner::with_concurrency`].
//!
//! # Known divergences
//!
//! The runner loads known divergences from a TOML file (by default
//! `tests/difftest/known-divergences.toml`) and annotates [`TestRecord`]
//! results accordingly, so that reports can distinguish new failures from
//! expected ones.
//!
//! # Usage
//!
//! ```ignore
//! use difftest::runner::DiffTestRunner;
//! use difftest::report::{JunitReport, SummaryReport, ReportGenerator};
//!
//! let runner = DiffTestRunner::new()
//!     .with_category_filter("unit_parsing")
//!     .with_concurrency(4);
//!
//! let records = runner.run_all();
//!
//! // Print summary
//! let summary = SummaryReport::new(&records);
//! println!("{summary}");
//!
//! // Write JUnit XML
//! let junit = JunitReport::new(&records);
//! junit.write_to_file("test-results.xml").unwrap();
//!
//! // Exit with failure if there are new divergences
//! if records.iter().any(|r| r.is_new_failure()) {
//!     std::process::exit(1);
//! }
//! ```

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use rayon::prelude::*;

use crate::{DiffTest, DiffTestRegistration, KnownDivergence, TestRecord, load_known_divergences};

// ── Default paths ───────────────────────────────────────────────────────────

/// Default path for the known-divergences TOML file.
const DEFAULT_KNOWN_DIVERGENCES_PATH: &str = "tests/difftest/known-divergences.toml";

// ── Filter ──────────────────────────────────────────────────────────────────

/// Filtering criteria for selecting which tests to run.
#[derive(Debug, Clone, Default)]
pub struct TestFilter {
    /// If set, only run tests whose name contains this substring.
    pub name_substring: Option<String>,

    /// If set, only run tests in this category.
    pub category: Option<String>,

    /// If set, only run tests that have at least one of these tags.
    pub tags: HashSet<String>,

    /// Whether to include ignored tests (default: `false`).
    pub include_ignored: bool,

    /// If set, **exclude** tests whose name contains any of these substrings.
    pub exclude_patterns: Vec<String>,
}

impl TestFilter {
    /// Returns `true` if the given registration passes the filter.
    pub fn matches(&self, reg: &DiffTestRegistration) -> bool {
        // Ignored check
        if reg.ignored && !self.include_ignored {
            return false;
        }

        // Name substring
        if let Some(ref sub) = self.name_substring
            && !reg.name.contains(sub.as_str())
        {
            return false;
        }

        // Category
        if let Some(ref cat) = self.category
            && reg.category != cat.as_str()
        {
            return false;
        }

        // Tags (if filter specifies tags, the test must have at least one match)
        if !self.tags.is_empty() {
            let test_tags: HashSet<&str> = reg.tags.iter().map(|s| s.as_str()).collect();
            let filter_tags: HashSet<&str> = self.tags.iter().map(|s| s.as_str()).collect();
            if test_tags.is_disjoint(&filter_tags) {
                return false;
            }
        }

        // Exclude patterns
        for pattern in &self.exclude_patterns {
            if reg.name.contains(pattern.as_str()) {
                return false;
            }
        }

        true
    }
}

// ── DiffTestRunner ──────────────────────────────────────────────────────────

/// Discovers, filters, and executes differential tests in parallel.
///
/// See the [module-level documentation](self) for a usage overview.
pub struct DiffTestRunner {
    /// Filter criteria.
    filter: TestFilter,

    /// Maximum number of parallel test threads (0 = use rayon default).
    concurrency: usize,

    /// Path to the known-divergences TOML file.
    known_divergences_path: PathBuf,

    /// Loaded known divergences (populated on first `run_all` call).
    known_divergences: Option<Vec<KnownDivergence>>,
}

impl Default for DiffTestRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl DiffTestRunner {
    /// Create a new runner with default settings.
    pub fn new() -> Self {
        Self {
            filter: TestFilter::default(),
            concurrency: 0,
            known_divergences_path: PathBuf::from(DEFAULT_KNOWN_DIVERGENCES_PATH),
            known_divergences: None,
        }
    }

    /// Filter tests by name substring.
    pub fn with_name_filter(mut self, substring: impl Into<String>) -> Self {
        self.filter.name_substring = Some(substring.into());
        self
    }

    /// Filter tests by category.
    pub fn with_category_filter(mut self, category: impl Into<String>) -> Self {
        self.filter.category = Some(category.into());
        self
    }

    /// Filter tests by tags (tests must have at least one matching tag).
    pub fn with_tag_filter(mut self, tags: impl IntoIterator<Item = String>) -> Self {
        self.filter.tags = tags.into_iter().collect();
        self
    }

    /// Include tests marked `#[difftest(ignore)]`.
    pub fn with_include_ignored(mut self, include: bool) -> Self {
        self.filter.include_ignored = include;
        self
    }

    /// Exclude tests matching any of the given name patterns.
    pub fn with_exclude_patterns(mut self, patterns: Vec<String>) -> Self {
        self.filter.exclude_patterns = patterns;
        self
    }

    /// Set the maximum concurrency (number of parallel threads).
    ///
    /// Use `0` to let rayon decide based on available CPUs.
    pub fn with_concurrency(mut self, n: usize) -> Self {
        self.concurrency = n;
        self
    }

    /// Override the path to the known-divergences file.
    pub fn with_known_divergences_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.known_divergences_path = path.into();
        self
    }

    /// Set the filter directly.
    pub fn with_filter(mut self, filter: TestFilter) -> Self {
        self.filter = filter;
        self
    }

    /// Return a reference to the current filter.
    pub fn filter(&self) -> &TestFilter {
        &self.filter
    }

    /// Load known divergences from the configured path.
    ///
    /// This is called automatically by [`run_all`](Self::run_all), but can be
    /// called manually to inspect the loaded divergences.
    pub fn load_known_divergences(&mut self) -> Vec<KnownDivergence> {
        if self.known_divergences.is_none() {
            let divergences =
                load_known_divergences(&self.known_divergences_path).unwrap_or_default();
            self.known_divergences = Some(divergences);
        }
        self.known_divergences.clone().unwrap_or_default()
    }

    /// Discover all registered tests, apply filters, and execute them in
    /// parallel. Returns a `Vec<TestRecord>` suitable for report generation.
    pub fn run_all(&mut self) -> Vec<TestRecord> {
        let known = self.load_known_divergences();
        let known_names: HashSet<&str> = known.iter().map(|d| d.test.as_str()).collect();

        // Build a lookup for known divergence reasons
        let known_reasons: std::collections::HashMap<&str, &str> = known
            .iter()
            .map(|d| (d.test.as_str(), d.reason.as_str()))
            .collect();

        // Discover tests from inventory
        let registrations: Vec<&DiffTestRegistration> = inventory::iter::<DiffTestRegistration>()
            .filter(|reg| self.filter.matches(reg))
            .collect();

        log::info!(
            "Running {} differential tests (concurrency: {})",
            registrations.len(),
            if self.concurrency == 0 {
                "auto".to_string()
            } else {
                self.concurrency.to_string()
            },
        );

        // Configure rayon thread pool if custom concurrency is requested
        let pool = if self.concurrency > 0 {
            Some(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(self.concurrency)
                    .build()
                    .expect("failed to build rayon thread pool"),
            )
        } else {
            None
        };

        // Execution closure: run a single test and produce a TestRecord
        let run_test = |reg: &&DiffTestRegistration| -> TestRecord {
            let test_impl: Box<dyn DiffTest> = (reg.constructor)();
            let start = Instant::now();

            let left = test_impl.run_systemd();
            let right = test_impl.run_systemd_rs();
            let result = test_impl.compare(&left, &right);

            let duration = start.elapsed();
            let duration_ms = duration.as_millis() as u64;

            let is_known = known_names.contains(reg.name);
            let reason = if is_known {
                known_reasons.get(reg.name).map(|r| (*r).to_string())
            } else {
                None
            };

            TestRecord {
                name: reg.name.to_string(),
                category: reg.category.to_string(),
                result,
                duration_ms,
                known_divergence: is_known,
                known_reason: reason,
            }
        };

        // Execute in parallel
        let records: Vec<TestRecord> = match pool {
            Some(pool) => pool.install(|| registrations.par_iter().map(run_test).collect()),
            None => registrations.par_iter().map(run_test).collect(),
        };

        // Log summary
        let pass = records.iter().filter(|r| r.result.is_pass()).count();
        let fail = records.iter().filter(|r| r.result.is_divergent()).count();
        let skip = records.iter().filter(|r| r.result.is_skipped()).count();
        let new_fail = records.iter().filter(|r| r.is_new_failure()).count();

        log::info!(
            "Results: {} pass, {} fail ({} new), {} skip, {} total",
            pass,
            fail,
            new_fail,
            skip,
            records.len(),
        );

        records
    }

    /// Run a single test by name, returning its [`TestRecord`].
    ///
    /// Returns `None` if no test with the given name is registered.
    pub fn run_one(&mut self, name: &str) -> Option<TestRecord> {
        let known = self.load_known_divergences();
        let known_reasons: std::collections::HashMap<&str, &str> = known
            .iter()
            .map(|d| (d.test.as_str(), d.reason.as_str()))
            .collect();
        let known_names: HashSet<&str> = known.iter().map(|d| d.test.as_str()).collect();

        let reg = inventory::iter::<DiffTestRegistration>().find(|r| r.name == name)?;

        let test_impl: Box<dyn DiffTest> = (reg.constructor)();
        let start = Instant::now();

        let left = test_impl.run_systemd();
        let right = test_impl.run_systemd_rs();
        let result = test_impl.compare(&left, &right);

        let duration = start.elapsed();
        let duration_ms = duration.as_millis() as u64;

        let is_known = known_names.contains(name);
        let reason = if is_known {
            known_reasons.get(name).map(|r| (*r).to_string())
        } else {
            None
        };

        Some(TestRecord {
            name: reg.name.to_string(),
            category: reg.category.to_string(),
            result,
            duration_ms,
            known_divergence: is_known,
            known_reason: reason,
        })
    }

    /// List all registered tests that pass the current filter, without
    /// executing them.
    ///
    /// Returns a list of `(name, category, ignored)` tuples.
    pub fn list_tests(&self) -> Vec<(&'static str, &'static str, bool)> {
        inventory::iter::<DiffTestRegistration>()
            .filter(|reg| self.filter.matches(reg))
            .map(|reg| (reg.name, reg.category, reg.ignored))
            .collect()
    }

    /// Count the number of registered tests that pass the current filter.
    pub fn test_count(&self) -> usize {
        inventory::iter::<DiffTestRegistration>()
            .filter(|reg| self.filter.matches(reg))
            .count()
    }
}

// ── Standalone execution helpers ────────────────────────────────────────────

/// Run all registered differential tests with default settings and return the
/// records.
///
/// This is a convenience wrapper around [`DiffTestRunner`] for simple use cases.
pub fn run_all_difftests() -> Vec<TestRecord> {
    let mut runner = DiffTestRunner::new();
    runner.run_all()
}

/// Run all registered differential tests, print a summary to stdout, and
/// exit with code 1 if there are new divergences.
///
/// Intended for use as a test binary's `main()`:
///
/// ```ignore
/// fn main() {
///     difftest::runner::run_and_exit();
/// }
/// ```
pub fn run_and_exit() {
    let mut runner = DiffTestRunner::new();

    // Parse basic environment-variable configuration
    if let Ok(category) = std::env::var("DIFFTEST_CATEGORY") {
        runner = runner.with_category_filter(category);
    }
    if let Ok(filter) = std::env::var("DIFFTEST_FILTER") {
        runner = runner.with_name_filter(filter);
    }
    if let Ok(conc) = std::env::var("DIFFTEST_CONCURRENCY")
        && let Ok(n) = conc.parse::<usize>()
    {
        runner = runner.with_concurrency(n);
    }
    if std::env::var("DIFFTEST_INCLUDE_IGNORED")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        runner = runner.with_include_ignored(true);
    }

    let records = runner.run_all();

    // Print summary
    let summary = crate::report::SummaryReport::new(&records);
    println!("{summary}");

    // Write reports if paths are configured
    if let Ok(junit_path) = std::env::var("DIFFTEST_JUNIT_PATH") {
        let report = crate::report::JunitReport::new(&records);
        if let Err(e) = crate::report::ReportGenerator::write_to_file(&report, &junit_path) {
            eprintln!("Warning: failed to write JUnit report to {junit_path}: {e}");
        } else {
            println!("JUnit report written to {junit_path}");
        }
    }
    if let Ok(json_path) = std::env::var("DIFFTEST_JSON_PATH") {
        let report = crate::report::JsonReport::new(&records);
        if let Err(e) = crate::report::ReportGenerator::write_to_file(&report, &json_path) {
            eprintln!("Warning: failed to write JSON report to {json_path}: {e}");
        } else {
            println!("JSON report written to {json_path}");
        }
    }
    if let Ok(md_path) = std::env::var("DIFFTEST_MARKDOWN_PATH") {
        let report = crate::report::MarkdownReport::new(&records);
        if let Err(e) = crate::report::ReportGenerator::write_to_file(&report, &md_path) {
            eprintln!("Warning: failed to write Markdown report to {md_path}: {e}");
        } else {
            println!("Markdown report written to {md_path}");
        }
    }

    // Exit with failure if there are new divergences
    let new_failures = records.iter().filter(|r| r.is_new_failure()).count();
    if new_failures > 0 {
        eprintln!("{new_failures} new divergence(s) detected — failing.");
        std::process::exit(1);
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // We can't easily test inventory-based discovery in unit tests because
    // no `#[difftest]` functions are registered in this crate. Instead, we
    // test the filter logic and runner configuration directly.

    /// Helper to create a mock DiffTestRegistration for filter testing.
    /// Because DiffTestRegistration has a fn pointer, we create the struct
    /// directly.
    fn mock_registration(
        name: &'static str,
        category: &'static str,
        tags: &'static [String],
        ignored: bool,
    ) -> DiffTestRegistration {
        DiffTestRegistration {
            name,
            category,
            timeout_ms: 30_000,
            tags,
            ignored,
            constructor: || {
                panic!("mock constructor should not be called in filter tests");
            },
        }
    }

    #[test]
    fn test_filter_default_matches_non_ignored() {
        let filter = TestFilter::default();
        let reg = mock_registration("test1", "cat", &[], false);
        assert!(filter.matches(&reg));
    }

    #[test]
    fn test_filter_default_rejects_ignored() {
        let filter = TestFilter::default();
        let reg = mock_registration("test1", "cat", &[], true);
        assert!(!filter.matches(&reg));
    }

    #[test]
    fn test_filter_include_ignored() {
        let filter = TestFilter {
            include_ignored: true,
            ..Default::default()
        };
        let reg = mock_registration("test1", "cat", &[], true);
        assert!(filter.matches(&reg));
    }

    #[test]
    fn test_filter_name_substring_match() {
        let filter = TestFilter {
            name_substring: Some("parser".into()),
            ..Default::default()
        };
        let reg = mock_registration("ini_parser_basic", "cat", &[], false);
        assert!(filter.matches(&reg));
    }

    #[test]
    fn test_filter_name_substring_no_match() {
        let filter = TestFilter {
            name_substring: Some("dbus".into()),
            ..Default::default()
        };
        let reg = mock_registration("ini_parser_basic", "cat", &[], false);
        assert!(!filter.matches(&reg));
    }

    #[test]
    fn test_filter_category_match() {
        let filter = TestFilter {
            category: Some("unit_parsing".into()),
            ..Default::default()
        };
        let reg_match = mock_registration("test1", "unit_parsing", &[], false);
        let reg_no = mock_registration("test2", "dbus", &[], false);
        assert!(filter.matches(&reg_match));
        assert!(!filter.matches(&reg_no));
    }

    #[test]
    fn test_filter_exclude_patterns() {
        let filter = TestFilter {
            exclude_patterns: vec!["flaky".into(), "slow".into()],
            ..Default::default()
        };
        let reg1 = mock_registration("fast_test", "cat", &[], false);
        let reg2 = mock_registration("flaky_test", "cat", &[], false);
        let reg3 = mock_registration("slow_test", "cat", &[], false);
        assert!(filter.matches(&reg1));
        assert!(!filter.matches(&reg2));
        assert!(!filter.matches(&reg3));
    }

    #[test]
    fn test_filter_combined() {
        let filter = TestFilter {
            name_substring: Some("parser".into()),
            category: Some("unit_parsing".into()),
            ..Default::default()
        };
        // Matches both criteria
        let reg1 = mock_registration("ini_parser_basic", "unit_parsing", &[], false);
        assert!(filter.matches(&reg1));

        // Name matches, category doesn't
        let reg2 = mock_registration("ini_parser_basic", "dbus", &[], false);
        assert!(!filter.matches(&reg2));

        // Category matches, name doesn't
        let reg3 = mock_registration("specifier_test", "unit_parsing", &[], false);
        assert!(!filter.matches(&reg3));
    }

    #[test]
    fn test_runner_builder_chain() {
        let runner = DiffTestRunner::new()
            .with_name_filter("parser")
            .with_category_filter("unit_parsing")
            .with_concurrency(4)
            .with_include_ignored(true)
            .with_exclude_patterns(vec!["flaky".into()]);

        assert_eq!(runner.filter.name_substring.as_deref(), Some("parser"));
        assert_eq!(runner.filter.category.as_deref(), Some("unit_parsing"));
        assert_eq!(runner.concurrency, 4);
        assert!(runner.filter.include_ignored);
        assert_eq!(runner.filter.exclude_patterns, vec!["flaky".to_string()]);
    }

    #[test]
    fn test_runner_default() {
        let runner = DiffTestRunner::new();
        assert!(runner.filter.name_substring.is_none());
        assert!(runner.filter.category.is_none());
        assert!(runner.filter.tags.is_empty());
        assert!(!runner.filter.include_ignored);
        assert!(runner.filter.exclude_patterns.is_empty());
        assert_eq!(runner.concurrency, 0);
    }

    #[test]
    fn test_runner_known_divergences_path() {
        let runner = DiffTestRunner::new().with_known_divergences_path("/custom/path.toml");
        assert_eq!(
            runner.known_divergences_path,
            PathBuf::from("/custom/path.toml")
        );
    }

    #[test]
    fn test_runner_load_known_divergences_missing_file() {
        let mut runner =
            DiffTestRunner::new().with_known_divergences_path("/nonexistent/path.toml");
        let divergences = runner.load_known_divergences();
        assert!(divergences.is_empty());
    }

    #[test]
    fn test_runner_load_known_divergences_from_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known-divergences.toml");
        std::fs::write(
            &path,
            r#"
[[divergence]]
test = "test_foo"
reason = "known issue #123"

[[divergence]]
test = "test_bar"
reason = "fixed in v257"
systemd_version = ">=257"
"#,
        )
        .unwrap();

        let mut runner = DiffTestRunner::new().with_known_divergences_path(&path);
        let divergences = runner.load_known_divergences();
        assert_eq!(divergences.len(), 2);
        assert_eq!(divergences[0].test, "test_foo");
        assert_eq!(divergences[0].reason, "known issue #123");
        assert_eq!(divergences[1].test, "test_bar");
        assert_eq!(divergences[1].systemd_version.as_deref(), Some(">=257"));
    }

    #[test]
    fn test_runner_run_all_empty() {
        // No tests are registered in this crate, so run_all should return empty
        let mut runner = DiffTestRunner::new();
        let records = runner.run_all();
        assert!(records.is_empty());
    }

    #[test]
    fn test_runner_list_tests_empty() {
        let runner = DiffTestRunner::new();
        let tests = runner.list_tests();
        assert!(tests.is_empty());
    }

    #[test]
    fn test_runner_test_count_empty() {
        let runner = DiffTestRunner::new();
        assert_eq!(runner.test_count(), 0);
    }

    #[test]
    fn test_runner_run_one_not_found() {
        let mut runner = DiffTestRunner::new();
        assert!(runner.run_one("nonexistent_test").is_none());
    }

    #[test]
    fn test_runner_with_filter() {
        let filter = TestFilter {
            name_substring: Some("special".into()),
            category: Some("my_cat".into()),
            include_ignored: true,
            ..Default::default()
        };
        let runner = DiffTestRunner::new().with_filter(filter);
        assert_eq!(runner.filter().name_substring.as_deref(), Some("special"));
        assert_eq!(runner.filter().category.as_deref(), Some("my_cat"));
        assert!(runner.filter().include_ignored);
    }

    #[test]
    fn test_runner_tag_filter() {
        let runner =
            DiffTestRunner::new().with_tag_filter(vec!["slow".to_string(), "vm".to_string()]);
        assert!(runner.filter.tags.contains("slow"));
        assert!(runner.filter.tags.contains("vm"));
        assert_eq!(runner.filter.tags.len(), 2);
    }
}
