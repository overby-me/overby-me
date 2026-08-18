//! Snapshot-based comparison for differential test outputs.
//!
//! Snapshots allow differential tests to record a "golden" reference output and
//! compare future runs against it. This is especially useful for:
//!
//! - Approving intentional divergences between systemd and rust-systemd
//! - Tracking expected output for tests where real systemd is not available
//! - Regression detection when rust-systemd behavior changes
//!
//! # Snapshot storage
//!
//! Snapshots are stored as JSON files under `tests/difftest/snapshots/`, organized
//! by category and test name:
//!
//! ```text
//! tests/difftest/snapshots/
//!   unit_parsing/
//!     ini_parser_basic.snap.json
//!     specifier_expansion.snap.json
//!   service_lifecycle/
//!     simple_start_stop.snap.json
//! ```
//!
//! # Update mode
//!
//! When the `DIFFTEST_UPDATE_SNAPSHOTS` environment variable is set (to any
//! non-empty value), snapshot comparisons will write the current output as the
//! new golden reference instead of comparing against the existing snapshot.
//! This is analogous to `cargo insta review` or `UPDATE_EXPECT=1` in
//! `expect-test`.
//!
//! # Snapshot format
//!
//! Each `.snap.json` file contains a [`SnapshotFile`] struct serialized as JSON:
//!
//! ```json
//! {
//!   "version": 1,
//!   "test_name": "ini_parser_basic",
//!   "category": "unit_parsing",
//!   "created_at": "2024-06-01T12:00:00Z",
//!   "updated_at": "2024-06-15T09:30:00Z",
//!   "systemd_output": { "RawText": "..." },
//!   "systemd_rs_output": { "RawText": "..." },
//!   "result": "Identical",
//!   "notes": null
//! }
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{DiffResult, DiffTestError, TestOutput};

// ── Constants ───────────────────────────────────────────────────────────────

/// Current snapshot format version.
const SNAPSHOT_VERSION: u32 = 1;

/// Environment variable that triggers snapshot update mode.
const UPDATE_SNAPSHOTS_ENV: &str = "DIFFTEST_UPDATE_SNAPSHOTS";

/// Default base directory for snapshots (relative to workspace root).
const DEFAULT_SNAPSHOT_DIR: &str = "tests/difftest/snapshots";

// ── SnapshotFile ────────────────────────────────────────────────────────────

/// On-disk representation of a snapshot file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotFile {
    /// Format version for forward compatibility.
    pub version: u32,

    /// The test name that produced this snapshot.
    pub test_name: String,

    /// The test category.
    pub category: String,

    /// ISO-8601 timestamp of when this snapshot was first created.
    pub created_at: String,

    /// ISO-8601 timestamp of the most recent update.
    pub updated_at: String,

    /// Output captured from real systemd.
    pub systemd_output: TestOutput,

    /// Output captured from rust-systemd.
    pub systemd_rs_output: TestOutput,

    /// The comparison result at the time the snapshot was recorded.
    pub result: DiffResult,

    /// Optional human-readable notes (e.g. why a divergence was approved).
    pub notes: Option<String>,

    /// Metadata key-value pairs for extensibility.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

impl SnapshotFile {
    /// Create a new snapshot from the current test outputs.
    pub fn new(
        test_name: impl Into<String>,
        category: impl Into<String>,
        systemd_output: TestOutput,
        systemd_rs_output: TestOutput,
        result: DiffResult,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            version: SNAPSHOT_VERSION,
            test_name: test_name.into(),
            category: category.into(),
            created_at: now.clone(),
            updated_at: now,
            systemd_output,
            systemd_rs_output,
            result,
            notes: None,
            metadata: BTreeMap::new(),
        }
    }

    /// Update the snapshot with new outputs, preserving `created_at` and `notes`.
    pub fn update(
        &mut self,
        systemd_output: TestOutput,
        systemd_rs_output: TestOutput,
        result: DiffResult,
    ) {
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self.systemd_output = systemd_output;
        self.systemd_rs_output = systemd_rs_output;
        self.result = result;
        // Preserve version, created_at, notes, and metadata
    }

    /// Add a note to the snapshot (e.g. explaining an approved divergence).
    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }

    /// Add a metadata key-value pair.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

