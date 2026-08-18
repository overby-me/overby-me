//! Package representation and loading.
//!
//! The [`Package`] struct is the fully-resolved representation of a `.pc` file.
//! It contains parsed and expanded fragment lists, dependency lists, resolved
//! variables, and metadata fields.
//!
//! # Loading
//!
//! Packages are typically loaded via [`Package::find()`], which searches the
//! client's search paths and returns a fully-resolved package. Alternatively,
//! [`Package::from_pc_file()`] can be used to convert a parsed [`PcFile`] into
//! a resolved `Package`.
//!
//! # Virtual Packages
//!
//! Packages can be created without a backing `.pc` file using
//! [`Package::new_virtual()`]. This is used for the built-in `pkg-config` and
//! `pkgconf` virtual packages, as well as for the synthetic "world" package
//! used during dependency resolution.
//!
//! # Uninstalled Packages
//!
//! When searching for a package `foo`, the resolver first looks for
//! `foo-uninstalled.pc` (unless disabled). If found, the package is flagged
//! as [`PackageFlags::UNINSTALLED`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::client::{Client, ClientFlags};
use crate::dependency::{Dependency, DependencyList};
use crate::error::{Error, Result};
use crate::fragment::FragmentList;
use crate::parser::{Keyword, PcFile};
use crate::version::Comparator;

/// Flags describing the state and origin of a package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PackageFlags(u32);

impl PackageFlags {
    /// No flags set.
    pub const NONE: Self = Self(0);

    /// This package was loaded with `--static`.
    pub const STATIC: Self = Self(1 << 0);

    /// This package is cached in the package cache.
    pub const CACHED: Self = Self(1 << 1);

    /// This package was loaded from an `-uninstalled.pc` file.
    pub const UNINSTALLED: Self = Self(1 << 2);

    /// This package is a virtual (synthetic) package with no `.pc` file.
    pub const VIRTUAL: Self = Self(1 << 3);

    /// This package has been visited during the current graph traversal.
    pub const VISITED: Self = Self(1 << 4);

    /// This package is an ancestor in the current traversal path (cycle detection).
    pub const ANCESTOR: Self = Self(1 << 5);

    /// This package's provides have been verified.
    pub const PROVIDES_VERIFIED: Self = Self(1 << 6);

    /// This package had its prefix redefined.
    pub const PREFIX_REDEFINED: Self = Self(1 << 7);

    /// Check if a specific flag is set.
    pub fn contains(self, flag: Self) -> bool {
        (self.0 & flag.0) == flag.0
    }

    /// Set a flag, returning the new flags value.
    pub fn set(self, flag: Self) -> Self {
        Self(self.0 | flag.0)
    }

    /// Unset a flag, returning the new flags value.
    pub fn clear(self, flag: Self) -> Self {
        Self(self.0 & !flag.0)
    }

    /// Get the raw bits.
    pub fn bits(self) -> u32 {
        self.0
    }

