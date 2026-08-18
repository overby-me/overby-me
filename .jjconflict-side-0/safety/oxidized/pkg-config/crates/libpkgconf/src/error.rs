//! Error types for libpkgconf.

use std::io;
use std::path::PathBuf;

/// Result type alias for libpkgconf operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur during pkg-config operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A requested package was not found.
    #[error("Package '{name}' was not found in the pkg-config search path")]
    PackageNotFound { name: String },

    /// A package was found but its version did not satisfy the constraint.
    #[error(
        "Package '{name}' version '{found}' does not satisfy constraint '{comparator} {required}'"
    )]
    VersionMismatch {
        name: String,
        found: String,
        required: String,
        comparator: String,
    },

    /// A package conflict was detected.
    #[error("Package '{name}' conflicts with '{conflicts_with}'")]
    PackageConflict {
        name: String,
        conflicts_with: String,
    },

    /// The dependency graph is broken (e.g. circular dependency or depth exceeded).
    #[error("Dependency graph error: {message}")]
    DependencyGraphError { message: String },

    /// Maximum traversal depth exceeded while walking the dependency graph.
    #[error("Maximum traversal depth ({depth}) exceeded while resolving '{name}'")]
    MaxDepthExceeded { name: String, depth: i32 },

    /// A .pc file could not be parsed.
    #[error("Parse error in '{path}' at line {line}: {message}")]
    ParseError {
        path: PathBuf,
        line: usize,
        message: String,
    },

    /// A variable reference could not be resolved.
    #[error("Undefined variable '{variable}' referenced in '{context}'")]
    UndefinedVariable { variable: String, context: String },

    /// A circular variable reference was detected.
    #[error("Circular variable reference detected for '{variable}'")]
    CircularVariableReference { variable: String },

    /// An I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// An invalid version string was encountered.
    #[error("Invalid version string: '{version}'")]
    InvalidVersion { version: String },

    /// An invalid comparator operator was encountered.
    #[error("Invalid comparator operator: '{operator}'")]
    InvalidComparator { operator: String },

    /// No packages were specified on the command line.
    #[error("Please specify at least one package name on the command line")]
    NoPackagesSpecified,

    /// A .pc file validation error.
    #[error("Validation error in '{path}': {message}")]
    ValidationError { path: PathBuf, message: String },

    /// Multiple errors occurred during dependency resolution.
    #[error("Multiple errors occurred:\n{}", format_errors(.0))]
    Multiple(Vec<Error>),
}

fn format_errors(errors: &[Error]) -> String {
    errors
        .iter()
        .enumerate()
        .map(|(i, e)| format!("  {}. {e}", i + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Error flags compatible with pkgconf's `PKGCONF_PKG_ERRF_*` constants.
/// These can be combined as bitflags to indicate multiple error conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorFlags(u32);

impl ErrorFlags {
    /// No error.
    pub const OK: Self = Self(0x0);
    /// Package not found.
    pub const PACKAGE_NOT_FOUND: Self = Self(0x1);
    /// Package version mismatch.
    pub const PACKAGE_VER_MISMATCH: Self = Self(0x2);
    /// Package conflict detected.
    pub const PACKAGE_CONFLICT: Self = Self(0x4);
    /// Dependency graph break.
    pub const DEPGRAPH_BREAK: Self = Self(0x8);

    /// Create a new empty (OK) error flags value.
    pub fn new() -> Self {
        Self::OK
    }

    /// Check if no errors are set.
    pub fn is_ok(self) -> bool {
        self.0 == 0
    }

    /// Combine two error flag sets.
    pub fn merge(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Check whether a specific flag is set.
    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Get the raw flag value.
    pub fn bits(self) -> u32 {
        self.0
    }
}

impl Default for ErrorFlags {
    fn default() -> Self {
        Self::OK
    }
}

impl From<&Error> for ErrorFlags {
    fn from(error: &Error) -> Self {
        match error {
            Error::PackageNotFound { .. } => Self::PACKAGE_NOT_FOUND,
            Error::VersionMismatch { .. } => Self::PACKAGE_VER_MISMATCH,
            Error::PackageConflict { .. } => Self::PACKAGE_CONFLICT,
            Error::DependencyGraphError { .. } | Error::MaxDepthExceeded { .. } => {
                Self::DEPGRAPH_BREAK
            }
            Error::Multiple(errors) => errors.iter().fold(Self::OK, |acc, e| acc.merge(e.into())),
            _ => Self::OK,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_flags_default_is_ok() {
        let flags = ErrorFlags::default();
        assert!(flags.is_ok());
        assert_eq!(flags.bits(), 0);
    }

    #[test]
    fn error_flags_merge() {
        let flags = ErrorFlags::PACKAGE_NOT_FOUND.merge(ErrorFlags::PACKAGE_CONFLICT);
        assert!(!flags.is_ok());
        assert!(flags.contains(ErrorFlags::PACKAGE_NOT_FOUND));
        assert!(flags.contains(ErrorFlags::PACKAGE_CONFLICT));
        assert!(!flags.contains(ErrorFlags::PACKAGE_VER_MISMATCH));
    }

    #[test]
    fn error_to_flags_conversion() {
        let err = Error::PackageNotFound {
            name: "foo".to_string(),
        };
        let flags: ErrorFlags = (&err).into();
        assert!(flags.contains(ErrorFlags::PACKAGE_NOT_FOUND));
    }

    #[test]
    fn error_display() {
        let err = Error::PackageNotFound {
            name: "zlib".to_string(),
        };
        assert!(err.to_string().contains("zlib"));

        let err = Error::VersionMismatch {
            name: "glib".to_string(),
            found: "2.0".to_string(),
            required: "3.0".to_string(),
            comparator: ">=".to_string(),
        };
        assert!(err.to_string().contains("glib"));
        assert!(err.to_string().contains("2.0"));
        assert!(err.to_string().contains("3.0"));
    }

    #[test]
    fn multiple_errors_display() {
        let err = Error::Multiple(vec![
            Error::PackageNotFound {
                name: "a".to_string(),
            },
            Error::PackageNotFound {
                name: "b".to_string(),
            },
        ]);
        let msg = err.to_string();
        assert!(msg.contains("1."));
        assert!(msg.contains("2."));
        assert!(msg.contains("'a'"));
        assert!(msg.contains("'b'"));
    }
}