// ── SnapshotStore ───────────────────────────────────────────────────────────

/// Manages reading and writing snapshot files on disk.
///
/// Snapshots are organized as:
/// `{base_dir}/{category}/{test_name}.snap.json`
pub struct SnapshotStore {
    /// Root directory for snapshot storage.
    base_dir: PathBuf,
}

impl SnapshotStore {
    /// Create a store rooted at the given directory.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Create a store using the default snapshot directory.
    ///
    /// Resolves relative to the current working directory (expected to be the
    /// workspace root).
    pub fn default_location() -> Self {
        Self::new(DEFAULT_SNAPSHOT_DIR)
    }

    /// Return the base directory path.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Compute the file path for a given test's snapshot.
    pub fn snapshot_path(&self, category: &str, test_name: &str) -> PathBuf {
        self.base_dir
            .join(sanitize_filename(category))
            .join(format!("{}.snap.json", sanitize_filename(test_name)))
    }

    /// Load a snapshot from disk, returning `None` if it doesn't exist.
    pub fn load(
        &self,
        category: &str,
        test_name: &str,
    ) -> Result<Option<SnapshotFile>, DiffTestError> {
        let path = self.snapshot_path(category, test_name);
        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&path)
            .map_err(|e| DiffTestError::Io(format!("reading snapshot {}: {e}", path.display())))?;

        let snapshot: SnapshotFile = serde_json::from_str(&content).map_err(|e| {
            DiffTestError::Serialization(format!("parsing snapshot {}: {e}", path.display()))
        })?;

        // Version check for forward compatibility
        if snapshot.version > SNAPSHOT_VERSION {
            return Err(DiffTestError::Config(format!(
                "snapshot {} has version {}, but we only support up to version {}",
                path.display(),
                snapshot.version,
                SNAPSHOT_VERSION
            )));
        }

        Ok(Some(snapshot))
    }

    /// Save a snapshot to disk, creating directories as needed.
    pub fn save(&self, snapshot: &SnapshotFile) -> Result<PathBuf, DiffTestError> {
        let path = self.snapshot_path(&snapshot.category, &snapshot.test_name);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DiffTestError::Io(format!(
                    "creating snapshot directory {}: {e}",
                    parent.display()
                ))
            })?;
        }

        let json = serde_json::to_string_pretty(snapshot)
            .map_err(|e| DiffTestError::Serialization(format!("serializing snapshot: {e}")))?;

        std::fs::write(&path, json.as_bytes())
            .map_err(|e| DiffTestError::Io(format!("writing snapshot {}: {e}", path.display())))?;

        Ok(path)
    }

    /// Delete a snapshot from disk.
    ///
    /// Returns `Ok(true)` if the file was removed, `Ok(false)` if it didn't
    /// exist, or an error on I/O failure.
    pub fn delete(&self, category: &str, test_name: &str) -> Result<bool, DiffTestError> {
        let path = self.snapshot_path(category, test_name);
        if !path.exists() {
            return Ok(false);
        }
        std::fs::remove_file(&path)
            .map_err(|e| DiffTestError::Io(format!("deleting snapshot {}: {e}", path.display())))?;
        Ok(true)
    }

    /// List all snapshot files in the store, grouped by category.
    ///
    /// Returns a map of `category → [test_name, ...]`.
    pub fn list_all(&self) -> Result<BTreeMap<String, Vec<String>>, DiffTestError> {
        let mut result = BTreeMap::new();

        if !self.base_dir.exists() {
            return Ok(result);
        }

        let entries = std::fs::read_dir(&self.base_dir).map_err(|e| {
            DiffTestError::Io(format!(
                "listing snapshot directory {}: {e}",
                self.base_dir.display()
            ))
        })?;

        for entry in entries {
            let entry = entry
                .map_err(|e| DiffTestError::Io(format!("reading snapshot directory entry: {e}")))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let category = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();

            let mut tests = Vec::new();
            let cat_entries = std::fs::read_dir(&path).map_err(|e| {
                DiffTestError::Io(format!(
                    "listing category directory {}: {e}",
                    path.display()
                ))
            })?;

            for cat_entry in cat_entries {
                let cat_entry = cat_entry
                    .map_err(|e| DiffTestError::Io(format!("reading category entry: {e}")))?;
                let file_name = cat_entry.file_name().to_string_lossy().into_owned();
                if file_name.ends_with(".snap.json") {
                    let test_name = file_name
                        .strip_suffix(".snap.json")
                        .unwrap_or(&file_name)
                        .to_string();
                    tests.push(test_name);
                }
            }

            tests.sort();
            if !tests.is_empty() {
                result.insert(category, tests);
            }
        }

        Ok(result)
    }
}

