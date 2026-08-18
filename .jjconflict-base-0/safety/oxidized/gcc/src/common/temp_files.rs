//! Proper temp file handling with RAII cleanup and $TMPDIR support.
//!
//! Provides:
//! - `temp_dir()`: returns the platform temp directory (respects $TMPDIR)
//! - `TempFile`: RAII guard that deletes the file on drop (even on panic/error)
//! - `make_temp_path()`: generates a unique temp file path

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Global counter for generating unique temp file names within a process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Returns the temp directory, respecting $TMPDIR (via std::env::temp_dir()).
///
/// On Unix, this checks $TMPDIR first, falling back to /tmp.
/// On other platforms, it uses the OS-appropriate temp directory.
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir()
}

/// Generate a unique temp file path with the given prefix and extension.
///
/// The path includes the PID and an atomic counter to avoid collisions
/// in parallel builds and multi-file compilations.
pub fn make_temp_path(prefix: &str, stem: &str, extension: &str) -> PathBuf {
    let pid = std::process::id();
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let filename = format!("{}_{}_{}.{}.{}", prefix, pid, stem, id, extension);
    temp_dir().join(filename)
}

/// RAII guard for a temporary file. Deletes the file when dropped.
///
/// This ensures cleanup happens even on early returns, panics, or errors.
/// The file path is accessible via `path()` and `to_str()`.
pub struct TempFile {
    path: PathBuf,
    /// If true, the file will NOT be deleted on drop (for debugging).
    keep: bool,
}

impl TempFile {
    /// Create a new TempFile guard for a unique temp path.
    ///
    /// The file is not created on disk — this just reserves the path and
    /// registers it for cleanup on drop.
    pub fn new(prefix: &str, stem: &str, extension: &str) -> Self {
        Self {
            path: make_temp_path(prefix, stem, extension),
            keep: false,
        }
    }

    /// Create a TempFile with a specific path (for cases where the caller
    /// controls the path but wants RAII cleanup).
    #[cfg_attr(not(feature = "gcc_assembler"), allow(dead_code))] // Used by gcc_assembler's CCC_KEEP_ASM path
    pub fn with_path(path: PathBuf) -> Self {
        Self { path, keep: false }
    }

    /// Mark this temp file to be kept (not deleted on drop).
    /// Useful for debugging with CCC_KEEP_ASM etc.
    #[cfg_attr(not(feature = "gcc_assembler"), allow(dead_code))] // Used by gcc_assembler's CCC_KEEP_ASM path
    pub fn set_keep(&mut self, keep: bool) {
        self.keep = keep;
    }

    /// Get the path as a Path reference.
    #[cfg_attr(not(feature = "gcc_assembler"), allow(dead_code))] // Used by gcc_assembler's assemble_with_extra()
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the path as a string slice (panics if not valid UTF-8).
    pub fn to_str(&self) -> &str {
        self.path
            .to_str()
            .expect("temp file path is not valid UTF-8")
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.keep {
            // Silently ignore errors (file may not have been created).
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl std::fmt::Display for TempFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path.display())
    }
}