    /// Check if no flags are set.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Merge two flag sets.
    pub fn merge(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// A fully-resolved package, ready for dependency resolution and flag collection.
///
/// This struct mirrors pkgconf's `pkgconf_pkg_t` and contains all information
/// needed to participate in dependency graph resolution.
#[derive(Debug, Clone)]
pub struct Package {
    /// The lookup identifier (e.g. `"zlib"`, `"glib-2.0"`).
    pub id: String,

    /// The path to the `.pc` file this package was loaded from, if any.
    pub filename: Option<PathBuf>,

    /// The directory containing the `.pc` file.
    pub pc_filedir: Option<PathBuf>,

    /// The `Name` field from the `.pc` file (display name).
    pub realname: Option<String>,

    /// The `Version` field.
    pub version: String,

    /// The `Description` field.
    pub description: Option<String>,

    /// The `URL` field.
    pub url: Option<String>,

    /// The `License` field.
    pub license: Option<String>,

    /// The `Maintainer` field.
    pub maintainer: Option<String>,

    /// The `Copyright` field.
    pub copyright: Option<String>,

    /// The `Source` field.
    pub source: Option<String>,

    /// The `LicenseFile` field.
    pub license_file: Option<String>,

    /// Parsed `Libs` fragments.
    pub libs: FragmentList,

    /// Parsed `Libs.private` fragments.
    pub libs_private: FragmentList,

    /// Parsed `Cflags` fragments.
    pub cflags: FragmentList,

    /// Parsed `Cflags.private` fragments.
    pub cflags_private: FragmentList,

    /// Parsed `Requires` dependencies.
    pub requires: DependencyList,

    /// Parsed `Requires.private` dependencies.
    pub requires_private: DependencyList,

    /// Parsed `Conflicts` dependencies.
    pub conflicts: DependencyList,

    /// Parsed `Provides` dependencies.
    pub provides: DependencyList,

    /// Resolved variable map.
    pub vars: HashMap<String, String>,

    /// Package state flags.
    pub flags: PackageFlags,

    /// Serial number used for graph traversal (to avoid re-visiting).
    pub serial: u64,

    /// Traversal depth when this package was visited.
    pub depth: i32,
}

impl Package {
    /// Create a new virtual package with the given id and version.
    ///
    /// Virtual packages have no backing `.pc` file and are used for
    /// built-in packages (`pkg-config`, `pkgconf`) and the synthetic
    /// "world" package during dependency resolution.
    pub fn new_virtual(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            filename: None,
            pc_filedir: None,
            realname: None,
            version: version.into(),
            description: None,
            url: None,
            license: None,
            maintainer: None,
            copyright: None,
            source: None,
            license_file: None,
            libs: FragmentList::new(),
            libs_private: FragmentList::new(),
            cflags: FragmentList::new(),
            cflags_private: FragmentList::new(),
            requires: DependencyList::new(),
            requires_private: DependencyList::new(),
            conflicts: DependencyList::new(),
            provides: DependencyList::new(),
            vars: HashMap::new(),
            flags: PackageFlags::VIRTUAL,
            serial: 0,
            depth: 0,
        }
    }

    /// Load a package from a parsed `.pc` file.
    ///
    /// This resolves all variables (applying global overrides, sysroot, and
    /// optionally prefix redefinition), expands all field values, and parses
    /// dependency and fragment lists.
    ///
    /// # Arguments
    ///
    /// * `client` — The client providing global variable overrides, sysroot, etc.
    /// * `pc` — The parsed `.pc` file.
    /// * `id` — The lookup identifier for this package (e.g. `"zlib"`).
    pub fn from_pc_file(client: &Client, pc: &PcFile, id: impl Into<String>) -> Result<Self> {
        let id = id.into();

        // Check if this is an uninstalled package
        let is_uninstalled = pc
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .is_some_and(|stem| stem.to_string_lossy().ends_with("-uninstalled"));

        let mut flags = PackageFlags::NONE;
        if is_uninstalled {
            flags = flags.set(PackageFlags::UNINSTALLED);
        }

        // Resolve variables with optional prefix redefinition
        let mut resolved_vars = resolve_with_prefix(client, pc, &id)?;

        // If prefix was redefined, mark it
        if client.flags().contains(ClientFlags::DEFINE_PREFIX)
            && !client.flags().contains(ClientFlags::DONT_DEFINE_PREFIX)
            && let Some(ref _dir) = pc.pc_filedir
        {
            flags = flags.set(PackageFlags::PREFIX_REDEFINED);
        }

        // Apply sysroot to resolved paths in variables
        if let Some(ref sysroot) = client.sysroot_dir().map(|s| s.to_string()) {
            apply_sysroot_to_vars(&mut resolved_vars, sysroot);
        }

        // Helper closure to resolve a field value
        let resolve_field = |kw: Keyword| -> Result<Option<String>> {
            match pc.get_field(kw) {
                Some(raw) => {
                    let expanded = crate::parser::resolve_field(raw, &resolved_vars)?;
                    Ok(Some(expanded))
                }
                None => Ok(None),
            }
        };

        // Parse fragments from resolved field values
        let libs = resolve_field(Keyword::Libs)?
            .as_deref()
            .map(FragmentList::parse)
            .unwrap_or_default();

        let libs_private = resolve_field(Keyword::LibsPrivate)?
            .as_deref()
            .map(FragmentList::parse)
            .unwrap_or_default();

        let cflags = resolve_field(Keyword::Cflags)?
            .as_deref()
            .map(FragmentList::parse)
            .unwrap_or_default();

        let cflags_private = resolve_field(Keyword::CflagsPrivate)?
            .as_deref()
            .map(FragmentList::parse)
            .unwrap_or_default();

        // Parse dependency lists from resolved field values
        let requires = resolve_field(Keyword::Requires)?
            .as_deref()
            .map(DependencyList::parse)
            .unwrap_or_default();

        let requires_private = resolve_field(Keyword::RequiresPrivate)?
            .as_deref()
            .map(DependencyList::parse)
            .unwrap_or_default();

        let conflicts = resolve_field(Keyword::Conflicts)?
            .as_deref()
            .map(DependencyList::parse)
            .unwrap_or_default();

        let provides = resolve_field(Keyword::Provides)?
            .as_deref()
            .map(DependencyList::parse)
            .unwrap_or_default();

        // Extract metadata fields
        let realname = pc.name().map(|s| s.to_string());
        let version = pc.version().unwrap_or("").to_string();
        let description = pc.description().map(|s| s.to_string());
        let url = pc.url().map(|s| s.to_string());
        let license = pc.license().map(|s| s.to_string());
        let source = pc.source().map(|s| s.to_string());

        // Extra metadata fields (may not have getters on PcFile)
        let maintainer = pc.get_field(Keyword::Maintainer).map(|s| s.to_string());
        let copyright = pc.get_field(Keyword::Copyright).map(|s| s.to_string());
        let license_file = pc.get_field(Keyword::LicenseFile).map(|s| s.to_string());

        Ok(Self {
            id,
            filename: pc.path.clone(),
            pc_filedir: pc.pc_filedir.clone(),
            realname,
            version,
            description,
            url,
            license,
            maintainer,
            copyright,
            source,
            license_file,
            libs,
            libs_private,
            cflags,
            cflags_private,
            requires,
            requires_private,
            conflicts,
            provides,
            vars: resolved_vars,
            flags,
            serial: 0,
            depth: 0,
        })
    }

    /// Find and load a package by name from the client's search paths.
    ///
    /// This method implements the full package search algorithm:
    ///
    /// 1. If the name looks like a file path (contains `/` or ends with `.pc`),
    ///    load it directly.
    /// 2. Unless disabled, look for `{name}-uninstalled.pc` first.
    /// 3. Look for `{name}.pc` in the search paths.
    /// 4. Check if any loaded package's `Provides` satisfies this name.
    ///
    /// # Arguments
    ///
    /// * `client` — The client providing search paths and configuration.
    /// * `name` — The package name to search for.
    pub fn find(client: &Client, name: &str) -> Result<Self> {
        // If the name looks like a path, load directly
        if name.contains('/') || name.ends_with(".pc") {
            let path = Path::new(name);
            let pc = PcFile::from_path(path)?;
            let id = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| name.to_string());
            return Self::from_pc_file(client, &pc, id);
        }

        // Try uninstalled variant first (unless disabled)
        if !client.flags().contains(ClientFlags::NO_UNINSTALLED) {
            let uninstalled_name = format!("{name}-uninstalled");
            if let Some(pc_path) = client.dir_list().find_pc_file(&uninstalled_name) {
                let pc = PcFile::from_path(&pc_path)?;
                return Self::from_pc_file(client, &pc, name);
            }
        }

        // Search in the client's search path
        if let Some(pc_path) = client.dir_list().find_pc_file(name) {
            let pc = PcFile::from_path(&pc_path)?;
            return Self::from_pc_file(client, &pc, name);
        }

        Err(Error::PackageNotFound {
            name: name.to_string(),
        })
    }