// ── Snapshot comparison ─────────────────────────────────────────────────────

/// Outcome of comparing current outputs against a stored snapshot.
#[derive(Debug, Clone)]
pub enum SnapshotComparison {
    /// No snapshot exists yet; this is a new test.
    New,

    /// Current outputs match the snapshot exactly.
    Match,

    /// Current outputs differ from the snapshot.
    Changed {
        /// The stored snapshot.
        previous: Box<SnapshotFile>,
        /// Description of what changed.
        changes: String,
    },

    /// The snapshot was updated (in update mode).
    Updated {
        /// The path where the snapshot was written.
        path: PathBuf,
    },
}

/// Compare current test outputs against the stored snapshot.
///
/// # Behavior
///
/// - If `DIFFTEST_UPDATE_SNAPSHOTS` is set, the current output is written as
///   the new snapshot and [`SnapshotComparison::Updated`] is returned.
/// - If no snapshot exists, returns [`SnapshotComparison::New`].
/// - If the snapshot exists and matches, returns [`SnapshotComparison::Match`].
/// - If the snapshot exists and differs, returns [`SnapshotComparison::Changed`].
pub fn compare_with_snapshot(
    store: &SnapshotStore,
    test_name: &str,
    category: &str,
    systemd_output: &TestOutput,
    systemd_rs_output: &TestOutput,
    result: &DiffResult,
) -> Result<SnapshotComparison, DiffTestError> {
    let update_mode = is_update_mode();

    if update_mode {
        return update_snapshot(
            store,
            test_name,
            category,
            systemd_output,
            systemd_rs_output,
            result,
        );
    }

    let existing = store.load(category, test_name)?;
    match existing {
        None => Ok(SnapshotComparison::New),
        Some(snap) => {
            let outputs_match = snap.systemd_output == *systemd_output
                && snap.systemd_rs_output == *systemd_rs_output
                && snap.result == *result;

            if outputs_match {
                Ok(SnapshotComparison::Match)
            } else {
                let changes =
                    describe_snapshot_changes(&snap, systemd_output, systemd_rs_output, result);
                Ok(SnapshotComparison::Changed {
                    previous: Box::new(snap),
                    changes,
                })
            }
        }
    }
}

/// Update (or create) a snapshot with the current outputs.
fn update_snapshot(
    store: &SnapshotStore,
    test_name: &str,
    category: &str,
    systemd_output: &TestOutput,
    systemd_rs_output: &TestOutput,
    result: &DiffResult,
) -> Result<SnapshotComparison, DiffTestError> {
    let existing = store.load(category, test_name)?;

    let snapshot = match existing {
        Some(mut snap) => {
            snap.update(
                systemd_output.clone(),
                systemd_rs_output.clone(),
                result.clone(),
            );
            snap
        }
        None => SnapshotFile::new(
            test_name,
            category,
            systemd_output.clone(),
            systemd_rs_output.clone(),
            result.clone(),
        ),
    };

    let path = store.save(&snapshot)?;
    Ok(SnapshotComparison::Updated { path })
}

