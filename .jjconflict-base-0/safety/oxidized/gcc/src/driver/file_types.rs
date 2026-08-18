//! File type detection utilities for the compiler driver.
//!
//! Classifies input files by extension or magic bytes to determine how they
//! should be processed: C source files go through the full pipeline, assembly
//! files go to the assembler, and object/archive files pass through to the
//! linker directly.

use super::Driver;

impl Driver {
    /// Check if a file is an object file or archive (pass to linker directly).
    /// Recognizes standard extensions (.o, .a, .so) plus common variants used by
    /// build systems (.os, .od, .lo, .obj), versioned shared libs (.so.*), and
    /// suffixed archives (.a.*) like the `.a.xyzzy` pattern used by skarnet.org
    /// build systems (skalibs, s6, etc.).
    pub(super) fn is_object_or_archive(path: &str) -> bool {
        // Standard extensions
        if path.ends_with(".o") || path.ends_with(".a") || path.ends_with(".so") {
            return true;
        }
        // Common non-standard extensions used by build systems:
        // .os - heatshrink uses for static-variant objects
        // .od - heatshrink uses for dynamic-variant objects
        // .lo - libtool object files
        // .obj - Windows-style object files sometimes used in cross-platform projects
        if path.ends_with(".os")
            || path.ends_with(".od")
            || path.ends_with(".lo")
            || path.ends_with(".obj")
        {
            return true;
        }
        // Versioned shared libraries: .so.1, .so.1.2.3, etc.
        if let Some(pos) = path.rfind(".so.") {
            let after = &path[pos + 4..];
            if after.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                return true;
            }
        }
        // Suffixed static archives: .a.xyzzy, .a.build, etc.
        // The skarnet.org build system (used by skalibs, s6, execline) uses
        // `lib%.a.xyzzy` as a build-time archive name to avoid collisions with
        // installed library names. These are regular `ar` archives and must be
        // passed through to the linker in their original command-line position.
        // Only match `.a.` in the filename component (after the last `/`), not
        // in directory names, to avoid false positives like `/path/to/data.a.dir/main.c`.
        let filename = path.rsplit('/').next().unwrap_or(path);
        if filename.contains(".a.") {
            return true;
        }
        false
    }

    /// Check if a file has a known C source extension.
    pub(super) fn is_c_source(path: &str) -> bool {
        path.ends_with(".c") || path.ends_with(".h") || path.ends_with(".i")
    }

    /// Check if a file appears to be a binary object/archive by inspecting magic bytes.
    /// Returns true for ELF files (\x7fELF) and ar archives (!<arch>\n).
    pub(super) fn looks_like_binary_object(path: &str) -> bool {
        use std::io::Read;
        let mut buf = [0u8; 8];
        if let Ok(mut f) = std::fs::File::open(path) {
            if let Ok(n) = f.read(&mut buf) {
                if n >= 4 && &buf[..4] == b"\x7fELF" {
                    return true;
                }
                if n >= 8 && &buf[..8] == b"!<arch>\n" {
                    return true;
                }
            }
        }
        false
    }

    /// Check if a file is an assembly source (.s or .S).
    /// .S files contain assembly with C preprocessor directives.
    /// .s files contain pure assembly.
    /// Both are passed to the target assembler (gcc) directly.
    pub(super) fn is_assembly_source(path: &str) -> bool {
        path.ends_with(".s") || path.ends_with(".S")
    }

    /// Check if an input file should be treated as assembly source based on
    /// the -x language override. This is used for stdin ("-") and for files
    /// like /dev/null where the extension doesn't indicate assembly.
    pub(super) fn is_explicit_assembly(&self) -> bool {
        matches!(
            self.explicit_language.as_deref(),
            Some("assembler") | Some("assembler-with-cpp")
        )
    }

    /// Strip GCC-style line markers (`# <num> "file"`) from preprocessed output.
    /// Used when -P flag is set. Filters out lines matching `# <digit>...` while
    /// preserving all other lines (including blank lines) verbatim.
    pub(super) fn strip_line_markers(input: &str) -> String {
        fn is_line_marker(line: &str) -> bool {
            let trimmed = line.trim_start();
            if !trimmed.starts_with('#') {
                return false;
            }
            trimmed[1..]
                .trim_start()
                .starts_with(|c: char| c.is_ascii_digit())
        }

        let mut result: String = input
            .lines()
            .filter(|line| !is_line_marker(line))
            .collect::<Vec<_>>()
            .join("\n");
        if input.ends_with('\n') {
            result.push('\n');
        }
        result
    }
}