    /// Check whether this package satisfies a given dependency.
    ///
    /// A package satisfies a dependency if:
    /// 1. The package's `id` matches the dependency's package name, OR
    /// 2. The package's `Provides` list includes an entry that matches.
    ///
    /// In either case, the version constraint must also be satisfied.
    pub fn verify_dependency(&self, dep: &Dependency) -> bool {
        // Check direct match by id
        if self.id == dep.package {
            return dep.version_satisfied_by(&self.version);
        }

        // Check provides
        if !self.flags.contains(PackageFlags::VIRTUAL) || !self.provides.is_empty() {
            for provided in self.provides.iter() {
                if provided.package == dep.package {
                    let provided_version = provided.version.as_deref().unwrap_or(&self.version);
                    return dep.version_satisfied_by(provided_version);
                }
            }
        }

        false
    }

    /// Check whether this package satisfies a dependency by name only
    /// (ignoring version constraints). Used for provides lookup.
    pub fn satisfies_name(&self, name: &str) -> bool {
        if self.id == name {
            return true;
        }
        self.provides
            .iter()
            .any(|provided| provided.package == name)
    }

    /// Check if this package is virtual (has no backing `.pc` file).
    pub fn is_virtual(&self) -> bool {
        self.flags.contains(PackageFlags::VIRTUAL)
    }

    /// Check if this package was loaded from an uninstalled `.pc` file.
    pub fn is_uninstalled(&self) -> bool {
        self.flags.contains(PackageFlags::UNINSTALLED)
    }

    /// Check if this package has been visited in the current traversal.
    pub fn is_visited(&self) -> bool {
        self.flags.contains(PackageFlags::VISITED)
    }

    /// Get the display name for this package.
    ///
    /// Returns the `Name` field if available, otherwise the lookup id.
    pub fn display_name(&self) -> &str {
        self.realname.as_deref().unwrap_or(&self.id)
    }

    /// Get a variable value from this package's resolved variable map.
    pub fn get_variable(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(|s| s.as_str())
    }

    /// Get all variable names defined in this package.
    pub fn variable_names(&self) -> Vec<&str> {
        self.vars.keys().map(|s| s.as_str()).collect()
    }

    /// Create a self-referencing provides entry.
    ///
    /// This adds an entry to the provides list for this package's own id
    /// and version, which is the implicit "a package provides itself" rule.
    pub fn add_self_provides(&mut self) {
        // Only add if not already present
        if !self.provides.iter().any(|p| p.package == self.id) {
            let self_dep = Dependency::with_version(&self.id, Comparator::Equal, &self.version);
            // Insert at the beginning so explicit provides take precedence
            let mut new_provides = DependencyList::new();
            new_provides.push(self_dep);
            new_provides.append(&self.provides);
            self.provides = new_provides;
        }
    }

    /// Check this package for conflicts against a dependency list.
    ///
    /// Returns the first conflicting dependency, if any.
    pub fn check_conflicts<'a>(&self, against: &'a DependencyList) -> Option<&'a Dependency> {
        against.iter().find(|conflict| {
            (self.id == conflict.package || self.satisfies_name(&conflict.package))
                && conflict.version_satisfied_by(&self.version)
        })
    }

    /// Collect all cflags fragments from this package.
    ///
    /// If `include_private` is true, also includes `Cflags.private`.
    pub fn collect_cflags(&self, include_private: bool) -> FragmentList {
        let mut result = self.cflags.clone();
        if include_private {
            result.append(&self.cflags_private);
        }
        result
    }

    /// Collect all libs fragments from this package.
    ///
    /// If `include_private` is true, also includes `Libs.private`.
    pub fn collect_libs(&self, include_private: bool) -> FragmentList {
        let mut result = self.libs.clone();
        if include_private {
            result.append(&self.libs_private);
        }
        result
    }

    /// Scan all `.pc` files in the client's search paths and load them.
    ///
    /// Returns a list of successfully loaded packages. Packages that fail
    /// to parse are silently skipped (a warning is printed if debug is enabled).
    pub fn scan_all(client: &Client) -> Vec<Self> {
        let pc_files = client.dir_list().list_all_pc_files();
        let mut packages = Vec::with_capacity(pc_files.len());

        for (name, path) in pc_files {
            match PcFile::from_path(&path) {
                Ok(pc) => match Self::from_pc_file(client, &pc, &name) {
                    Ok(pkg) => packages.push(pkg),
                    Err(_) => {
                        if client.debug() {
                            eprintln!(
                                "warning: failed to resolve package '{}' from {}",
                                name,
                                path.display()
                            );
                        }
                    }
                },
                Err(_) => {
                    if client.debug() {
                        eprintln!("warning: failed to parse {}", path.display());
                    }
                }
            }
        }

        packages
    }
}