/// Build a human-readable description of what changed between the stored
/// snapshot and the current outputs.
fn describe_snapshot_changes(
    snap: &SnapshotFile,
    systemd_output: &TestOutput,
    systemd_rs_output: &TestOutput,
    result: &DiffResult,
) -> String {
    let mut parts = Vec::new();

    if snap.systemd_output != *systemd_output {
        parts.push("systemd output changed".to_string());
    }
    if snap.systemd_rs_output != *systemd_rs_output {
        parts.push("rust-systemd output changed".to_string());
    }
    if snap.result != *result {
        parts.push(format!("result changed from {} to {}", snap.result, result));
    }

    if parts.is_empty() {
        "unknown change".to_string()
    } else {
        parts.join("; ")
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Returns `true` if snapshot update mode is active.
pub fn is_update_mode() -> bool {
    std::env::var(UPDATE_SNAPSHOTS_ENV)
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// Sanitize a string for use as a filename component.
///
/// Replaces characters that are problematic in file paths with underscores.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c if c.is_ascii_control() => '_',
            c => c,
        })
        .collect()
}

/// Summary statistics for a collection of snapshot comparisons.
#[derive(Debug, Clone, Default)]
pub struct SnapshotSummary {
    /// Number of tests that matched their snapshots.
    pub matched: usize,
    /// Number of tests with changed outputs.
    pub changed: usize,
    /// Number of new tests without existing snapshots.
    pub new: usize,
    /// Number of snapshots that were updated.
    pub updated: usize,
}

impl SnapshotSummary {
    /// Record a comparison result in the summary.
    pub fn record(&mut self, comparison: &SnapshotComparison) {
        match comparison {
            SnapshotComparison::Match => self.matched += 1,
            SnapshotComparison::Changed { .. } => self.changed += 1,
            SnapshotComparison::New => self.new += 1,
            SnapshotComparison::Updated { .. } => self.updated += 1,
        }
    }

    /// Total number of comparisons recorded.
    pub fn total(&self) -> usize {
        self.matched + self.changed + self.new + self.updated
    }

    /// Returns `true` if there are no changes or new tests requiring attention.
    pub fn is_clean(&self) -> bool {
        self.changed == 0 && self.new == 0
    }
}

