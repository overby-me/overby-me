//! Client state management for pkg-config operations.
//!
//! The [`Client`] struct is the central coordination point for all pkg-config
//! operations. It manages:
//!
//! - Search paths for `.pc` files
//! - System directory filter lists (for `-I` and `-L` filtering)
//! - Global variable overrides (from `--define-variable`)
//! - Sysroot and buildroot directories
//! - Client flags controlling behaviour
//! - Prefix variable name (default: `prefix`)
//! - Maximum traversal depth
//!
//! # Example
//!
//! ```rust
//! use libpkgconf::client::Client;
//!
//! let client = Client::builder()
//!     .define_variable("prefix", "/opt/custom")
//!     .keep_system_cflags(true)
//!     .build();
//!
//! assert!(client.global_vars().contains_key("prefix"));
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::parser::PcFile;
use crate::path::SearchPath;

/// Client flags controlling pkg-config behaviour.
///
/// These flags correspond to pkgconf's `PKGCONF_PKG_PKGF_*` constants and
/// control various aspects of dependency resolution, output filtering,
/// and search behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClientFlags(u64);

impl ClientFlags {
    /// No flags set.
    pub const NONE: Self = Self(0);

    /// Search private dependencies (for `--static`).
    pub const SEARCH_PRIVATE: Self = Self(1 << 0);

    /// Merge private dependency fragments into the output.
    pub const MERGE_PRIVATE_FRAGMENTS: Self = Self(1 << 1);

    /// Skip conflicts checking.
    pub const SKIP_CONFLICTS: Self = Self(1 << 2);

    /// Do not filter system include dirs from cflags.
    pub const NO_FILTER_SYSTEM_INCLUDEDIRS: Self = Self(1 << 3);

    /// Do not filter system lib dirs from libs.
    pub const NO_FILTER_SYSTEM_LIBDIRS: Self = Self(1 << 4);

    /// Do not cache packages.
    pub const NO_CACHE: Self = Self(1 << 5);

    /// Do not use uninstalled packages.
    pub const NO_UNINSTALLED: Self = Self(1 << 6);

    /// Skip provides rules when resolving.
    pub const SKIP_PROVIDES: Self = Self(1 << 7);

    /// Skip root virtual package.
    pub const SKIP_ROOT_VIRTUAL: Self = Self(1 << 8);

    /// Skip errors (continue despite missing packages).
    pub const SKIP_ERRORS: Self = Self(1 << 9);

    /// Use MSVC-style output syntax.
    pub const MSVC_SYNTAX: Self = Self(1 << 10);

    /// Redefine prefix from `.pc` file location.
    pub const DEFINE_PREFIX: Self = Self(1 << 11);

    /// Never redefine prefix.
    pub const DONT_DEFINE_PREFIX: Self = Self(1 << 12);

    /// Do not relocate paths.
    pub const DONT_RELOCATE_PATHS: Self = Self(1 << 13);

    /// Use pure dependency graph (no private deps flattening).
    pub const PURE_DEPGRAPH: Self = Self(1 << 14);

    /// Use FDO sysroot rules.
    pub const FDO_SYSROOT_RULES: Self = Self(1 << 15);

    /// Only search paths from environment variables.
    pub const ENV_ONLY: Self = Self(1 << 16);

    /// Ignore conflicts rules in modules.
    pub const IGNORE_CONFLICTS: Self = Self(1 << 17);

    /// Check if no flags are set.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Set a flag.
    pub fn set(self, flag: Self) -> Self {
        Self(self.0 | flag.0)
    }

    /// Clear a flag.
    pub fn clear(self, flag: Self) -> Self {
        Self(self.0 & !flag.0)
    }

    /// Check whether a specific flag is set.
    pub fn contains(self, flag: Self) -> bool {
        (self.0 & flag.0) == flag.0
    }

    /// Get the raw bits.
    pub fn bits(self) -> u64 {
        self.0
    }