impl std::fmt::Display for Package {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.id)?;
        if !self.version.is_empty() {
            write!(f, " {}", self.version)?;
        }
        Ok(())
    }
}

// ── Helper functions ────────────────────────────────────────────────────

/// Resolve variables with optional prefix redefinition.
///
/// If the client has `DEFINE_PREFIX` set (and not `DONT_DEFINE_PREFIX`),
/// the prefix variable is computed from the `.pc` file's location.
fn resolve_with_prefix(client: &Client, pc: &PcFile, _id: &str) -> Result<HashMap<String, String>> {
    let mut global_vars = client.global_vars().clone();

    // Prefix redefinition: compute prefix from the .pc file location
    if client.flags().contains(ClientFlags::DEFINE_PREFIX)
        && !client.flags().contains(ClientFlags::DONT_DEFINE_PREFIX)
        && let Some(ref pc_filedir) = pc.pc_filedir
    {
        let prefix_var = client.prefix_variable();
        // Only redefine if not already overridden by --define-variable
        if !global_vars.contains_key(prefix_var)
            && let Some(computed_prefix) = compute_prefix_from_pc_dir(pc, pc_filedir)
        {
            global_vars.insert(prefix_var.to_string(), computed_prefix);
        }
    }

    crate::parser::resolve_variables(pc, &global_vars, client.sysroot_dir())
}

/// Compute the effective prefix from the `.pc` file directory.
///
/// The algorithm works by examining where the `.pc` file is located relative
/// to the original prefix variable. For example, if:
/// - `prefix=/usr/local`
/// - The `.pc` file is at `/opt/myapp/lib/pkgconfig/foo.pc`
///
/// Then the `.pc` file dir is `/opt/myapp/lib/pkgconfig`. We check the
/// original prefix value's expected `.pc` location pattern and strip back
/// from the `.pc` file dir to find the new prefix.
///
/// Specifically, pkgconf's algorithm:
/// 1. Get the original `prefix` variable value from the `.pc` file.
/// 2. Get the `pcfiledir` (directory containing the `.pc` file).
/// 3. Walk up from `pcfiledir`, stripping path components until we find
///    a reasonable prefix (typically going up 2 levels from `lib/pkgconfig`
///    or `share/pkgconfig`).
fn compute_prefix_from_pc_dir(pc: &PcFile, pc_filedir: &Path) -> Option<String> {
    // Get the original prefix value from the .pc file
    let _original_prefix = pc.get_variable_raw("prefix")?;

    // The standard layout is: {prefix}/lib/pkgconfig/{name}.pc
    // or: {prefix}/share/pkgconfig/{name}.pc
    // So we go up 2 directories from the .pc file's directory to get the prefix.
    //
    // But we also need to handle deeper nesting like:
    //   {prefix}/lib/{arch}/pkgconfig/{name}.pc  (3 levels up)
    //
    // pkgconf checks if the parent of pcfiledir is "pkgconfig" and then
    // goes up from there.

    let parent = pc_filedir.parent()?;
    let parent_name = pc_filedir.file_name()?.to_str()?;

    if parent_name == "pkgconfig" {
        // We're in a `pkgconfig` directory, go up one more level
        // to get past `lib` or `share`
        let grandparent = parent.parent()?;
        let grandparent_name = parent.file_name()?.to_str()?;

        if grandparent_name == "lib"
            || grandparent_name == "lib64"
            || grandparent_name == "share"
            || grandparent_name == "libdata"
        {
            // Standard case: prefix/lib/pkgconfig/ → prefix is grandparent
            Some(grandparent.to_string_lossy().into_owned())
        } else {
            // Possibly prefix/lib/{arch}/pkgconfig/ → need to go up one more
            if let Some(great_grandparent) = grandparent.parent() {
                let gg_name = grandparent.file_name()?.to_str()?;
                if gg_name == "lib" || gg_name == "lib64" {
                    // prefix/lib/{arch}/pkgconfig/ → prefix is great_grandparent
                    Some(great_grandparent.to_string_lossy().into_owned())
                } else {
                    // Fall back: just use grandparent
                    Some(grandparent.to_string_lossy().into_owned())
                }
            } else {
                Some(grandparent.to_string_lossy().into_owned())
            }
        }
    } else {
        // Not in a pkgconfig directory — just use the parent
        Some(parent.to_string_lossy().into_owned())
    }
}

/// Apply sysroot prefix to path-like variable values.
///
/// Variables whose values look like absolute paths get the sysroot
/// prepended, unless they already start with the sysroot.
fn apply_sysroot_to_vars(vars: &mut HashMap<String, String>, sysroot: &str) {
    if sysroot.is_empty() {
        return;
    }
    // Only apply to variables with absolute path values
    let keys: Vec<String> = vars.keys().cloned().collect();
    for key in keys {
        // Skip the sysroot-related variables themselves
        if key == "pc_sysrootdir" || key == "pcfiledir" {
            continue;
        }
        let val = &vars[&key];
        if val.starts_with('/') && !val.starts_with(sysroot) {
            let new_val = format!("{sysroot}{val}");
            vars.insert(key, new_val);
        }
    }
}