impl std::fmt::Display for SnapshotSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Snapshots: {} total, {} matched, {} changed, {} new, {} updated",
            self.total(),
            self.matched,
            self.changed,
            self.new,
            self.updated
        )
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, SnapshotStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(dir.path().to_path_buf());
        (dir, store)
    }

    #[test]
    fn test_snapshot_file_creation() {
        let snap = SnapshotFile::new(
            "test_basic",
            "unit_parsing",
            TestOutput::RawText("hello".into()),
            TestOutput::RawText("hello".into()),
            DiffResult::Identical,
        );
        assert_eq!(snap.version, SNAPSHOT_VERSION);
        assert_eq!(snap.test_name, "test_basic");
        assert_eq!(snap.category, "unit_parsing");
        assert!(snap.notes.is_none());
        assert!(snap.metadata.is_empty());
    }

    #[test]
    fn test_snapshot_file_with_notes() {
        let snap = SnapshotFile::new(
            "test1",
            "cat",
            TestOutput::ExitCode(0),
            TestOutput::ExitCode(0),
            DiffResult::Identical,
        )
        .with_notes("approved divergence for PID ordering");

        assert_eq!(
            snap.notes.as_deref(),
            Some("approved divergence for PID ordering")
        );
    }

    #[test]
    fn test_snapshot_file_with_metadata() {
        let snap = SnapshotFile::new(
            "test1",
            "cat",
            TestOutput::ExitCode(0),
            TestOutput::ExitCode(0),
            DiffResult::Identical,
        )
        .with_metadata("systemd_version", "256")
        .with_metadata("distro", "nixos");

        assert_eq!(snap.metadata.get("systemd_version").unwrap(), "256");
        assert_eq!(snap.metadata.get("distro").unwrap(), "nixos");
    }

    #[test]
    fn test_snapshot_file_update() {
        let mut snap = SnapshotFile::new(
            "test1",
            "cat",
            TestOutput::RawText("old".into()),
            TestOutput::RawText("old".into()),
            DiffResult::Identical,
        )
        .with_notes("keep this note");

        let original_created = snap.created_at.clone();

        // Simulate a small delay by just checking invariants
        snap.update(
            TestOutput::RawText("new".into()),
            TestOutput::RawText("new".into()),
            DiffResult::Equivalent("normalized".into()),
        );

        // created_at should be preserved
        assert_eq!(snap.created_at, original_created);
        // notes should be preserved
        assert_eq!(snap.notes.as_deref(), Some("keep this note"));
        // outputs should be updated
        assert_eq!(snap.systemd_output, TestOutput::RawText("new".into()));
        assert_eq!(snap.result, DiffResult::Equivalent("normalized".into()));
    }

    #[test]
    fn test_snapshot_store_path_generation() {
        let store = SnapshotStore::new("/tmp/snapshots");
        let path = store.snapshot_path("unit_parsing", "basic_test");
        assert_eq!(
            path,
            PathBuf::from("/tmp/snapshots/unit_parsing/basic_test.snap.json")
        );
    }

    #[test]
    fn test_snapshot_store_path_sanitization() {
        let store = SnapshotStore::new("/tmp/snapshots");
        let path = store.snapshot_path("my/category", "test:name");
        assert_eq!(
            path,
            PathBuf::from("/tmp/snapshots/my_category/test_name.snap.json")
        );
    }

    #[test]
    fn test_snapshot_store_save_and_load() {
        let (_dir, store) = temp_store();

        let snap = SnapshotFile::new(
            "my_test",
            "my_category",
            TestOutput::RawText("systemd output".into()),
            TestOutput::RawText("rust-systemd output".into()),
            DiffResult::Divergent("they differ".into()),
        )
        .with_notes("expected divergence");

        // Save
        let path = store.save(&snap).unwrap();
        assert!(path.exists());

        // Load
        let loaded = store.load("my_category", "my_test").unwrap().unwrap();
        assert_eq!(loaded.test_name, "my_test");
        assert_eq!(loaded.category, "my_category");
        assert_eq!(
            loaded.systemd_output,
            TestOutput::RawText("systemd output".into())
        );
        assert_eq!(
            loaded.systemd_rs_output,
            TestOutput::RawText("rust-systemd output".into())
        );
        assert_eq!(loaded.result, DiffResult::Divergent("they differ".into()));
        assert_eq!(loaded.notes.as_deref(), Some("expected divergence"));
    }

    #[test]
    fn test_snapshot_store_load_nonexistent() {
        let (_dir, store) = temp_store();
        let result = store.load("nonexistent", "nope").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_snapshot_store_delete() {
        let (_dir, store) = temp_store();

        let snap = SnapshotFile::new(
            "deleteme",
            "cat",
            TestOutput::ExitCode(0),
            TestOutput::ExitCode(0),
            DiffResult::Identical,
        );
        store.save(&snap).unwrap();

        // Should exist
        assert!(store.load("cat", "deleteme").unwrap().is_some());

        // Delete
        assert!(store.delete("cat", "deleteme").unwrap());

        // Should be gone
        assert!(store.load("cat", "deleteme").unwrap().is_none());

        // Delete again should return false
        assert!(!store.delete("cat", "deleteme").unwrap());
    }

    #[test]
    fn test_snapshot_store_list_all() {
        let (_dir, store) = temp_store();

        // Save a few snapshots across categories
        for (cat, name) in &[("alpha", "test1"), ("alpha", "test2"), ("beta", "test3")] {
            let snap = SnapshotFile::new(
                *name,
                *cat,
                TestOutput::ExitCode(0),
                TestOutput::ExitCode(0),
                DiffResult::Identical,
            );
            store.save(&snap).unwrap();
        }

        let listing = store.list_all().unwrap();
        assert_eq!(listing.len(), 2);
        assert_eq!(
            listing.get("alpha").unwrap(),
            &vec!["test1".to_string(), "test2".to_string()]
        );
        assert_eq!(listing.get("beta").unwrap(), &vec!["test3".to_string()]);
    }

    #[test]
    fn test_snapshot_store_list_all_empty() {
        let (_dir, store) = temp_store();
        let listing = store.list_all().unwrap();
        assert!(listing.is_empty());
    }

    #[test]
    fn test_snapshot_store_list_all_nonexistent_dir() {
        let store = SnapshotStore::new("/nonexistent/path/snapshots");
        let listing = store.list_all().unwrap();
        assert!(listing.is_empty());
    }

    #[test]
    fn test_compare_with_snapshot_new() {
        let (_dir, store) = temp_store();

        let result = compare_with_snapshot(
            &store,
            "new_test",
            "cat",
            &TestOutput::ExitCode(0),
            &TestOutput::ExitCode(0),
            &DiffResult::Identical,
        )
        .unwrap();

        assert!(matches!(result, SnapshotComparison::New));
    }

    #[test]
    fn test_compare_with_snapshot_match() {
        let (_dir, store) = temp_store();

        let systemd = TestOutput::RawText("hello".into());
        let systemd_rs = TestOutput::RawText("hello".into());
        let diff = DiffResult::Identical;

        // Store a snapshot
        let snap = SnapshotFile::new(
            "match_test",
            "cat",
            systemd.clone(),
            systemd_rs.clone(),
            diff.clone(),
        );
        store.save(&snap).unwrap();

        // Compare against it
        let result =
            compare_with_snapshot(&store, "match_test", "cat", &systemd, &systemd_rs, &diff)
                .unwrap();

        assert!(matches!(result, SnapshotComparison::Match));
    }

    #[test]
    fn test_compare_with_snapshot_changed() {
        let (_dir, store) = temp_store();

        // Store with old output
        let snap = SnapshotFile::new(
            "change_test",
            "cat",
            TestOutput::RawText("old".into()),
            TestOutput::RawText("old".into()),
            DiffResult::Identical,
        );
        store.save(&snap).unwrap();

        // Compare with new output
        let result = compare_with_snapshot(
            &store,
            "change_test",
            "cat",
            &TestOutput::RawText("new".into()),
            &TestOutput::RawText("new".into()),
            &DiffResult::Identical,
        )
        .unwrap();

        match result {
            SnapshotComparison::Changed { changes, .. } => {
                assert!(changes.contains("systemd output changed"));
                assert!(changes.contains("rust-systemd output changed"));
            }
            other => panic!("expected Changed, got: {other:?}"),
        }
    }

    #[test]
    fn test_compare_with_snapshot_result_changed() {
        let (_dir, store) = temp_store();

        let output = TestOutput::RawText("same".into());

        // Store with Identical result
        let snap = SnapshotFile::new(
            "result_test",
            "cat",
            output.clone(),
            output.clone(),
            DiffResult::Identical,
        );
        store.save(&snap).unwrap();

        // Compare with Divergent result but same outputs
        let result = compare_with_snapshot(
            &store,
            "result_test",
            "cat",
            &output,
            &output,
            &DiffResult::Divergent("something changed".into()),
        )
        .unwrap();

        match result {
            SnapshotComparison::Changed { changes, .. } => {
                assert!(changes.contains("result changed"));
            }
            other => panic!("expected Changed, got: {other:?}"),
        }
    }

    #[test]
    fn test_sanitize_filename_basic() {
        assert_eq!(sanitize_filename("hello_world"), "hello_world");
        assert_eq!(sanitize_filename("hello world"), "hello world");
    }

    #[test]
    fn test_sanitize_filename_special_chars() {
        assert_eq!(sanitize_filename("a/b\\c:d"), "a_b_c_d");
        assert_eq!(sanitize_filename("file*name?.txt"), "file_name_.txt");
        assert_eq!(sanitize_filename("a<b>c|d"), "a_b_c_d");
    }

    #[test]
    fn test_sanitize_filename_empty() {
        assert_eq!(sanitize_filename(""), "");
    }

    #[test]
    fn test_snapshot_summary_default() {
        let summary = SnapshotSummary::default();
        assert_eq!(summary.total(), 0);
        assert!(summary.is_clean());
    }

    #[test]
    fn test_snapshot_summary_record() {
        let mut summary = SnapshotSummary::default();

        summary.record(&SnapshotComparison::Match);
        summary.record(&SnapshotComparison::Match);
        summary.record(&SnapshotComparison::New);
        summary.record(&SnapshotComparison::Changed {
            previous: Box::new(SnapshotFile::new(
                "x",
                "y",
                TestOutput::ExitCode(0),
                TestOutput::ExitCode(0),
                DiffResult::Identical,
            )),
            changes: "something".into(),
        });
        summary.record(&SnapshotComparison::Updated {
            path: PathBuf::from("/tmp/test.snap.json"),
        });

        assert_eq!(summary.matched, 2);
        assert_eq!(summary.new, 1);
        assert_eq!(summary.changed, 1);
        assert_eq!(summary.updated, 1);
        assert_eq!(summary.total(), 5);
        assert!(!summary.is_clean());
    }

    #[test]
    fn test_snapshot_summary_is_clean() {
        let mut summary = SnapshotSummary::default();
        summary.record(&SnapshotComparison::Match);
        summary.record(&SnapshotComparison::Updated {
            path: PathBuf::from("/tmp/x"),
        });
        // Updated doesn't count as dirty — only Changed and New do
        assert!(summary.is_clean());
    }

    #[test]
    fn test_snapshot_summary_display() {
        let mut summary = SnapshotSummary::default();
        summary.matched = 10;
        summary.changed = 2;
        summary.new = 1;
        summary.updated = 0;
        let display = summary.to_string();
        assert!(display.contains("13 total"));
        assert!(display.contains("10 matched"));
        assert!(display.contains("2 changed"));
        assert!(display.contains("1 new"));
        assert!(display.contains("0 updated"));
    }

    #[test]
    fn test_snapshot_roundtrip_all_output_types() {
        let (_dir, store) = temp_store();

        let outputs = vec![
            ("text", TestOutput::RawText("hello\nworld".into())),
            ("exit", TestOutput::ExitCode(42)),
            (
                "json",
                TestOutput::StructuredJson(serde_json::json!({"key": "value"})),
            ),
            ("binary", TestOutput::BinaryBlob(vec![0, 1, 2, 255])),
            ("unavailable", TestOutput::Unavailable("no systemd".into())),
            (
                "dbus",
                TestOutput::DBusPropertyMap({
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("ActiveState".into(), "active".into());
                    m
                }),
            ),
            (
                "tree",
                TestOutput::FileTreeSnapshot({
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("file.txt".into(), "hash123".into());
                    m
                }),
            ),
            (
                "composite",
                TestOutput::Composite(vec![
                    ("stdout".into(), TestOutput::RawText("out".into())),
                    ("code".into(), TestOutput::ExitCode(0)),
                ]),
            ),
        ];

        for (name, output) in &outputs {
            let snap = SnapshotFile::new(
                *name,
                "roundtrip",
                output.clone(),
                output.clone(),
                DiffResult::Identical,
            );
            store.save(&snap).unwrap();

            let loaded = store.load("roundtrip", name).unwrap().unwrap();
            assert_eq!(
                loaded.systemd_output, *output,
                "roundtrip failed for {name}"
            );
            assert_eq!(
                loaded.systemd_rs_output, *output,
                "roundtrip failed for {name}"
            );
        }
    }

    #[test]
    fn test_snapshot_roundtrip_all_diff_results() {
        let (_dir, store) = temp_store();

        let results = vec![
            ("identical", DiffResult::Identical),
            (
                "equivalent",
                DiffResult::Equivalent("PIDs normalized".into()),
            ),
            ("divergent", DiffResult::Divergent("mismatch".into())),
            ("skipped", DiffResult::Skipped("no systemd".into())),
        ];

        let output = TestOutput::ExitCode(0);

        for (name, result) in &results {
            let snap = SnapshotFile::new(
                *name,
                "results",
                output.clone(),
                output.clone(),
                result.clone(),
            );
            store.save(&snap).unwrap();

            let loaded = store.load("results", name).unwrap().unwrap();
            assert_eq!(loaded.result, *result, "roundtrip failed for {name}");
        }
    }

    #[test]
    fn test_is_update_mode_default_false() {
        // Remove the env var if it exists (can't guarantee it's not set)
        // SAFETY: This test is not run in parallel with other tests that
        // depend on this environment variable.
        unsafe {
            std::env::remove_var(UPDATE_SNAPSHOTS_ENV);
        }
        assert!(!is_update_mode());
    }
}
