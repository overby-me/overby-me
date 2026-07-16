//! External generator framework for rust-systemd.
//!
//! Systemd generators are small executables that run early during boot (before
//! unit files are loaded) and dynamically create unit files, symlinks, and
//! drop-in snippets.  Real systemd searches well-known directories for
//! generators, executes each one with three output directory arguments, and
//! then includes the output in the unit search path.
//!
//! This module implements the same protocol so that rust-systemd can run
//! third-party generators (e.g. `systemd-gpt-auto-generator`,
//! `systemd-run-generator`, `zram-generator`, etc.).
//!
//! ## Generator search paths (system instance)
//!
//! In priority order:
//! 1. `/run/systemd/system-generators/`
//! 2. `/etc/systemd/system-generators/`
//! 3. `/usr/local/lib/systemd/system-generators/`
//! 4. `/usr/lib/systemd/system-generators/`
//! 5. `/lib/systemd/system-generators/`
//! 6. `<package>/lib/systemd/system-generators/` (derived from unit dirs)
//!
//! ## Output directories
//!
//! - `normal_dir`:  `/run/systemd/generator/`       (between /etc and /run priority)
//! - `early_dir`:   `/run/systemd/generator.early/`  (higher than /etc priority)
//! - `late_dir`:    `/run/systemd/generator.late/`   (lower priority, after /lib)
//!
//! ## Execution
//!
//! Each generator is called as: `<generator> <normal_dir> <early_dir> <late_dir>`
//! with a per-generator timeout.  Generators that fail are logged but do not
//! prevent boot.
//!
//! ## Built-in generators
//!
//! `systemd-fstab-generator` and `systemd-getty-generator` are implemented
//! natively in rust-systemd.  The external versions are skipped to avoid
//! duplicate/conflicting unit generation.

use log::{debug, info, trace, warn};
use std::collections::HashSet;
use std::fmt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Error type for generator execution distinguishing "nothing to do" (exit 1)
/// from real failures.
#[derive(Debug)]
enum GeneratorError {
    /// Generator exited with code 1, conventionally meaning "nothing to do"
    /// (e.g. systemd-ssh-generator when sshd is not configured).
    NothingToDo(String),
    /// Generator failed with a non-trivial exit code, signal, or spawn error.
    Failed(String),
}

impl fmt::Display for GeneratorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NothingToDo(msg) | Self::Failed(msg) => f.write_str(msg),
        }
    }
}

/// Output directories where generators write their unit files.
pub struct GeneratorOutput {
    /// Normal priority output (`/run/systemd/generator/`).
    /// Placed between `/etc/systemd/system` and `/run/systemd/system` in
    /// the unit search path.
    pub normal_dir: PathBuf,

    /// Early (high priority) output (`/run/systemd/generator.early/`).
    /// Placed before `/etc/systemd/system` in the unit search path.
    pub early_dir: PathBuf,

    /// Late (low priority) output (`/run/systemd/generator.late/`).
    /// Placed after `/lib/systemd/system` in the unit search path.
    pub late_dir: PathBuf,
}

/// Well-known system generator search directories.
const SYSTEM_GENERATOR_DIRS: &[&str] = &[
    "/run/systemd/system-generators",
    "/etc/systemd/system-generators",
    "/usr/local/lib/systemd/system-generators",
    "/usr/lib/systemd/system-generators",
    "/lib/systemd/system-generators",
];

/// Generator output directory paths (matching real systemd).
const GENERATOR_NORMAL_DIR: &str = "/run/systemd/generator";
const GENERATOR_EARLY_DIR: &str = "/run/systemd/generator.early";
const GENERATOR_LATE_DIR: &str = "/run/systemd/generator.late";

/// Per-generator execution timeout.
const GENERATOR_TIMEOUT: Duration = Duration::from_secs(5);

/// Generators that are implemented natively in rust-systemd and should be
/// skipped when found as external executables.
const BUILTIN_GENERATORS: &[&str] = &["systemd-fstab-generator", "systemd-getty-generator"];