/// Create the built-in `pkg-config` virtual package.
///
/// This package provides metadata about the pkg-config implementation itself,
/// including search paths and system directory information.
pub fn builtin_pkg_config(client: &Client) -> Package {
    let mut pkg = Package::new_virtual("pkg-config", crate::PKGCONFIG_COMPAT_VERSION);
    pkg.realname = Some("pkg-config".to_string());
    pkg.description = Some("Package metadata tool (compatibility shim)".to_string());
    pkg.url = Some("https://tangled.org/overby.me/overby.me/tree/main/rust/pkg-config".to_string());

    // Set well-known variables
    pkg.vars
        .insert("pc_path".to_string(), client.dir_list().to_delimited(':'));
    pkg.vars.insert(
        "pc_system_libdirs".to_string(),
        client.filter_libdirs().to_delimited(':'),
    );
    pkg.vars.insert(
        "pc_system_includedirs".to_string(),
        client.filter_includedirs().to_delimited(':'),
    );

    // Self-provides
    pkg.add_self_provides();

    pkg
}

/// Create the built-in `pkgconf` virtual package.
///
/// This is an alias for the `pkg-config` virtual package, using the
/// library version as the pkgconf version.
pub fn builtin_pkgconf(client: &Client) -> Package {
    let mut pkg = Package::new_virtual("pkgconf", crate::VERSION);
    pkg.realname = Some("pkgconf".to_string());
    pkg.description = Some("Package metadata toolkit (Rust implementation)".to_string());
    pkg.url = Some("https://tangled.org/overby.me/overby.me/tree/main/rust/pkg-config".to_string());

    // Same variables as pkg-config
    pkg.vars
        .insert("pc_path".to_string(), client.dir_list().to_delimited(':'));
    pkg.vars.insert(
        "pc_system_libdirs".to_string(),
        client.filter_libdirs().to_delimited(':'),
    );
    pkg.vars.insert(
        "pc_system_includedirs".to_string(),
        client.filter_includedirs().to_delimited(':'),
    );

    // Self-provides
    pkg.add_self_provides();

    // Also provide pkg-config compatibility
    pkg.provides.push(Dependency::with_version(
        "pkg-config",
        Comparator::Equal,
        crate::PKGCONFIG_COMPAT_VERSION,
    ));

    pkg
}

