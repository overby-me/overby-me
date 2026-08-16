//! `libpkgconf` — A pure Rust implementation of the pkg-config/pkgconf library.
//!
//! This crate provides the core library functionality for parsing `.pc` files,
//! resolving dependencies, managing compiler/linker flag fragments, and comparing
//! versions. It is designed as a drop-in replacement for the C `libpkgconf` library
//! from the [pkgconf project](https://github.com/pkgconf/pkgconf).
//!
//! # Architecture
//!
//! The library is organized into the following modules:
//!
//! - [`error`] — Error types and result aliases
//! - [`version`] — RPM-style version comparison and comparator operators
//! - [`parser`] — `.pc` file parsing, variable expansion, and argv splitting
//! - [`dependency`] — Dependency specification parsing and representation
//! - [`fragment`] — Compiler/linker flag fragment management, filtering, and deduplication
//! - [`client`] — Client state, search path management, and configuration
//! - [`pkg`] — Package representation, loading, prefix redefinition, and provides resolution
//! - [`cache`] — Package cache for avoiding redundant lookups and virtual package registration
//! - [`personality`] — Cross-compilation personality support (triplet-based configuration)
//! - [`queue`] — Package queue, dependency graph solver, and fragment collection
//! - [`audit`] — Audit logging for dependency resolution events
//!
//! # Example
//!
//! ```rust,no_run
//! use libpkgconf::parser::PcFile;
//! use libpkgconf::dependency::DependencyList;
//! use libpkgconf::fragment::FragmentList;
//! use libpkgconf::version;
//! use std::collections::HashMap;
//! use std::path::Path;
//!
//! // Parse a .pc file
//! let pc = PcFile::from_path(Path::new("/usr/lib/pkgconfig/zlib.pc")).unwrap();
//!
//! // Resolve variables
//! let global_vars = HashMap::new();
//! let resolved = libpkgconf::parser::resolve_variables(&pc, &global_vars, None).unwrap();
//!
//! // Get expanded Libs field
//! if let Some(libs_raw) = pc.get_field(libpkgconf::parser::Keyword::Libs) {
//!     let libs_expanded = libpkgconf::parser::resolve_field(libs_raw, &resolved).unwrap();
//!     let fragments = FragmentList::parse(&libs_expanded);
//!     println!("Libs: {}", fragments.render(' '));
//! }
//!
//! // Compare versions
//! assert!(version::compare("1.2.12", "1.2.11") > 0);
//! ```

pub mod audit;
pub mod cache;
pub mod client;
pub mod dependency;
pub mod error;
pub mod fragment;
pub mod parser;
pub mod path;
pub mod personality;
pub mod pkg;
pub mod queue;
pub mod version;

/// The version of this library (mirrors pkgconf compatibility version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The pkg-config protocol version we claim compatibility with.
///
/// This is the version reported by `--version` and used for
/// `--atleast-pkgconfig-version` checks. We target compatibility
/// with pkg-config 0.29.x / pkgconf 2.x.
pub const PKGCONFIG_COMPAT_VERSION: &str = "0.29.2";

/// Default maximum depth when traversing the dependency graph.
///
/// A value of 2000 is used by pkgconf to prevent runaway recursion
/// while still supporting very deep dependency trees.
pub const DEFAULT_MAX_TRAVERSAL_DEPTH: i32 = 2000;

/// Default system library directories that are filtered from `-L` output.
///
/// These are the directories that compilers search by default, so including
/// them in pkg-config output is unnecessary and can even cause problems.
#[cfg(unix)]
pub const DEFAULT_SYSTEM_LIBDIRS: &[&str] = &["/usr/lib", "/lib"];

/// Default system library directories on Windows.
///
/// On Windows, there are no conventional system library directories that
/// compilers search by default, so this list is empty. Users should rely
/// on `PKG_CONFIG_SYSTEM_LIBRARY_PATH` or toolchain-specific configuration.
#[cfg(windows)]
pub const DEFAULT_SYSTEM_LIBDIRS: &[&str] = &[];

/// Default system include directories that are filtered from `-I` output.
#[cfg(unix)]
pub const DEFAULT_SYSTEM_INCLUDEDIRS: &[&str] = &["/usr/include"];

/// Default system include directories on Windows.
///
/// On Windows, there are no conventional system include directories that
/// compilers search by default, so this list is empty.
#[cfg(windows)]
pub const DEFAULT_SYSTEM_INCLUDEDIRS: &[&str] = &[];

/// Default `.pc` file search path.
#[cfg(unix)]
pub const DEFAULT_PKGCONFIG_PATH: &[&str] = &[
    "/usr/local/lib/pkgconfig",
    "/usr/local/share/pkgconfig",
    "/usr/lib/pkgconfig",
    "/usr/share/pkgconfig",
];

/// Default `.pc` file search path on Windows.
///
/// On Windows there is no standard system-wide location for `.pc` files.
/// The search path is typically configured via `PKG_CONFIG_PATH` or
/// `PKG_CONFIG_LIBDIR` environment variables. The MSYS2/MinGW paths
/// are included as a convenience for the most common Windows pkg-config
/// usage scenario.
#[cfg(windows)]
pub const DEFAULT_PKGCONFIG_PATH: &[&str] = &[
    "C:/msys64/mingw64/lib/pkgconfig",
    "C:/msys64/mingw64/share/pkgconfig",
];