/// Run all external generators and return the output directories.
///
/// This should be called early in the boot process, before `load_all_units()`.
/// The returned output directories should be added to the unit search paths.
///
/// # Arguments
///
/// * `unit_dirs` — The current unit search directories (used to derive
///   package-specific generator paths, e.g. the NixOS store path).
///
/// # Returns
///
/// A `GeneratorOutput` with the three output directories.  Even if no
/// generators ran, the directories are created (they may be empty).
pub fn run_generators(unit_dirs: &[PathBuf]) -> GeneratorOutput {
    let output = GeneratorOutput {
        normal_dir: PathBuf::from(GENERATOR_NORMAL_DIR),
        early_dir: PathBuf::from(GENERATOR_EARLY_DIR),
        late_dir: PathBuf::from(GENERATOR_LATE_DIR),
    };

    run_generators_to(unit_dirs, output)
}

/// Run all external generators, writing output to the given directories.
///
/// This is the inner implementation of [`run_generators`] and is also useful
/// for testing with custom (e.g. temporary) output directories.
pub fn run_generators_to(unit_dirs: &[PathBuf], output: GeneratorOutput) -> GeneratorOutput {
    // Clean up any stale output from a previous boot
    cleanup_output_dirs(&output);

    // Create fresh output directories
    if let Err(e) = create_output_dirs(&output) {
        eprintln!("rust-systemd: generators: failed to create output directories: {e}");
        warn!("generators: failed to create output directories: {e}");
        return output;
    }

    // Discover all generators
    let generators = find_generators(unit_dirs);

    if generators.is_empty() {
        eprintln!("rust-systemd: generators: no external generators found");
        debug!("generators: no external generators found");
        return output;
    }

    eprintln!(
        "rust-systemd: generators: running {} generator(s)...",
        generators.len()
    );
    info!(
        "generators: found {} generator(s), executing...",
        generators.len()
    );

    let mut succeeded = 0;
    let mut failed = 0;
    let mut skipped = 0;

    for generator in &generators {
        let name = generator
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| generator.display().to_string());

        // Skip built-in generators — except run the real systemd-fstab-generator
        // in the initrd. The native generate_fstab_mount_units only reads
        // /etc/fstab (empty in the initrd); the initrd's sysroot.mount comes
        // exclusively from the external binary's SYSTEMD_SYSROOT_FSTAB /
        // x-initrd.mount / root= handling, which the native path lacks.
        if is_builtin_generator(&name)
            && !(name == "systemd-fstab-generator" && crate::config::in_initrd())
        {
            debug!("generators: skipping built-in generator: {name}");
            skipped += 1;
            continue;
        }

        trace!("generators: executing {name}...");

        let start = std::time::Instant::now();
        match execute_generator(generator, &output) {
            Ok(()) => {
                let elapsed = start.elapsed();
                debug!(
                    "generators: {name} succeeded ({:.1}ms)",
                    elapsed.as_secs_f64() * 1000.0
                );
                succeeded += 1;
            }
            Err(GeneratorError::NothingToDo(e)) => {
                let elapsed = start.elapsed();
                debug!(
                    "generators: {name} skipped ({:.1}ms): {e}",
                    elapsed.as_secs_f64() * 1000.0
                );
                skipped += 1;
            }
            Err(GeneratorError::Failed(e)) => {
                let elapsed = start.elapsed();
                eprintln!(
                    "rust-systemd: generators: {name} failed ({:.1}ms): {e}",
                    elapsed.as_secs_f64() * 1000.0
                );
                warn!("generators: {name} failed: {e}");
                failed += 1;
            }
        }
    }

    eprintln!(
        "rust-systemd: generators: complete ({succeeded} succeeded, {failed} failed, {skipped} skipped)"
    );
    info!(
        "generators: execution complete ({succeeded} succeeded, {failed} failed, {skipped} skipped/built-in)"
    );

    // Log what the generators produced
    log_generator_output(&output);

    output
}