    /// Merge two flag sets (logical OR).
    pub fn merge(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// The central client for pkg-config operations.
///
/// `Client` holds all the state needed to discover, parse, and query
/// `.pc` files. It manages search paths, global variable overrides,
/// system directory filters, and configuration flags.
///
/// Use [`Client::new()`] for a default configuration, or
/// [`Client::builder()`] for fine-grained control.
#[derive(Debug, Clone)]
pub struct Client {
    /// Directories to search for `.pc` files, in priority order.
    dir_list: SearchPath,

    /// System library directories to filter from `-L` output.
    filter_libdirs: SearchPath,

    /// System include directories to filter from `-I` output.
    filter_includedirs: SearchPath,

    /// Global variable overrides (from `--define-variable`).
    global_vars: HashMap<String, String>,

    /// Sysroot directory (from `PKG_CONFIG_SYSROOT_DIR` or `--sysroot`).
    sysroot_dir: Option<String>,

    /// Build root directory (from `PKG_CONFIG_TOP_BUILD_DIR`).
    buildroot_dir: Option<String>,

    /// Configuration flags.
    flags: ClientFlags,

    /// The name of the variable considered to be the package prefix.
    /// Defaults to `"prefix"`.
    prefix_variable: String,

    /// Maximum depth for dependency graph traversal.
    max_traversal_depth: i32,

    /// Log file path for audit logging.
    log_file: Option<PathBuf>,

    /// Whether to output debug information.
    debug: bool,
}

impl Client {
    /// Create a new client with default configuration.
    ///
    /// The default client:
    /// - Builds search paths from environment variables and system defaults
    /// - Uses default system library and include directory filter lists
    /// - Has no global variable overrides
    /// - Uses `"prefix"` as the prefix variable name
    /// - Has a maximum traversal depth of 2000
    pub fn new() -> Self {
        let mut client = Self {
            dir_list: SearchPath::new(),
            filter_libdirs: SearchPath::from_paths(crate::DEFAULT_SYSTEM_LIBDIRS),
            filter_includedirs: SearchPath::from_paths(crate::DEFAULT_SYSTEM_INCLUDEDIRS),
            global_vars: HashMap::new(),
            sysroot_dir: None,
            buildroot_dir: None,
            flags: ClientFlags::NONE,
            prefix_variable: "prefix".to_string(),
            max_traversal_depth: crate::DEFAULT_MAX_TRAVERSAL_DEPTH,
            log_file: None,
            debug: false,
        };
        client.apply_env();
        client.build_dir_list();
        client
    }

    /// Create a builder for configuring a client.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::new()
    }

    // ── Getters ─────────────────────────────────────────────────────

    /// Get the search path list.
    pub fn dir_list(&self) -> &SearchPath {
        &self.dir_list
    }

    /// Get a mutable reference to the search path list.
    pub fn dir_list_mut(&mut self) -> &mut SearchPath {
        &mut self.dir_list
    }

    /// Get the system library directory filter list.
    pub fn filter_libdirs(&self) -> &SearchPath {
        &self.filter_libdirs
    }

    /// Get the system include directory filter list.
    pub fn filter_includedirs(&self) -> &SearchPath {
        &self.filter_includedirs
    }

    /// Get the global variable overrides.
    pub fn global_vars(&self) -> &HashMap<String, String> {
        &self.global_vars
    }

    /// Get a mutable reference to the global variable overrides.
    pub fn global_vars_mut(&mut self) -> &mut HashMap<String, String> {
        &mut self.global_vars
    }

    /// Get the sysroot directory.
    pub fn sysroot_dir(&self) -> Option<&str> {
        self.sysroot_dir.as_deref()
    }

    /// Get the build root directory.
    pub fn buildroot_dir(&self) -> Option<&str> {
        self.buildroot_dir.as_deref()
    }

    /// Get the client flags.
    pub fn flags(&self) -> ClientFlags {
        self.flags
    }

    /// Get the prefix variable name.
    pub fn prefix_variable(&self) -> &str {
        &self.prefix_variable
    }

    /// Get the maximum traversal depth.
    pub fn max_traversal_depth(&self) -> i32 {
        self.max_traversal_depth
    }

    /// Get the log file path.
    pub fn log_file(&self) -> Option<&Path> {
        self.log_file.as_deref()
    }

    /// Whether debug output is enabled.
    pub fn debug(&self) -> bool {
        self.debug
    }

    /// Whether to keep system cflags (not filter them).
    pub fn keep_system_cflags(&self) -> bool {
        self.flags
            .contains(ClientFlags::NO_FILTER_SYSTEM_INCLUDEDIRS)
    }

    /// Whether to keep system libs (not filter them).
    pub fn keep_system_libs(&self) -> bool {
        self.flags.contains(ClientFlags::NO_FILTER_SYSTEM_LIBDIRS)
    }

    /// Whether static mode is enabled.
    pub fn is_static(&self) -> bool {
        self.flags.contains(ClientFlags::SEARCH_PRIVATE)
    }

    // ── Setters ─────────────────────────────────────────────────────

    /// Set the client flags.
    pub fn set_flags(&mut self, flags: ClientFlags) {
        self.flags = flags;
    }

    /// Add a flag.
    pub fn add_flag(&mut self, flag: ClientFlags) {
        self.flags = self.flags.set(flag);
    }

    /// Remove a flag.
    pub fn remove_flag(&mut self, flag: ClientFlags) {
        self.flags = self.flags.clear(flag);
    }

    /// Set the sysroot directory.
    pub fn set_sysroot_dir(&mut self, sysroot: Option<String>) {
        self.sysroot_dir = sysroot;
    }

    /// Set the build root directory.
    pub fn set_buildroot_dir(&mut self, buildroot: Option<String>) {
        self.buildroot_dir = buildroot;
    }

    /// Set the prefix variable name.
    pub fn set_prefix_variable(&mut self, name: String) {
        self.prefix_variable = name;
    }

    /// Set the maximum traversal depth.
    pub fn set_max_traversal_depth(&mut self, depth: i32) {
        self.max_traversal_depth = depth;
    }

    /// Set the log file path.
    pub fn set_log_file(&mut self, path: Option<PathBuf>) {
        self.log_file = path;
    }

    /// Set debug mode.
    pub fn set_debug(&mut self, debug: bool) {
        self.debug = debug;
    }

    /// Define a global variable override.
    pub fn define_variable(&mut self, key: &str, value: &str) {
        self.global_vars.insert(key.to_string(), value.to_string());
    }

    /// Parse and define a variable from `key=value` format.
    ///
    /// Returns an error if the format is invalid.
    pub fn define_variable_from_str(&mut self, definition: &str) -> Result<()> {
        if let Some((key, value)) = definition.split_once('=') {
            self.global_vars.insert(key.to_string(), value.to_string());
            Ok(())
        } else {
            Err(Error::ParseError {
                path: PathBuf::from("<cmdline>"),
                line: 0,
                message: format!(
                    "Invalid --define-variable format: '{definition}' (expected varname=value)"
                ),
            })
        }
    }

    /// Add a directory to the search path.
    pub fn add_search_path<P: Into<PathBuf>>(&mut self, path: P) {
        self.dir_list.add(path);
    }

    /// Prepend a directory to the search path (highest priority).
    pub fn prepend_search_path<P: Into<PathBuf>>(&mut self, path: P) {
        self.dir_list.prepend(path);
    }

    // ── Package operations ──────────────────────────────────────────

    /// Find and load a `.pc` file by package name.
    ///
    /// Searches the client's search path list for `{name}.pc` and parses
    /// the first match. If `name` contains a `/` or ends with `.pc`, it
    /// is treated as a direct file path instead.
    pub fn find_package(&self, name: &str) -> Result<PcFile> {
        // If the name looks like a path, load directly
        if name.contains('/') || name.ends_with(".pc") {
            let path = Path::new(name);
            return PcFile::from_path(path);
        }

        // Search in dir_list
        if let Some(pc_path) = self.dir_list.find_pc_file(name) {
            return PcFile::from_path(&pc_path);
        }

        Err(Error::PackageNotFound {
            name: name.to_string(),
        })
    }

    /// Resolve all variables in a package using this client's global overrides and sysroot.
    pub fn resolve_variables(&self, pc: &PcFile) -> Result<HashMap<String, String>> {
        crate::parser::resolve_variables(pc, &self.global_vars, self.sysroot_dir.as_deref())
    }

    /// Resolve a field value using already-resolved variables.
    pub fn resolve_field(
        &self,
        field_value: &str,
        resolved_vars: &HashMap<String, String>,
    ) -> Result<String> {
        crate::parser::resolve_field(field_value, resolved_vars)
    }

    /// Get the system library directory paths as string slices.
    pub fn system_libdirs(&self) -> Vec<String> {
        self.filter_libdirs
            .dirs()
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }

    /// Get the system include directory paths as string slices.
    pub fn system_includedirs(&self) -> Vec<String> {
        self.filter_includedirs
            .dirs()
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }

    /// List all available packages.
    ///
    /// Returns a list of `(name, description, version)` tuples for every
    /// `.pc` file found in the search path.
    pub fn list_all(&self) -> Vec<(String, String, String)> {
        let pc_files = self.dir_list.list_all_pc_files();
        let mut result = Vec::with_capacity(pc_files.len());

        for (name, path) in pc_files {
            match PcFile::from_path(&path) {
                Ok(pc) => {
                    let description = pc.description().unwrap_or("").to_string();
                    let version = pc.version().unwrap_or("").to_string();
                    result.push((name, description, version));
                }
                Err(_) => {
                    // Skip unparsable .pc files
                    if self.debug {
                        eprintln!("warning: failed to parse {}", path.display());
                    }
                }
            }
        }

        result
    }

    /// List all available package names (without descriptions).
    pub fn list_package_names(&self) -> Vec<String> {
        self.dir_list
            .list_all_pc_files()
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    // ── Internal ────────────────────────────────────────────────────

    /// Apply environment variable overrides.
    fn apply_env(&mut self) {
        // PKG_CONFIG_SYSROOT_DIR
        if let Ok(sysroot) = std::env::var(crate::ENV_PKG_CONFIG_SYSROOT_DIR)
            && !sysroot.is_empty()
        {
            self.sysroot_dir = Some(sysroot);
        }

        // PKG_CONFIG_TOP_BUILD_DIR
        if let Ok(buildroot) = std::env::var(crate::ENV_PKG_CONFIG_TOP_BUILD_DIR)
            && !buildroot.is_empty()
        {
            self.buildroot_dir = Some(buildroot);
        }

        // PKG_CONFIG_ALLOW_SYSTEM_CFLAGS
        if std::env::var(crate::ENV_PKG_CONFIG_ALLOW_SYSTEM_CFLAGS).is_ok() {
            self.flags = self.flags.set(ClientFlags::NO_FILTER_SYSTEM_INCLUDEDIRS);
        }

        // PKG_CONFIG_ALLOW_SYSTEM_LIBS
        if std::env::var(crate::ENV_PKG_CONFIG_ALLOW_SYSTEM_LIBS).is_ok() {
            self.flags = self.flags.set(ClientFlags::NO_FILTER_SYSTEM_LIBDIRS);
        }

        // PKG_CONFIG_DISABLE_UNINSTALLED
        if std::env::var(crate::ENV_PKG_CONFIG_DISABLE_UNINSTALLED).is_ok() {
            self.flags = self.flags.set(ClientFlags::NO_UNINSTALLED);
        }

        // PKG_CONFIG_MAXIMUM_TRAVERSE_DEPTH
        if let Ok(depth_str) = std::env::var(crate::ENV_PKG_CONFIG_MAXIMUM_TRAVERSE_DEPTH)
            && let Ok(depth) = depth_str.parse::<i32>()
        {
            self.max_traversal_depth = depth;
        }

        // PKG_CONFIG_IGNORE_CONFLICTS
        if std::env::var(crate::ENV_PKG_CONFIG_IGNORE_CONFLICTS).is_ok() {
            self.flags = self.flags.set(ClientFlags::IGNORE_CONFLICTS);
        }

        // PKG_CONFIG_PURE_DEPGRAPH
        if std::env::var(crate::ENV_PKG_CONFIG_PURE_DEPGRAPH).is_ok() {
            self.flags = self.flags.set(ClientFlags::PURE_DEPGRAPH);
        }

        // PKG_CONFIG_LOG
        if let Ok(log_path) = std::env::var(crate::ENV_PKG_CONFIG_LOG)
            && !log_path.is_empty()
        {
            self.log_file = Some(PathBuf::from(log_path));
        }

        // PKG_CONFIG_DONT_DEFINE_PREFIX
        if std::env::var(crate::ENV_PKG_CONFIG_DONT_DEFINE_PREFIX).is_ok() {
            self.flags = self.flags.set(ClientFlags::DONT_DEFINE_PREFIX);
        }

        // PKG_CONFIG_DONT_RELOCATE_PATHS
        if std::env::var(crate::ENV_PKG_CONFIG_DONT_RELOCATE_PATHS).is_ok() {
            self.flags = self.flags.set(ClientFlags::DONT_RELOCATE_PATHS);
        }

        // PKG_CONFIG_MSVC_SYNTAX
        if std::env::var(crate::ENV_PKG_CONFIG_MSVC_SYNTAX).is_ok() {
            self.flags = self.flags.set(ClientFlags::MSVC_SYNTAX);
        }

        // PKG_CONFIG_FDO_SYSROOT_RULES
        if std::env::var(crate::ENV_PKG_CONFIG_FDO_SYSROOT_RULES).is_ok() {
            self.flags = self.flags.set(ClientFlags::FDO_SYSROOT_RULES);
        }

        // PKG_CONFIG_DEBUG_SPEW
        if std::env::var(crate::ENV_PKG_CONFIG_DEBUG_SPEW).is_ok() {
            self.debug = true;
        }
    }

    /// Build the search path list from environment and defaults.
    ///
    /// Resolution order:
    /// 1. Paths added via `--with-path` (already in dir_list from builder)
    /// 2. `PKG_CONFIG_PATH` (prepended to defaults)
    /// 3. `PKG_CONFIG_LIBDIR` (replaces defaults) or system defaults
    fn build_dir_list(&mut self) {
        // Collect paths from --with-path that were already added
        let with_paths: Vec<PathBuf> = self.dir_list.dirs().to_vec();
        self.dir_list.clear();

        // Start with --with-path entries (highest priority)
        for p in &with_paths {
            self.dir_list.add(p.clone());
        }

        // PKG_CONFIG_PATH is prepended to the default path
        if let Ok(pkg_config_path) = std::env::var(crate::ENV_PKG_CONFIG_PATH) {
            self.dir_list
                .add_delimited(&pkg_config_path, crate::path::PATH_SEPARATOR);
        }

        // PKG_CONFIG_LIBDIR replaces the default path; otherwise use defaults
        if let Ok(libdir) = std::env::var(crate::ENV_PKG_CONFIG_LIBDIR) {
            self.dir_list
                .add_delimited(&libdir, crate::path::PATH_SEPARATOR);
        } else if !self.flags.contains(ClientFlags::ENV_ONLY) {
            for p in crate::DEFAULT_PKGCONFIG_PATH {
                self.dir_list.add(*p);
            }
        }

        // Deduplicate
        self.dir_list.deduplicate();
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for constructing a [`Client`] with specific configuration.
///
/// # Example
///
/// ```rust
/// use libpkgconf::client::{Client, ClientFlags};
///
/// let client = Client::builder()
///     .define_variable("prefix", "/opt")
///     .keep_system_cflags(true)
///     .keep_system_libs(true)
///     .max_traversal_depth(500)
///     .flag(ClientFlags::NO_CACHE)
///     .build();
///
/// assert_eq!(client.max_traversal_depth(), 500);
/// assert!(client.keep_system_cflags());
/// assert!(client.keep_system_libs());
/// ```
#[derive(Debug, Clone, Default)]
pub struct ClientBuilder {
    with_paths: Vec<PathBuf>,
    filter_libdirs: Option<Vec<String>>,
    filter_includedirs: Option<Vec<String>>,
    global_vars: HashMap<String, String>,
    sysroot_dir: Option<String>,
    buildroot_dir: Option<String>,
    flags: ClientFlags,
    prefix_variable: Option<String>,
    max_traversal_depth: Option<i32>,
    log_file: Option<PathBuf>,
    debug: bool,
    skip_env: bool,
    skip_default_paths: bool,
}

impl ClientBuilder {
    /// Create a new builder with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a search path (like `--with-path`).
    pub fn with_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.with_paths.push(path.into());
        self
    }

    /// Add multiple search paths.
    pub fn with_paths<I, P>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        for p in paths {
            self.with_paths.push(p.into());
        }
        self
    }

    /// Set the system library directories to filter.
    pub fn filter_libdirs(mut self, dirs: Vec<String>) -> Self {
        self.filter_libdirs = Some(dirs);
        self
    }

    /// Set the system include directories to filter.
    pub fn filter_includedirs(mut self, dirs: Vec<String>) -> Self {
        self.filter_includedirs = Some(dirs);
        self
    }

    /// Define a global variable override.
    pub fn define_variable(mut self, key: &str, value: &str) -> Self {
        self.global_vars.insert(key.to_string(), value.to_string());
        self
    }

    /// Define multiple global variable overrides from `key=value` strings.
    ///
    /// Invalid entries (those without `=`) are silently skipped.
    pub fn define_variables_from_strs<I, S>(mut self, definitions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for def in definitions {
            if let Some((key, value)) = def.as_ref().split_once('=') {
                self.global_vars.insert(key.to_string(), value.to_string());
            }
        }
        self
    }

    /// Set the sysroot directory.
    pub fn sysroot_dir(mut self, sysroot: &str) -> Self {
        self.sysroot_dir = Some(sysroot.to_string());
        self
    }

    /// Set the build root directory.
    pub fn buildroot_dir(mut self, buildroot: &str) -> Self {
        self.buildroot_dir = Some(buildroot.to_string());
        self
    }

    /// Set a client flag.
    pub fn flag(mut self, flag: ClientFlags) -> Self {
        self.flags = self.flags.set(flag);
        self
    }

    /// Set all client flags at once.
    pub fn flags(mut self, flags: ClientFlags) -> Self {
        self.flags = flags;
        self
    }

    /// Set whether to keep system cflags in output.
    pub fn keep_system_cflags(mut self, keep: bool) -> Self {
        if keep {
            self.flags = self.flags.set(ClientFlags::NO_FILTER_SYSTEM_INCLUDEDIRS);
        } else {
            self.flags = self.flags.clear(ClientFlags::NO_FILTER_SYSTEM_INCLUDEDIRS);
        }
        self
    }

    /// Set whether to keep system libs in output.
    pub fn keep_system_libs(mut self, keep: bool) -> Self {
        if keep {
            self.flags = self.flags.set(ClientFlags::NO_FILTER_SYSTEM_LIBDIRS);
        } else {
            self.flags = self.flags.clear(ClientFlags::NO_FILTER_SYSTEM_LIBDIRS);
        }
        self
    }

    /// Enable static mode (`--static`).
    pub fn enable_static(mut self, enable: bool) -> Self {
        if enable {
            self.flags = self.flags.set(ClientFlags::SEARCH_PRIVATE);
            self.flags = self.flags.set(ClientFlags::MERGE_PRIVATE_FRAGMENTS);
        } else {
            self.flags = self.flags.clear(ClientFlags::SEARCH_PRIVATE);
            self.flags = self.flags.clear(ClientFlags::MERGE_PRIVATE_FRAGMENTS);
        }
        self
    }

    /// Enable pure dependency graph mode (`--pure`).
    pub fn pure(mut self, enable: bool) -> Self {
        if enable {
            self.flags = self.flags.set(ClientFlags::PURE_DEPGRAPH);
        } else {
            self.flags = self.flags.clear(ClientFlags::PURE_DEPGRAPH);
        }
        self
    }

    /// Set the prefix variable name.
    pub fn prefix_variable(mut self, name: &str) -> Self {
        self.prefix_variable = Some(name.to_string());
        self
    }

    /// Set the maximum traversal depth.
    pub fn max_traversal_depth(mut self, depth: i32) -> Self {
        self.max_traversal_depth = Some(depth);
        self
    }

    /// Set the log file path.
    pub fn log_file<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.log_file = Some(path.into());
        self
    }

    /// Enable or disable debug output.
    pub fn debug(mut self, debug: bool) -> Self {
        self.debug = debug;
        self
    }

    /// Skip reading environment variables during construction.
    ///
    /// Useful for testing.
    pub fn skip_env(mut self, skip: bool) -> Self {
        self.skip_env = skip;
        self
    }

    /// Skip adding default system search paths.
    ///
    /// When set, only explicitly-added paths (via `with_path`) and
    /// environment variable paths will be used.
    pub fn skip_default_paths(mut self, skip: bool) -> Self {
        self.skip_default_paths = skip;
        self
    }

    /// Build the client.
    pub fn build(self) -> Client {
        let filter_libdirs = if let Some(dirs) = self.filter_libdirs {
            SearchPath::from_paths(&dirs.iter().map(|s| s.as_str()).collect::<Vec<_>>())
        } else {
            SearchPath::from_paths(crate::DEFAULT_SYSTEM_LIBDIRS)
        };

        let filter_includedirs = if let Some(dirs) = self.filter_includedirs {
            SearchPath::from_paths(&dirs.iter().map(|s| s.as_str()).collect::<Vec<_>>())
        } else {
            SearchPath::from_paths(crate::DEFAULT_SYSTEM_INCLUDEDIRS)
        };

        let mut client = Client {
            dir_list: SearchPath::new(),
            filter_libdirs,
            filter_includedirs,
            global_vars: self.global_vars,
            sysroot_dir: self.sysroot_dir,
            buildroot_dir: self.buildroot_dir,
            flags: self.flags,
            prefix_variable: self.prefix_variable.unwrap_or_else(|| "prefix".to_string()),
            max_traversal_depth: self
                .max_traversal_depth
                .unwrap_or(crate::DEFAULT_MAX_TRAVERSAL_DEPTH),
            log_file: self.log_file,
            debug: self.debug,
        };

        // Add --with-path entries first
        for p in &self.with_paths {
            client.dir_list.add(p.clone());
        }

        // Apply environment variables (unless skipped)
        if !self.skip_env {
            client.apply_env();
        }

        // Build dir_list from env + defaults (unless skipped)
        if self.skip_default_paths {
            // Only keep the with_paths already added
            // and env paths if not skipped
            if !self.skip_env {
                if let Ok(pkg_config_path) = std::env::var(crate::ENV_PKG_CONFIG_PATH) {
                    client
                        .dir_list
                        .add_delimited(&pkg_config_path, crate::path::PATH_SEPARATOR);
                }
                if let Ok(libdir) = std::env::var(crate::ENV_PKG_CONFIG_LIBDIR) {
                    client
                        .dir_list
                        .add_delimited(&libdir, crate::path::PATH_SEPARATOR);
                }
            }
            client.dir_list.deduplicate();
        } else {
            client.build_dir_list();
        }

        client
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ClientFlags ─────────────────────────────────────────────────

    #[test]
    fn flags_default_is_none() {
        let flags = ClientFlags::default();
        assert!(flags.is_empty());
        assert_eq!(flags.bits(), 0);
    }

    #[test]
    fn flags_set_and_contains() {
        let flags = ClientFlags::NONE
            .set(ClientFlags::SEARCH_PRIVATE)
            .set(ClientFlags::NO_CACHE);
        assert!(flags.contains(ClientFlags::SEARCH_PRIVATE));
        assert!(flags.contains(ClientFlags::NO_CACHE));
        assert!(!flags.contains(ClientFlags::MSVC_SYNTAX));
    }

    #[test]
    fn flags_clear() {
        let flags = ClientFlags::NONE
            .set(ClientFlags::SEARCH_PRIVATE)
            .set(ClientFlags::NO_CACHE)
            .clear(ClientFlags::SEARCH_PRIVATE);
        assert!(!flags.contains(ClientFlags::SEARCH_PRIVATE));
        assert!(flags.contains(ClientFlags::NO_CACHE));
    }

    #[test]
    fn flags_merge() {
        let a = ClientFlags::NONE.set(ClientFlags::SEARCH_PRIVATE);
        let b = ClientFlags::NONE.set(ClientFlags::NO_CACHE);
        let merged = a.merge(b);
        assert!(merged.contains(ClientFlags::SEARCH_PRIVATE));
        assert!(merged.contains(ClientFlags::NO_CACHE));
    }

    // ── Client construction ─────────────────────────────────────────

    #[test]
    fn client_default() {
        let client = Client::builder().skip_env(true).build();
        assert_eq!(client.prefix_variable(), "prefix");
        assert_eq!(
            client.max_traversal_depth(),
            crate::DEFAULT_MAX_TRAVERSAL_DEPTH
        );
        assert!(!client.debug());
        assert!(client.sysroot_dir().is_none());
        assert!(client.buildroot_dir().is_none());
        assert!(client.log_file().is_none());
    }

    #[test]
    fn builder_define_variable() {
        let client = Client::builder()
            .skip_env(true)
            .define_variable("prefix", "/opt")
            .define_variable("libdir", "/opt/lib")
            .build();
        assert_eq!(client.global_vars().get("prefix").unwrap(), "/opt");
        assert_eq!(client.global_vars().get("libdir").unwrap(), "/opt/lib");
    }

    #[test]
    fn builder_define_variables_from_strs() {
        let client = Client::builder()
            .skip_env(true)
            .define_variables_from_strs(vec!["prefix=/opt", "libdir=/opt/lib", "invalid"])
            .build();
        assert_eq!(client.global_vars().get("prefix").unwrap(), "/opt");
        assert_eq!(client.global_vars().get("libdir").unwrap(), "/opt/lib");
        assert!(!client.global_vars().contains_key("invalid"));
    }

    #[test]
    fn builder_flags() {
        let client = Client::builder()
            .skip_env(true)
            .keep_system_cflags(true)
            .keep_system_libs(true)
            .flag(ClientFlags::NO_CACHE)
            .build();
        assert!(client.keep_system_cflags());
        assert!(client.keep_system_libs());
        assert!(client.flags().contains(ClientFlags::NO_CACHE));
    }

    #[test]
    fn builder_static_mode() {
        let client = Client::builder().skip_env(true).enable_static(true).build();
        assert!(client.is_static());
        assert!(client.flags().contains(ClientFlags::SEARCH_PRIVATE));
        assert!(
            client
                .flags()
                .contains(ClientFlags::MERGE_PRIVATE_FRAGMENTS)
        );
    }

    #[test]
    fn builder_sysroot_and_buildroot() {
        let client = Client::builder()
            .skip_env(true)
            .sysroot_dir("/cross")
            .buildroot_dir("/build")
            .build();
        assert_eq!(client.sysroot_dir(), Some("/cross"));
        assert_eq!(client.buildroot_dir(), Some("/build"));
    }

    #[test]
    fn builder_prefix_variable() {
        let client = Client::builder()
            .skip_env(true)
            .prefix_variable("my_prefix")
            .build();
        assert_eq!(client.prefix_variable(), "my_prefix");
    }

    #[test]
    fn builder_max_depth() {
        let client = Client::builder()
            .skip_env(true)
            .max_traversal_depth(100)
            .build();
        assert_eq!(client.max_traversal_depth(), 100);
    }

    #[test]
    fn builder_debug() {
        let client = Client::builder().skip_env(true).debug(true).build();
        assert!(client.debug());
    }

    #[test]
    fn builder_log_file() {
        let client = Client::builder()
            .skip_env(true)
            .log_file("/tmp/pkgconf.log")
            .build();
        assert_eq!(client.log_file(), Some(Path::new("/tmp/pkgconf.log")));
    }

    #[test]
    fn builder_with_paths() {
        let client = Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .with_path("/custom/lib/pkgconfig")
            .with_path("/custom/share/pkgconfig")
            .build();
        assert!(client.dir_list().contains("/custom/lib/pkgconfig"));
        assert!(client.dir_list().contains("/custom/share/pkgconfig"));
    }

    #[test]
    fn builder_skip_default_paths() {
        let client = Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .build();
        // Should have no default paths
        assert!(client.dir_list().is_empty());
    }

    // ── Client mutations ────────────────────────────────────────────

    #[test]
    fn client_define_variable() {
        let mut client = Client::builder().skip_env(true).build();
        client.define_variable("foo", "bar");
        assert_eq!(client.global_vars().get("foo").unwrap(), "bar");
    }

    #[test]
    fn client_define_variable_from_str() {
        let mut client = Client::builder().skip_env(true).build();
        assert!(client.define_variable_from_str("foo=bar").is_ok());
        assert_eq!(client.global_vars().get("foo").unwrap(), "bar");
        assert!(client.define_variable_from_str("invalid").is_err());
    }

    #[test]
    fn client_add_and_prepend_search_path() {
        let mut client = Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .build();
        client.add_search_path("/a");
        client.add_search_path("/b");
        client.prepend_search_path("/first");
        assert_eq!(client.dir_list().dirs()[0], PathBuf::from("/first"));
        assert_eq!(client.dir_list().dirs()[1], PathBuf::from("/a"));
        assert_eq!(client.dir_list().dirs()[2], PathBuf::from("/b"));
    }

    #[test]
    fn client_set_flags() {
        let mut client = Client::builder().skip_env(true).build();
        assert!(!client.flags().contains(ClientFlags::NO_CACHE));
        client.add_flag(ClientFlags::NO_CACHE);
        assert!(client.flags().contains(ClientFlags::NO_CACHE));
        client.remove_flag(ClientFlags::NO_CACHE);
        assert!(!client.flags().contains(ClientFlags::NO_CACHE));
    }

    #[test]
    fn client_set_sysroot() {
        let mut client = Client::builder().skip_env(true).build();
        assert!(client.sysroot_dir().is_none());
        client.set_sysroot_dir(Some("/sysroot".to_string()));
        assert_eq!(client.sysroot_dir(), Some("/sysroot"));
    }

    #[test]
    fn client_set_max_depth() {
        let mut client = Client::builder().skip_env(true).build();
        client.set_max_traversal_depth(42);
        assert_eq!(client.max_traversal_depth(), 42);
    }

    #[test]
    fn client_system_dirs() {
        let client = Client::builder().skip_env(true).build();
        let libdirs = client.system_libdirs();
        let includedirs = client.system_includedirs();
        // Should contain the defaults
        assert!(libdirs.iter().any(|d| d.contains("lib")));
        assert!(includedirs.iter().any(|d| d.contains("include")));
    }

    // ── Package discovery ───────────────────────────────────────────

    #[test]
    fn find_package_in_test_data() {
        let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/data");

        if test_dir.exists() {
            let client = Client::builder()
                .skip_env(true)
                .skip_default_paths(true)
                .with_path(test_dir)
                .build();

            let pc = client.find_package("zlib");
            assert!(pc.is_ok());
            let pc = pc.unwrap();
            assert_eq!(pc.name(), Some("zlib"));
            assert_eq!(pc.version(), Some("1.2.13"));
        }
    }

    #[test]
    fn find_package_not_found() {
        let client = Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .build();
        let result = client.find_package("nonexistent_package_xyz");
        assert!(result.is_err());
    }

    #[test]
    fn resolve_variables_with_client() {
        let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/data");

        if test_dir.exists() {
            let client = Client::builder()
                .skip_env(true)
                .skip_default_paths(true)
                .with_path(test_dir)
                .define_variable("prefix", "/override")
                .build();

            let pc = client.find_package("zlib").unwrap();
            let resolved = client.resolve_variables(&pc).unwrap();
            assert_eq!(resolved.get("prefix").unwrap(), "/override");
        }
    }

    #[test]
    fn list_all_packages() {
        let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/data");

        if test_dir.exists() {
            let client = Client::builder()
                .skip_env(true)
                .skip_default_paths(true)
                .with_path(test_dir)
                .build();

            let all = client.list_all();
            assert!(all.len() >= 3);
            let names: Vec<_> = all.iter().map(|(n, _, _)| n.as_str()).collect();
            assert!(names.contains(&"zlib"));
            assert!(names.contains(&"libfoo"));
            assert!(names.contains(&"libbar"));
        }
    }

    #[test]
    fn list_package_names() {
        let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/data");

        if test_dir.exists() {
            let client = Client::builder()
                .skip_env(true)
                .skip_default_paths(true)
                .with_path(test_dir)
                .build();

            let names = client.list_package_names();
            assert!(names.len() >= 3);
            assert!(names.contains(&"zlib".to_string()));
        }
    }

    // ── filter_libdirs / filter_includedirs override ────────────────

    #[test]
    fn builder_custom_filter_dirs() {
        let client = Client::builder()
            .skip_env(true)
            .filter_libdirs(vec!["/custom/lib".to_string()])
            .filter_includedirs(vec!["/custom/include".to_string()])
            .build();
        assert!(client.filter_libdirs().contains("/custom/lib"));
        assert!(!client.filter_libdirs().contains("/usr/lib"));
        assert!(client.filter_includedirs().contains("/custom/include"));
        assert!(!client.filter_includedirs().contains("/usr/include"));
    }

    // ── pure mode ───────────────────────────────────────────────────

    #[test]
    fn builder_pure_mode() {
        let client = Client::builder().skip_env(true).pure(true).build();
        assert!(client.flags().contains(ClientFlags::PURE_DEPGRAPH));
    }
}