/// Default Homebrew prefix on Apple Silicon macOS.
#[cfg(target_os = "macos")]
pub const HOMEBREW_PREFIX_ARM64: &str = "/opt/homebrew";

/// Default Homebrew prefix on Intel macOS.
#[cfg(target_os = "macos")]
pub const HOMEBREW_PREFIX_X86_64: &str = "/usr/local";

/// Additional macOS-specific `.pc` file search paths.
///
/// These paths cover both Homebrew on Apple Silicon (`/opt/homebrew`) and
/// Intel (`/usr/local`), as well as the standard system locations. At
/// runtime, the active Homebrew prefix should be detected via
/// `brew --prefix` or the `HOMEBREW_PREFIX` environment variable, but
/// these paths serve as reasonable defaults.
#[cfg(target_os = "macos")]
pub const MACOS_EXTRA_PKGCONFIG_PATHS: &[&str] = &[
    "/opt/homebrew/lib/pkgconfig",
    "/opt/homebrew/share/pkgconfig",
];

/// The `PKG_CONFIG_PATH` environment variable name.
pub const ENV_PKG_CONFIG_PATH: &str = "PKG_CONFIG_PATH";

/// The `PKG_CONFIG_LIBDIR` environment variable name.
///
/// When set, this *replaces* the default search path instead of prepending to it.
pub const ENV_PKG_CONFIG_LIBDIR: &str = "PKG_CONFIG_LIBDIR";

/// The `PKG_CONFIG_SYSROOT_DIR` environment variable name.
pub const ENV_PKG_CONFIG_SYSROOT_DIR: &str = "PKG_CONFIG_SYSROOT_DIR";

/// The `PKG_CONFIG_TOP_BUILD_DIR` environment variable name.
pub const ENV_PKG_CONFIG_TOP_BUILD_DIR: &str = "PKG_CONFIG_TOP_BUILD_DIR";

/// The `PKG_CONFIG_ALLOW_SYSTEM_CFLAGS` environment variable name.
pub const ENV_PKG_CONFIG_ALLOW_SYSTEM_CFLAGS: &str = "PKG_CONFIG_ALLOW_SYSTEM_CFLAGS";

/// The `PKG_CONFIG_ALLOW_SYSTEM_LIBS` environment variable name.
pub const ENV_PKG_CONFIG_ALLOW_SYSTEM_LIBS: &str = "PKG_CONFIG_ALLOW_SYSTEM_LIBS";

/// The `PKG_CONFIG_DISABLE_UNINSTALLED` environment variable name.
pub const ENV_PKG_CONFIG_DISABLE_UNINSTALLED: &str = "PKG_CONFIG_DISABLE_UNINSTALLED";

/// The `PKG_CONFIG_DEBUG_SPEW` environment variable name.
pub const ENV_PKG_CONFIG_DEBUG_SPEW: &str = "PKG_CONFIG_DEBUG_SPEW";

/// The `PKG_CONFIG_MAXIMUM_TRAVERSE_DEPTH` environment variable name.
pub const ENV_PKG_CONFIG_MAXIMUM_TRAVERSE_DEPTH: &str = "PKG_CONFIG_MAXIMUM_TRAVERSE_DEPTH";

/// The `PKG_CONFIG_IGNORE_CONFLICTS` environment variable name.
pub const ENV_PKG_CONFIG_IGNORE_CONFLICTS: &str = "PKG_CONFIG_IGNORE_CONFLICTS";

/// The `PKG_CONFIG_PURE_DEPGRAPH` environment variable name.
pub const ENV_PKG_CONFIG_PURE_DEPGRAPH: &str = "PKG_CONFIG_PURE_DEPGRAPH";

/// The `PKG_CONFIG_LOG` environment variable name.
pub const ENV_PKG_CONFIG_LOG: &str = "PKG_CONFIG_LOG";

/// The `PKG_CONFIG_RELOCATE_PATHS` environment variable name.
pub const ENV_PKG_CONFIG_RELOCATE_PATHS: &str = "PKG_CONFIG_RELOCATE_PATHS";

/// The `PKG_CONFIG_DONT_DEFINE_PREFIX` environment variable name.
pub const ENV_PKG_CONFIG_DONT_DEFINE_PREFIX: &str = "PKG_CONFIG_DONT_DEFINE_PREFIX";

/// The `PKG_CONFIG_DONT_RELOCATE_PATHS` environment variable name.
pub const ENV_PKG_CONFIG_DONT_RELOCATE_PATHS: &str = "PKG_CONFIG_DONT_RELOCATE_PATHS";

/// The `PKG_CONFIG_MSVC_SYNTAX` environment variable name.
pub const ENV_PKG_CONFIG_MSVC_SYNTAX: &str = "PKG_CONFIG_MSVC_SYNTAX";

/// The `PKG_CONFIG_FDO_SYSROOT_RULES` environment variable name.
pub const ENV_PKG_CONFIG_FDO_SYSROOT_RULES: &str = "PKG_CONFIG_FDO_SYSROOT_RULES";

/// The `PKG_CONFIG_PRELOADED_FILES` environment variable name.
pub const ENV_PKG_CONFIG_PRELOADED_FILES: &str = "PKG_CONFIG_PRELOADED_FILES";