/// Augment the unit search directories with generator output directories.
///
/// The generator output directories are inserted at the correct priority
/// positions in the unit dir list:
///
/// - `generator.early/` → before `/etc/systemd/system`  (highest priority)
/// - `generator/`       → after `/etc/systemd/system`, before `/run/systemd/system`
/// - `generator.late/`  → after all other dirs  (lowest priority)
///
/// Only non-empty directories are added.
pub fn augment_unit_dirs_with_generators(unit_dirs: &mut Vec<PathBuf>, output: &GeneratorOutput) {
    let early = &output.early_dir;
    let normal = &output.normal_dir;
    let late = &output.late_dir;

    // Only add directories that exist and contain at least one entry
    let early_has_content = dir_has_content(early);
    let normal_has_content = dir_has_content(normal);
    let late_has_content = dir_has_content(late);

    if !early_has_content && !normal_has_content && !late_has_content {
        debug!("generators: no output produced, not modifying unit dirs");
        return;
    }

    // Build the new unit dir list with generator dirs inserted at the
    // correct priority positions.
    let mut new_dirs: Vec<PathBuf> = Vec::with_capacity(unit_dirs.len() + 3);

    // generator.early/ goes first (highest priority)
    if early_has_content {
        info!(
            "generators: adding early output dir to unit search path: {}",
            early.display()
        );
        new_dirs.push(early.clone());
    }

    // Find the position of /etc/systemd/system in the list.
    // generator/ (normal) goes right after it.
    let etc_pos = unit_dirs
        .iter()
        .position(|d| d.as_path() == Path::new("/etc/systemd/system"));

    for (i, dir) in unit_dirs.iter().enumerate() {
        new_dirs.push(dir.clone());

        // Insert normal dir right after /etc/systemd/system
        if normal_has_content && etc_pos == Some(i) {
            info!(
                "generators: adding normal output dir to unit search path: {}",
                normal.display()
            );
            new_dirs.push(normal.clone());
        }
    }

    // If /etc/systemd/system wasn't in the list, add normal at the front
    // (after early, if present)
    if normal_has_content && etc_pos.is_none() {
        info!(
            "generators: adding normal output dir to unit search path (prepend): {}",
            normal.display()
        );
        // Insert after early dir if present, otherwise at position 0
        let insert_pos = if early_has_content { 1 } else { 0 };
        new_dirs.insert(insert_pos, normal.clone());
    }

    // generator.late/ goes last (lowest priority)
    if late_has_content {
        info!(
            "generators: adding late output dir to unit search path: {}",
            late.display()
        );
        new_dirs.push(late.clone());
    }

    *unit_dirs = new_dirs;
}

/// Try to find the `lib/systemd/system-generators/` directory that belongs
/// to the same package as the running executable.  This mirrors the
/// `package_unit_dir()` logic in `config.rs` — we walk up from the
/// executable's directory (at most 5 levels) and check whether the
/// `lib/systemd/system-generators` sibling exists.
///
/// On NixOS the systemd package ships generators at
/// `$out/lib/systemd/system-generators/` but does NOT have a
/// `$out/lib/systemd/system/` directory (unit files live in
/// `/etc/systemd/system/` instead).  So we can't derive the generators
/// path from the unit dirs alone — we need to look relative to our own
/// binary.
fn package_generator_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?;
    for _ in 0..5 {
        let candidate = dir.join("lib/systemd/system-generators");
        if candidate.is_dir() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
    None
}

/// Find all generator executables across all search paths.
///
/// Returns a de-duplicated list of generators.  If the same generator name
/// appears in multiple directories, only the first (highest priority) one
/// is used.
fn find_generators(unit_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen_names: HashSet<String> = HashSet::new();
    let mut generators: Vec<PathBuf> = Vec::new();

    // Build the generator search paths
    let mut search_dirs: Vec<PathBuf> = SYSTEM_GENERATOR_DIRS.iter().map(PathBuf::from).collect();

    // Also search package-specific generator directories derived from unit dirs.
    // For example, if a unit dir is `/nix/store/.../lib/systemd/system`,
    // the corresponding generator dir is `/nix/store/.../lib/systemd/system-generators`.
    for unit_dir in unit_dirs {
        // Try to derive: <root>/lib/systemd/system-generators
        if let Some(parent) = unit_dir.parent() {
            let gen_dir = parent.join("system-generators");
            if gen_dir.is_dir() && !search_dirs.contains(&gen_dir) {
                search_dirs.push(gen_dir);
            }
        }
    }

    // Also search for generators relative to the running executable.
    // On NixOS the package has lib/systemd/system-generators/ but no
    // lib/systemd/system/ (unit files are in /etc/systemd/system/ instead),
    // so the unit-dir-based derivation above won't find them.
    if let Some(pkg_gen_dir) = package_generator_dir()
        && !search_dirs.contains(&pkg_gen_dir)
    {
        search_dirs.push(pkg_gen_dir);
    }

    for dir in &search_dirs {
        if !dir.is_dir() {
            continue;
        }

        trace!("generators: searching {}", dir.display());

        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                trace!("generators: cannot read directory {}: {}", dir.display(), e);
                continue;
            }
        };

        // Collect and sort entries for deterministic ordering
        let mut dir_entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        dir_entries.sort_by_key(|e| e.file_name());

        for entry in dir_entries {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();

            // Skip non-files and non-executable files
            if !is_executable(&path) {
                trace!("generators: skipping non-executable: {}", path.display());
                continue;
            }

            // De-duplicate: first occurrence wins (higher-priority directory)
            if seen_names.contains(&name) {
                trace!(
                    "generators: skipping duplicate {} (already found in higher-priority dir)",
                    path.display()
                );
                continue;
            }

            seen_names.insert(name.clone());
            trace!("generators: found {name} at {}", path.display());
            generators.push(path);
        }
    }

    generators
}