/// Create a "world" virtual package used as the root of the dependency graph.
///
/// The world package has no version or metadata; it exists solely to hold
/// the top-level dependency list during resolution.
pub fn world_package() -> Package {
    let mut pkg = Package::new_virtual("virtual:world", "0");
    pkg.description = Some("the virtual world package".to_string());
    pkg
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── PackageFlags ────────────────────────────────────────────────

    #[test]
    fn package_flags_default_is_none() {
        let flags = PackageFlags::default();
        assert!(flags.is_empty());
        assert_eq!(flags.bits(), 0);
    }

    #[test]
    fn package_flags_set_and_contains() {
        let flags = PackageFlags::NONE
            .set(PackageFlags::VIRTUAL)
            .set(PackageFlags::CACHED);
        assert!(flags.contains(PackageFlags::VIRTUAL));
        assert!(flags.contains(PackageFlags::CACHED));
        assert!(!flags.contains(PackageFlags::UNINSTALLED));
    }

    #[test]
    fn package_flags_clear() {
        let flags = PackageFlags::NONE
            .set(PackageFlags::VIRTUAL)
            .set(PackageFlags::CACHED);
        let flags = flags.clear(PackageFlags::CACHED);
        assert!(flags.contains(PackageFlags::VIRTUAL));
        assert!(!flags.contains(PackageFlags::CACHED));
    }

    #[test]
    fn package_flags_merge() {
        let a = PackageFlags::VIRTUAL;
        let b = PackageFlags::UNINSTALLED;
        let merged = a.merge(b);
        assert!(merged.contains(PackageFlags::VIRTUAL));
        assert!(merged.contains(PackageFlags::UNINSTALLED));
    }

    // ── Virtual packages ────────────────────────────────────────────

    #[test]
    fn virtual_package_creation() {
        let pkg = Package::new_virtual("test-pkg", "1.0.0");
        assert_eq!(pkg.id, "test-pkg");
        assert_eq!(pkg.version, "1.0.0");
        assert!(pkg.is_virtual());
        assert!(!pkg.is_uninstalled());
        assert!(pkg.filename.is_none());
        assert!(pkg.libs.is_empty());
        assert!(pkg.requires.is_empty());
    }

    #[test]
    fn virtual_package_display() {
        let pkg = Package::new_virtual("zlib", "1.2.13");
        assert_eq!(format!("{pkg}"), "zlib 1.2.13");
    }

    #[test]
    fn virtual_package_display_no_version() {
        let mut pkg = Package::new_virtual("foo", "");
        pkg.version = String::new();
        assert_eq!(format!("{pkg}"), "foo");
    }

    #[test]
    fn virtual_package_display_name() {
        let mut pkg = Package::new_virtual("foo", "1.0");
        assert_eq!(pkg.display_name(), "foo");
        pkg.realname = Some("Foo Library".to_string());
        assert_eq!(pkg.display_name(), "Foo Library");
    }

    // ── Dependency verification ────────────────────────────────────

    #[test]
    fn verify_dependency_by_id() {
        let pkg = Package::new_virtual("zlib", "1.2.13");
        let dep = Dependency::new("zlib");
        assert!(pkg.verify_dependency(&dep));
    }

    #[test]
    fn verify_dependency_version_match() {
        let pkg = Package::new_virtual("zlib", "1.2.13");
        let dep = Dependency::with_version("zlib", Comparator::GreaterThanEqual, "1.2.0");
        assert!(pkg.verify_dependency(&dep));
    }

    #[test]
    fn verify_dependency_version_mismatch() {
        let pkg = Package::new_virtual("zlib", "1.2.13");
        let dep = Dependency::with_version("zlib", Comparator::GreaterThanEqual, "2.0");
        assert!(!pkg.verify_dependency(&dep));
    }

    #[test]
    fn verify_dependency_by_provides() {
        let mut pkg = Package::new_virtual("libfoo", "1.2.3");
        pkg.provides.push(Dependency::with_version(
            "libfoo-compat",
            Comparator::Equal,
            "1.2.3",
        ));
        let dep = Dependency::new("libfoo-compat");
        assert!(pkg.verify_dependency(&dep));
    }

    #[test]
    fn verify_dependency_provides_version_check() {
        let mut pkg = Package::new_virtual("libfoo", "1.2.3");
        pkg.provides.push(Dependency::with_version(
            "libfoo-compat",
            Comparator::Equal,
            "1.0",
        ));
        // The provided version is "1.0", and the dep requires >= 2.0
        let dep = Dependency::with_version("libfoo-compat", Comparator::GreaterThanEqual, "2.0");
        assert!(!pkg.verify_dependency(&dep));
    }

    #[test]
    fn verify_dependency_wrong_name() {
        let pkg = Package::new_virtual("zlib", "1.2.13");
        let dep = Dependency::new("openssl");
        assert!(!pkg.verify_dependency(&dep));
    }

    // ── Self provides ──────────────────────────────────────────────

    #[test]
    fn add_self_provides() {
        let mut pkg = Package::new_virtual("zlib", "1.2.13");
        assert!(pkg.provides.is_empty());
        pkg.add_self_provides();
        assert_eq!(pkg.provides.len(), 1);
        assert_eq!(pkg.provides.entries()[0].package, "zlib");
        assert_eq!(pkg.provides.entries()[0].version.as_deref(), Some("1.2.13"));
    }

    #[test]
    fn add_self_provides_idempotent() {
        let mut pkg = Package::new_virtual("zlib", "1.2.13");
        pkg.add_self_provides();
        pkg.add_self_provides();
        // Should only have one self-provides entry
        let count = pkg.provides.iter().filter(|p| p.package == "zlib").count();
        assert_eq!(count, 1);
    }

    // ── satisfies_name ─────────────────────────────────────────────

    #[test]
    fn satisfies_name_by_id() {
        let pkg = Package::new_virtual("zlib", "1.2.13");
        assert!(pkg.satisfies_name("zlib"));
        assert!(!pkg.satisfies_name("openssl"));
    }

    #[test]
    fn satisfies_name_by_provides() {
        let mut pkg = Package::new_virtual("libfoo", "1.0");
        pkg.provides.push(Dependency::new("libfoo-compat"));
        assert!(pkg.satisfies_name("libfoo-compat"));
    }

    // ── Conflicts ──────────────────────────────────────────────────

    #[test]
    fn check_conflicts_none() {
        let pkg = Package::new_virtual("zlib", "1.2.13");
        let conflicts = DependencyList::parse("openssl < 1.0");
        assert!(pkg.check_conflicts(&conflicts).is_none());
    }

    #[test]
    fn check_conflicts_detected() {
        let pkg = Package::new_virtual("zlib", "0.9");
        let conflicts = DependencyList::parse("zlib < 1.0");
        assert!(pkg.check_conflicts(&conflicts).is_some());
    }

    #[test]
    fn check_conflicts_version_not_in_range() {
        let pkg = Package::new_virtual("zlib", "1.2.13");
        let conflicts = DependencyList::parse("zlib < 1.0");
        assert!(pkg.check_conflicts(&conflicts).is_none());
    }

    // ── Fragment collection ────────────────────────────────────────

    #[test]
    fn collect_cflags_without_private() {
        let mut pkg = Package::new_virtual("test", "1.0");
        pkg.cflags = FragmentList::parse("-I/usr/include/test");
        pkg.cflags_private = FragmentList::parse("-DTEST_INTERNAL");
        let result = pkg.collect_cflags(false);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn collect_cflags_with_private() {
        let mut pkg = Package::new_virtual("test", "1.0");
        pkg.cflags = FragmentList::parse("-I/usr/include/test");
        pkg.cflags_private = FragmentList::parse("-DTEST_INTERNAL");
        let result = pkg.collect_cflags(true);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn collect_libs_without_private() {
        let mut pkg = Package::new_virtual("test", "1.0");
        pkg.libs = FragmentList::parse("-L/usr/lib -ltest");
        pkg.libs_private = FragmentList::parse("-lm -lpthread");
        let result = pkg.collect_libs(false);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn collect_libs_with_private() {
        let mut pkg = Package::new_virtual("test", "1.0");
        pkg.libs = FragmentList::parse("-L/usr/lib -ltest");
        pkg.libs_private = FragmentList::parse("-lm -lpthread");
        let result = pkg.collect_libs(true);
        assert_eq!(result.len(), 4);
    }

    // ── get_variable ───────────────────────────────────────────────

    #[test]
    fn get_variable_existing() {
        let mut pkg = Package::new_virtual("test", "1.0");
        pkg.vars.insert("prefix".to_string(), "/usr".to_string());
        assert_eq!(pkg.get_variable("prefix"), Some("/usr"));
    }

    #[test]
    fn get_variable_missing() {
        let pkg = Package::new_virtual("test", "1.0");
        assert_eq!(pkg.get_variable("nonexistent"), None);
    }

    #[test]
    fn variable_names() {
        let mut pkg = Package::new_virtual("test", "1.0");
        pkg.vars.insert("prefix".to_string(), "/usr".to_string());
        pkg.vars
            .insert("libdir".to_string(), "/usr/lib".to_string());
        let names = pkg.variable_names();
        assert!(names.contains(&"prefix"));
        assert!(names.contains(&"libdir"));
    }

    // ── Built-in packages ──────────────────────────────────────────

    #[test]
    fn builtin_pkg_config_package() {
        let client = Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .build();
        let pkg = builtin_pkg_config(&client);
        assert_eq!(pkg.id, "pkg-config");
        assert_eq!(pkg.version, crate::PKGCONFIG_COMPAT_VERSION);
        assert!(pkg.is_virtual());
        assert!(pkg.vars.contains_key("pc_path"));
        assert!(pkg.vars.contains_key("pc_system_libdirs"));
        assert!(pkg.vars.contains_key("pc_system_includedirs"));
        // Should have self-provides
        assert!(pkg.provides.iter().any(|p| p.package == "pkg-config"));
    }

    #[test]
    fn builtin_pkgconf_package() {
        let client = Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .build();
        let pkg = builtin_pkgconf(&client);
        assert_eq!(pkg.id, "pkgconf");
        assert_eq!(pkg.version, crate::VERSION);
        assert!(pkg.is_virtual());
        // Should provide both pkgconf and pkg-config
        assert!(pkg.provides.iter().any(|p| p.package == "pkgconf"));
        assert!(pkg.provides.iter().any(|p| p.package == "pkg-config"));
    }

    #[test]
    fn world_package_creation() {
        let pkg = world_package();
        assert_eq!(pkg.id, "virtual:world");
        assert!(pkg.is_virtual());
        assert!(pkg.requires.is_empty());
    }

    // ── apply_sysroot_to_vars ──────────────────────────────────────

    #[test]
    fn sysroot_applied_to_absolute_paths() {
        let mut vars = HashMap::new();
        vars.insert("prefix".to_string(), "/usr".to_string());
        vars.insert("libdir".to_string(), "/usr/lib".to_string());
        vars.insert("name".to_string(), "foo".to_string());

        apply_sysroot_to_vars(&mut vars, "/sysroot");

        assert_eq!(vars["prefix"], "/sysroot/usr");
        assert_eq!(vars["libdir"], "/sysroot/usr/lib");
        assert_eq!(vars["name"], "foo"); // not a path, untouched
    }

    #[test]
    fn sysroot_not_double_applied() {
        let mut vars = HashMap::new();
        vars.insert("prefix".to_string(), "/sysroot/usr".to_string());

        apply_sysroot_to_vars(&mut vars, "/sysroot");

        // Should not become /sysroot/sysroot/usr
        assert_eq!(vars["prefix"], "/sysroot/usr");
    }

    #[test]
    fn sysroot_skips_special_vars() {
        let mut vars = HashMap::new();
        vars.insert("pcfiledir".to_string(), "/usr/lib/pkgconfig".to_string());
        vars.insert("pc_sysrootdir".to_string(), "/other/sysroot".to_string());

        apply_sysroot_to_vars(&mut vars, "/sysroot");

        // These should be untouched
        assert_eq!(vars["pcfiledir"], "/usr/lib/pkgconfig");
        assert_eq!(vars["pc_sysrootdir"], "/other/sysroot");
    }

    #[test]
    fn sysroot_empty_is_noop() {
        let mut vars = HashMap::new();
        vars.insert("prefix".to_string(), "/usr".to_string());

        apply_sysroot_to_vars(&mut vars, "");

        assert_eq!(vars["prefix"], "/usr");
    }

    // ── compute_prefix_from_pc_dir ─────────────────────────────────

    #[test]
    fn compute_prefix_standard_lib_pkgconfig() {
        let pc = make_test_pc_file("/usr/local");
        let dir = Path::new("/opt/myapp/lib/pkgconfig");
        let prefix = compute_prefix_from_pc_dir(&pc, dir);
        assert_eq!(prefix.as_deref(), Some("/opt/myapp"));
    }

    #[test]
    fn compute_prefix_share_pkgconfig() {
        let pc = make_test_pc_file("/usr");
        let dir = Path::new("/opt/myapp/share/pkgconfig");
        let prefix = compute_prefix_from_pc_dir(&pc, dir);
        assert_eq!(prefix.as_deref(), Some("/opt/myapp"));
    }

    #[test]
    fn compute_prefix_lib64_pkgconfig() {
        let pc = make_test_pc_file("/usr");
        let dir = Path::new("/opt/myapp/lib64/pkgconfig");
        let prefix = compute_prefix_from_pc_dir(&pc, dir);
        assert_eq!(prefix.as_deref(), Some("/opt/myapp"));
    }

    #[test]
    fn compute_prefix_not_pkgconfig_dir() {
        let pc = make_test_pc_file("/usr");
        let dir = Path::new("/opt/myapp/custom");
        let prefix = compute_prefix_from_pc_dir(&pc, dir);
        assert_eq!(prefix.as_deref(), Some("/opt/myapp"));
    }

    // ── Loading from .pc files ─────────────────────────────────────

    #[test]
    fn from_pc_file_zlib() {
        let test_data = test_data_dir();
        let pc_path = test_data.join("zlib.pc");
        let pc = PcFile::from_path(&pc_path).unwrap();
        let client = Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .build();
        let pkg = Package::from_pc_file(&client, &pc, "zlib").unwrap();

        assert_eq!(pkg.id, "zlib");
        assert_eq!(pkg.version, "1.2.13");
        assert_eq!(pkg.realname.as_deref(), Some("zlib"));
        assert_eq!(pkg.description.as_deref(), Some("zlib compression library"));
        assert!(!pkg.libs.is_empty());
        assert!(!pkg.cflags.is_empty());
        assert!(!pkg.is_virtual());
        assert!(!pkg.is_uninstalled());
    }

    #[test]
    fn from_pc_file_libfoo() {
        let test_data = test_data_dir();
        let pc_path = test_data.join("libfoo.pc");
        let pc = PcFile::from_path(&pc_path).unwrap();
        let client = Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .build();
        let pkg = Package::from_pc_file(&client, &pc, "libfoo").unwrap();

        assert_eq!(pkg.id, "libfoo");
        assert_eq!(pkg.version, "1.2.3");
        assert!(!pkg.requires.is_empty());
        assert!(!pkg.requires_private.is_empty());
        assert!(!pkg.conflicts.is_empty());
        assert!(!pkg.provides.is_empty());
        assert!(!pkg.libs.is_empty());
        assert!(!pkg.libs_private.is_empty());
        assert_eq!(pkg.license.as_deref(), Some("MIT"));
    }

    #[test]
    fn from_pc_file_libbar() {
        let test_data = test_data_dir();
        let pc_path = test_data.join("libbar.pc");
        let pc = PcFile::from_path(&pc_path).unwrap();
        let client = Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .build();
        let pkg = Package::from_pc_file(&client, &pc, "libbar").unwrap();

        assert_eq!(pkg.id, "libbar");
        assert_eq!(pkg.version, "2.3.1");
        assert_eq!(pkg.realname.as_deref(), Some("Bar"));
        assert_eq!(pkg.url.as_deref(), Some("https://example.com/bar"));
        assert_eq!(pkg.license.as_deref(), Some("MIT"));
        assert!(pkg.maintainer.is_some());
        assert!(pkg.copyright.is_some());
        assert!(pkg.source.is_some());
        assert!(!pkg.requires.is_empty());
        assert!(!pkg.requires_private.is_empty());
        assert!(!pkg.conflicts.is_empty());
        assert!(!pkg.provides.is_empty());
    }

    #[test]
    fn find_package_in_test_data() {
        let test_data = test_data_dir();
        let client = Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .with_path(test_data.to_str().unwrap())
            .build();

        let pkg = Package::find(&client, "zlib").unwrap();
        assert_eq!(pkg.id, "zlib");
        assert_eq!(pkg.version, "1.2.13");
    }

    #[test]
    fn find_package_not_found() {
        let client = Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .build();

        let result = Package::find(&client, "nonexistent");
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::PackageNotFound { name } => assert_eq!(name, "nonexistent"),
            e => panic!("Expected PackageNotFound, got: {e:?}"),
        }
    }

    #[test]
    fn find_package_by_path() {
        let test_data = test_data_dir();
        let pc_path = test_data.join("zlib.pc");
        let client = Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .build();

        let pkg = Package::find(&client, pc_path.to_str().unwrap()).unwrap();
        assert_eq!(pkg.version, "1.2.13");
    }

    #[test]
    fn scan_all_packages() {
        let test_data = test_data_dir();
        let client = Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .with_path(test_data.to_str().unwrap())
            .build();

        let packages = Package::scan_all(&client);
        assert!(packages.len() >= 3); // zlib, libfoo, libbar
        assert!(packages.iter().any(|p| p.id == "zlib"));
        assert!(packages.iter().any(|p| p.id == "libfoo"));
        assert!(packages.iter().any(|p| p.id == "libbar"));
    }

    #[test]
    fn verify_dependency_with_loaded_package() {
        let test_data = test_data_dir();
        let client = Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .with_path(test_data.to_str().unwrap())
            .build();

        let pkg = Package::find(&client, "zlib").unwrap();

        // zlib 1.2.13 should satisfy >= 1.2.0
        let dep = Dependency::with_version("zlib", Comparator::GreaterThanEqual, "1.2.0");
        assert!(pkg.verify_dependency(&dep));

        // zlib 1.2.13 should not satisfy >= 2.0
        let dep = Dependency::with_version("zlib", Comparator::GreaterThanEqual, "2.0");
        assert!(!pkg.verify_dependency(&dep));
    }

    // ── Helpers ────────────────────────────────────────────────────

    fn test_data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests")
            .join("data")
    }

    /// Create a minimal PcFile with a prefix variable for testing.
    fn make_test_pc_file(prefix: &str) -> PcFile {
        let content = format!("prefix={prefix}\nName: test\nVersion: 1.0\n");
        PcFile::from_str(&content, Path::new("test.pc")).unwrap()
    }
}