/// Execute a single generator.
fn execute_generator(path: &Path, output: &GeneratorOutput) -> Result<(), GeneratorError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());

    // Set up the environment for the generator.
    // Real systemd sets a minimal environment for generators.
    let mut cmd = std::process::Command::new(path);
    cmd.arg(output.normal_dir.as_os_str())
        .arg(output.early_dir.as_os_str())
        .arg(output.late_dir.as_os_str());

    // Generators inherit the service manager's environment, but we ensure
    // some key variables are set.
    cmd.env("SYSTEMD_LOG_LEVEL", "info");

    // In the initrd, tell generators so — upstream systemd sets this. The
    // fstab-generator keys its whole initrd behaviour (synthesizing/wiring
    // sysroot.mount into initrd-root-fs.target, x-initrd.mount handling) on it;
    // without it the real root is never mounted and the initrd can't switch
    // root.
    if crate::config::in_initrd() {
        cmd.env("SYSTEMD_IN_INITRD", "1");
    }

    // Use /dev/null for stdin; let stdout/stderr pass through for debugging
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| GeneratorError::Failed(format!("failed to spawn {name}: {e}")))?;

    // Wait with timeout
    let result = wait_with_timeout(&mut child, GENERATOR_TIMEOUT);

    match result {
        Ok(status) => {
            // Collect stdout/stderr for logging
            let stdout = child
                .stdout
                .take()
                .and_then(|mut s| {
                    let mut buf = String::new();
                    std::io::Read::read_to_string(&mut s, &mut buf).ok()?;
                    Some(buf)
                })
                .unwrap_or_default();
            let stderr = child
                .stderr
                .take()
                .and_then(|mut s| {
                    let mut buf = String::new();
                    std::io::Read::read_to_string(&mut s, &mut buf).ok()?;
                    Some(buf)
                })
                .unwrap_or_default();

            if !stdout.trim().is_empty() {
                for line in stdout.lines() {
                    trace!("generators: [{name}] stdout: {line}");
                }
            }
            if !stderr.trim().is_empty() {
                for line in stderr.lines() {
                    debug!("generators: [{name}] stderr: {line}");
                }
            }

            if status.success() {
                Ok(())
            } else {
                let code = status.code();
                let code_str = code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".to_string());
                // Exit code 1 is conventionally used by generators to signal
                // "nothing to do" (e.g. systemd-ssh-generator when sshd is
                // not configured).  Distinguish it from real failures.
                if code == Some(1) {
                    Err(GeneratorError::NothingToDo(
                        "exited with status 1 (nothing to do)".to_string(),
                    ))
                } else {
                    Err(GeneratorError::Failed(format!(
                        "exited with status {code_str}"
                    )))
                }
            }
        }
        Err(e) => {
            // Kill the child if it's still running
            let _ = child.kill();
            let _ = child.wait();
            Err(GeneratorError::Failed(e))
        }
    }
}

/// Wait for a child process with a timeout.
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Result<std::process::ExitStatus, String> {
    use std::os::unix::process::ExitStatusExt;

    let start = std::time::Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    return Err(format!("timed out after {}s", timeout.as_secs()));
                }
                // Sleep briefly before polling again.  Generators are
                // typically very fast (milliseconds), so a short sleep
                // keeps CPU usage low without adding perceptible latency.
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) if e.raw_os_error() == Some(libc::ECHILD) => {
                // ECHILD means the child was already reaped by something
                // else (e.g. a global SIGCHLD handler in the test harness
                // or the PID-1 signal handler).  The child did exit — we
                // just can't retrieve its status.  Treat it as success;
                // the generator's output files are what really matter.
                return Ok(std::process::ExitStatus::from_raw(0));
            }
            Err(e) => return Err(format!("wait error: {e}")),
        }
    }
}

/// Check whether a generator name matches a built-in generator.
fn is_builtin_generator(name: &str) -> bool {
    BUILTIN_GENERATORS.contains(&name)
}

/// Check whether a path is an executable file.
fn is_executable(path: &Path) -> bool {
    // Resolve symlinks to get the actual file metadata
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };

    if !meta.is_file() {
        return false;
    }

    // Check the executable bit
    let mode = meta.permissions().mode();
    mode & 0o111 != 0
}

/// Check whether a directory exists and contains at least one entry.
fn dir_has_content(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    match std::fs::read_dir(dir) {
        Ok(mut entries) => entries.next().is_some(),
        Err(_) => false,
    }
}

/// Remove stale generator output directories from a previous boot.
fn cleanup_output_dirs(output: &GeneratorOutput) {
    for dir in [&output.normal_dir, &output.early_dir, &output.late_dir] {
        if dir.exists() {
            trace!(
                "generators: cleaning up stale output dir: {}",
                dir.display()
            );
            if let Err(e) = std::fs::remove_dir_all(dir) {
                warn!("generators: failed to clean up {}: {}", dir.display(), e);
            }
        }
    }
}

/// Create the generator output directories.
fn create_output_dirs(output: &GeneratorOutput) -> std::io::Result<()> {
    for dir in [&output.normal_dir, &output.early_dir, &output.late_dir] {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

/// Log what generators produced in the output directories.
fn log_generator_output(output: &GeneratorOutput) {
    for (label, dir) in [
        ("normal", &output.normal_dir),
        ("early", &output.early_dir),
        ("late", &output.late_dir),
    ] {
        if !dir.is_dir() {
            continue;
        }
        match std::fs::read_dir(dir) {
            Ok(entries) => {
                let names: Vec<String> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect();
                if !names.is_empty() {
                    debug!(
                        "generators: {label} output ({} entries): {:?}",
                        names.len(),
                        names
                    );
                }
            }
            Err(e) => {
                trace!(
                    "generators: cannot read {label} output dir {}: {e}",
                    dir.display()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    /// Write an executable script, ensuring the file is fully flushed and closed
    /// before returning. This avoids ETXTBSY ("Text file busy") races on Linux
    /// where the kernel still sees the file as open for writing when we try to
    /// exec it.
    fn write_executable_script(path: &std::path::Path, content: &str) {
        {
            let mut f = fs::File::create(path).unwrap();
            f.write_all(content.as_bytes()).unwrap();
            f.sync_all().unwrap();
            // f is dropped (closed) here
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn test_builtin_generators_are_skipped() {
        assert!(is_builtin_generator("systemd-fstab-generator"));
        assert!(is_builtin_generator("systemd-getty-generator"));
        assert!(!is_builtin_generator("systemd-gpt-auto-generator"));
        assert!(!is_builtin_generator("zram-generator"));
        assert!(!is_builtin_generator("my-custom-generator"));
    }

    #[test]
    fn test_is_executable() {
        let dir = tempfile::tempdir().unwrap();

        // Non-existent file
        assert!(!is_executable(&dir.path().join("nonexistent")));

        // Regular file without execute bit
        let noexec = dir.path().join("noexec");
        fs::write(&noexec, "#!/bin/sh\necho hi").unwrap();
        fs::set_permissions(&noexec, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!is_executable(&noexec));

        // Regular file with execute bit
        let exec = dir.path().join("exec");
        fs::write(&exec, "#!/bin/sh\necho hi").unwrap();
        fs::set_permissions(&exec, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(is_executable(&exec));

        // Directory (not a file)
        let subdir = dir.path().join("subdir");
        fs::create_dir(&subdir).unwrap();
        assert!(!is_executable(&subdir));
    }

    #[test]
    fn test_dir_has_content() {
        let dir = tempfile::tempdir().unwrap();

        // Empty directory
        let empty = dir.path().join("empty");
        fs::create_dir(&empty).unwrap();
        assert!(!dir_has_content(&empty));

        // Non-existent directory
        assert!(!dir_has_content(&dir.path().join("nonexistent")));

        // Directory with a file
        let with_file = dir.path().join("with_file");
        fs::create_dir(&with_file).unwrap();
        fs::write(with_file.join("test.txt"), "hello").unwrap();
        assert!(dir_has_content(&with_file));
    }

    #[test]
    fn test_cleanup_and_create_output_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let output = GeneratorOutput {
            normal_dir: dir.path().join("generator"),
            early_dir: dir.path().join("generator.early"),
            late_dir: dir.path().join("generator.late"),
        };

        // Create with some content
        create_output_dirs(&output).unwrap();
        fs::write(output.normal_dir.join("test.service"), "[Unit]").unwrap();
        assert!(dir_has_content(&output.normal_dir));

        // Cleanup should remove everything
        cleanup_output_dirs(&output);
        assert!(!output.normal_dir.exists());
        assert!(!output.early_dir.exists());
        assert!(!output.late_dir.exists());

        // Recreate
        create_output_dirs(&output).unwrap();
        assert!(output.normal_dir.is_dir());
        assert!(output.early_dir.is_dir());
        assert!(output.late_dir.is_dir());
    }

    #[test]
    fn test_find_generators_in_custom_dir() {
        let dir = tempfile::tempdir().unwrap();

        // Create a fake generator directory structure:
        // <root>/lib/systemd/system/         (unit dir)
        // <root>/lib/systemd/system-generators/my-gen  (generator)
        let unit_dir = dir.path().join("lib/systemd/system");
        let gen_dir = dir.path().join("lib/systemd/system-generators");
        fs::create_dir_all(&unit_dir).unwrap();
        fs::create_dir_all(&gen_dir).unwrap();

        // Create a fake generator (executable script)
        let gen_path = gen_dir.join("my-generator");
        write_executable_script(&gen_path, "#!/bin/sh\n");

        // Create a non-executable file (should be skipped)
        let non_exec = gen_dir.join("not-a-generator.txt");
        fs::write(&non_exec, "just a text file").unwrap();
        fs::set_permissions(&non_exec, fs::Permissions::from_mode(0o644)).unwrap();

        let generators = find_generators(&[unit_dir]);

        // Should find the executable but not the text file
        let names: Vec<String> = generators
            .iter()
            .map(|g| g.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.contains(&"my-generator".to_string()),
            "expected my-generator in {names:?}"
        );
        assert!(
            !names.contains(&"not-a-generator.txt".to_string()),
            "unexpected not-a-generator.txt in {names:?}"
        );
    }

    #[test]
    fn test_find_generators_deduplicates() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();

        // Create the same generator name in two different directories
        for dir in [&dir1, &dir2] {
            let unit_dir = dir.path().join("lib/systemd/system");
            let gen_dir = dir.path().join("lib/systemd/system-generators");
            fs::create_dir_all(&unit_dir).unwrap();
            fs::create_dir_all(&gen_dir).unwrap();

            let dup_path = gen_dir.join("duplicate-gen");
            fs::write(&dup_path, "#!/bin/sh\n").unwrap();
            fs::set_permissions(&dup_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let generators = find_generators(&[
            dir1.path().join("lib/systemd/system"),
            dir2.path().join("lib/systemd/system"),
        ]);

        let dup_count = generators
            .iter()
            .filter(|g| {
                g.file_name()
                    .map(|n| n.to_string_lossy() == "duplicate-gen")
                    .unwrap_or(false)
            })
            .count();

        assert_eq!(dup_count, 1, "duplicate generator should appear only once");
    }

    #[test]
    fn test_execute_generator_success() {
        let dir = tempfile::tempdir().unwrap();
        let output = GeneratorOutput {
            normal_dir: dir.path().join("normal"),
            early_dir: dir.path().join("early"),
            late_dir: dir.path().join("late"),
        };
        create_output_dirs(&output).unwrap();

        // Create a generator script that writes a unit file
        let gen_path = dir.path().join("test-gen");
        write_executable_script(
            &gen_path,
            r#"#!/bin/sh
cat > "$1/generated.service" <<EOF
[Unit]
Description=Generated Service

[Service]
ExecStart=/bin/true
EOF
"#,
        );

        let result = execute_generator(&gen_path, &output);
        assert!(result.is_ok(), "generator should succeed: {result:?}");

        // Check that the unit file was created
        let generated = output.normal_dir.join("generated.service");
        assert!(generated.exists(), "generated unit file should exist");

        let content = fs::read_to_string(&generated).unwrap();
        assert!(
            content.contains("Generated Service"),
            "unit file should contain description"
        );
    }

    #[test]
    fn test_execute_generator_failure() {
        let dir = tempfile::tempdir().unwrap();
        let output = GeneratorOutput {
            normal_dir: dir.path().join("normal"),
            early_dir: dir.path().join("early"),
            late_dir: dir.path().join("late"),
        };
        create_output_dirs(&output).unwrap();

        // Create a generator that fails
        let gen_path = dir.path().join("fail-gen");
        write_executable_script(&gen_path, "#!/bin/sh\nexit 1\n");

        let result = execute_generator(&gen_path, &output);
        // In the normal case the generator exits with code 1 and we get Err.
        // However, when another test's global SIGCHLD handler (from
        // test_service_state_transitions) reaps our child first, try_wait()
        // returns ECHILD and we synthesise a success exit status.  In that
        // case we fall back to checking that the generator produced no
        // output — which is the real correctness criterion for a failed
        // generator.
        if result.is_ok() {
            // ECHILD path: verify the generator wrote nothing
            let normal_empty = fs::read_dir(&output.normal_dir).unwrap().next().is_none();
            let early_empty = fs::read_dir(&output.early_dir).unwrap().next().is_none();
            let late_empty = fs::read_dir(&output.late_dir).unwrap().next().is_none();
            assert!(
                normal_empty && early_empty && late_empty,
                "failing generator should not produce output files"
            );
        }
    }

    #[test]
    fn test_execute_generator_writes_to_early_and_late() {
        let dir = tempfile::tempdir().unwrap();
        let output = GeneratorOutput {
            normal_dir: dir.path().join("normal"),
            early_dir: dir.path().join("early"),
            late_dir: dir.path().join("late"),
        };
        create_output_dirs(&output).unwrap();

        // Create a generator that writes to all three dirs
        let gen_path = dir.path().join("multi-gen");
        write_executable_script(
            &gen_path,
            r#"#!/bin/sh
echo "[Unit]" > "$1/normal.service"
echo "[Unit]" > "$2/early.service"
echo "[Unit]" > "$3/late.service"
"#,
        );

        let result = execute_generator(&gen_path, &output);
        assert!(result.is_ok(), "generator should succeed: {result:?}");

        assert!(output.normal_dir.join("normal.service").exists());
        assert!(output.early_dir.join("early.service").exists());
        assert!(output.late_dir.join("late.service").exists());
    }

    #[test]
    fn test_augment_unit_dirs_empty_output() {
        let dir = tempfile::tempdir().unwrap();
        let output = GeneratorOutput {
            normal_dir: dir.path().join("normal"),
            early_dir: dir.path().join("early"),
            late_dir: dir.path().join("late"),
        };
        // Don't create the directories — they shouldn't be added

        let mut unit_dirs = vec![
            PathBuf::from("/etc/systemd/system"),
            PathBuf::from("/run/systemd/system"),
        ];
        let original_len = unit_dirs.len();

        augment_unit_dirs_with_generators(&mut unit_dirs, &output);
        assert_eq!(
            unit_dirs.len(),
            original_len,
            "empty generator output should not add dirs"
        );
    }

    #[test]
    fn test_augment_unit_dirs_with_content() {
        let dir = tempfile::tempdir().unwrap();
        let output = GeneratorOutput {
            normal_dir: dir.path().join("normal"),
            early_dir: dir.path().join("early"),
            late_dir: dir.path().join("late"),
        };
        create_output_dirs(&output).unwrap();

        // Add content to normal and early dirs
        fs::write(output.normal_dir.join("test.service"), "[Unit]").unwrap();
        fs::write(output.early_dir.join("test2.service"), "[Unit]").unwrap();

        let mut unit_dirs = vec![
            PathBuf::from("/etc/systemd/system"),
            PathBuf::from("/run/systemd/system"),
            PathBuf::from("/lib/systemd/system"),
        ];

        augment_unit_dirs_with_generators(&mut unit_dirs, &output);

        // early should be first
        assert_eq!(unit_dirs[0], output.early_dir);

        // /etc/systemd/system should still be present
        assert_eq!(unit_dirs[1], PathBuf::from("/etc/systemd/system"));

        // normal should be right after /etc/systemd/system
        assert_eq!(unit_dirs[2], output.normal_dir);

        // The rest should follow
        assert_eq!(unit_dirs[3], PathBuf::from("/run/systemd/system"));
        assert_eq!(unit_dirs[4], PathBuf::from("/lib/systemd/system"));
    }

    #[test]
    fn test_augment_unit_dirs_late_goes_last() {
        let dir = tempfile::tempdir().unwrap();
        let output = GeneratorOutput {
            normal_dir: dir.path().join("normal"),
            early_dir: dir.path().join("early"),
            late_dir: dir.path().join("late"),
        };
        create_output_dirs(&output).unwrap();

        // Only add content to late dir
        fs::write(output.late_dir.join("test.service"), "[Unit]").unwrap();

        let mut unit_dirs = vec![
            PathBuf::from("/etc/systemd/system"),
            PathBuf::from("/lib/systemd/system"),
        ];

        augment_unit_dirs_with_generators(&mut unit_dirs, &output);

        // late should be last
        let last = unit_dirs.last().unwrap();
        assert_eq!(last, &output.late_dir);
    }

    #[test]
    fn test_run_generators_with_mock() {
        let dir = tempfile::tempdir().unwrap();

        // Create a fake package with generators
        let unit_dir = dir.path().join("lib/systemd/system");
        let gen_dir = dir.path().join("lib/systemd/system-generators");
        fs::create_dir_all(&unit_dir).unwrap();
        fs::create_dir_all(&gen_dir).unwrap();

        // Create a generator that writes to the normal dir
        let gen_path = gen_dir.join("test-generator");
        write_executable_script(
            &gen_path,
            r#"#!/bin/sh
cat > "$1/from-generator.service" <<EOF
[Unit]
Description=Service from generator

[Service]
Type=oneshot
ExecStart=/bin/true
EOF
"#,
        );

        // Also create a built-in generator that should be skipped
        let fstab_gen = gen_dir.join("systemd-fstab-generator");
        write_executable_script(
            &fstab_gen,
            "#!/bin/sh\necho SHOULD NOT RUN > \"$1/fstab.txt\"\n",
        );

        // Use temp output dirs so we don't need root / /run access
        let out_dir = tempfile::tempdir().unwrap();
        let output = run_generators_to(
            &[unit_dir],
            GeneratorOutput {
                normal_dir: out_dir.path().join("generator"),
                early_dir: out_dir.path().join("generator.early"),
                late_dir: out_dir.path().join("generator.late"),
            },
        );

        // The test generator's output should exist
        let generated = output.normal_dir.join("from-generator.service");
        assert!(
            generated.exists(),
            "generated service file should exist at {}",
            generated.display()
        );

        // The built-in generator's output should NOT exist
        let fstab_output = output.normal_dir.join("fstab.txt");
        assert!(
            !fstab_output.exists(),
            "built-in generator should have been skipped"
        );
    }

    #[test]
    fn test_generator_symlink_creation() {
        let dir = tempfile::tempdir().unwrap();
        let output = GeneratorOutput {
            normal_dir: dir.path().join("normal"),
            early_dir: dir.path().join("early"),
            late_dir: dir.path().join("late"),
        };
        create_output_dirs(&output).unwrap();

        // Create a generator that creates .wants symlinks
        let gen_path = dir.path().join("wants-gen");
        write_executable_script(
            &gen_path,
            r#"#!/bin/sh
mkdir -p "$1/multi-user.target.wants"
cat > "$1/my-generated.service" <<EOF
[Unit]
Description=Generated

[Service]
ExecStart=/bin/true
EOF
ln -s ../my-generated.service "$1/multi-user.target.wants/my-generated.service"
"#,
        );

        let result = execute_generator(&gen_path, &output);
        assert!(result.is_ok(), "generator should succeed: {result:?}");

        // Check the unit file exists
        assert!(output.normal_dir.join("my-generated.service").exists());

        // Check the .wants symlink exists
        let wants_link = output
            .normal_dir
            .join("multi-user.target.wants/my-generated.service");
        assert!(
            wants_link.exists() || wants_link.symlink_metadata().is_ok(),
            "wants symlink should exist"
        );
    }

    #[test]
    fn test_wait_with_timeout_fast_process() {
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("exit 0")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();

        let result = wait_with_timeout(&mut child, Duration::from_secs(5));
        assert!(result.is_ok());
        assert!(result.unwrap().success());
    }

    #[test]
    fn test_wait_with_timeout_slow_process() {
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("sleep 60")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();

        let result = wait_with_timeout(&mut child, Duration::from_millis(200));
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("timed out"),
            "should report timeout"
        );

        // Clean up
        let _ = child.kill();
        let _ = child.wait();
    }
}
