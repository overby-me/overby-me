//! Package cache for avoiding redundant `.pc` file lookups.
//!
//! The [`Cache`] struct provides an in-memory cache of loaded [`Package`]s,
//! keyed by their lookup identifier (e.g. `"zlib"`, `"glib-2.0"`). This
//! avoids re-parsing and re-resolving the same `.pc` file multiple times
//! during dependency graph resolution.
//!
//! # Virtual Package Registration
//!
//! The cache also supports preloading virtual packages (such as the built-in
//! `pkg-config` and `pkgconf` packages) via [`Cache::register()`].
//!
//! # Cache Bypass
//!
//! When the client's `NO_CACHE` flag is set, [`Cache::lookup()`] always
//! returns `None`, effectively disabling the cache while still allowing
//! packages to be registered for virtual package resolution.

use std::collections::HashMap;

use crate::client::{Client, ClientFlags};
use crate::error::Result;
use crate::pkg::{self, Package};

/// An in-memory cache of resolved packages.
///
/// Packages are stored by their lookup identifier and can be retrieved
/// without re-parsing their `.pc` files.
#[derive(Debug, Clone, Default)]
pub struct Cache {
    /// The package store, keyed by package id.
    packages: HashMap<String, Package>,
}

impl Cache {
    /// Create a new, empty cache.
    pub fn new() -> Self {
        Self {
            packages: HashMap::new(),
        }
    }

    /// Create a new cache pre-populated with the built-in virtual packages
    /// (`pkg-config` and `pkgconf`).
    pub fn with_builtins(client: &Client) -> Self {
        let mut cache = Self::new();
        cache.register(pkg::builtin_pkg_config(client));
        cache.register(pkg::builtin_pkgconf(client));
        cache
    }

    /// Look up a package by its identifier.
    ///
    /// Returns `None` if the package is not cached or if the client's
    /// `NO_CACHE` flag is set (unless the package is virtual, in which
    /// case it is always returned).
    pub fn lookup(&self, name: &str, client: &Client) -> Option<&Package> {
        let pkg = self.packages.get(name)?;

        // Virtual packages are always returned, even with NO_CACHE
        if pkg.is_virtual() {
            return Some(pkg);
        }

        // Respect NO_CACHE for non-virtual packages
        if client.flags().contains(ClientFlags::NO_CACHE) {
            return None;
        }

        Some(pkg)
    }

    /// Look up a package by its identifier, without checking client flags.
    ///
    /// This is useful for internal operations that need to check the cache
    /// regardless of the `NO_CACHE` flag.
    pub fn lookup_unchecked(&self, name: &str) -> Option<&Package> {
        self.packages.get(name)
    }

    /// Look up a package that provides the given name.
    ///
    /// Searches all cached packages to find one whose `Provides` list
    /// includes the requested name. Returns `None` if no provider is found.
    pub fn lookup_provider(&self, name: &str, client: &Client) -> Option<&Package> {
        for pkg in self.packages.values() {
            // Skip non-virtual packages if NO_CACHE is set
            if !pkg.is_virtual() && client.flags().contains(ClientFlags::NO_CACHE) {
                continue;
            }

            if pkg.satisfies_name(name) && pkg.id != name {
                return Some(pkg);
            }
        }
        None
    }

    /// Add a package to the cache.
    ///
    /// If a package with the same id already exists, it is replaced.
    /// The package's `CACHED` flag is set automatically.
    ///
    /// If the client's `NO_CACHE` flag is set, the package is still stored
    /// but will not be returned by [`lookup()`](Cache::lookup) unless it
    /// is virtual.
    pub fn add(&mut self, mut pkg: Package) {
        use crate::pkg::PackageFlags;
        pkg.flags = pkg.flags.set(PackageFlags::CACHED);
        self.packages.insert(pkg.id.clone(), pkg);
    }

    /// Register a virtual or preloaded package in the cache.
    ///
    /// This is equivalent to [`add()`](Cache::add) but makes the intent
    /// clearer for virtual packages and preloaded `.pc` files.
    pub fn register(&mut self, pkg: Package) {
        self.add(pkg);
    }

    /// Remove a package from the cache by id.
    ///
    /// Returns the removed package, or `None` if it was not cached.
    pub fn remove(&mut self, name: &str) -> Option<Package> {
        self.packages.remove(name)
    }

    /// Check if a package is present in the cache.
    pub fn contains(&self, name: &str) -> bool {
        self.packages.contains_key(name)
    }

    /// Get the number of cached packages.
    pub fn len(&self) -> usize {
        self.packages.len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// Clear all non-virtual packages from the cache.
    ///
    /// Virtual packages (built-ins, preloaded) are preserved.
    pub fn clear_non_virtual(&mut self) {
        self.packages.retain(|_, pkg| pkg.is_virtual());
    }

    /// Clear the entire cache, including virtual packages.
    pub fn clear(&mut self) {
        self.packages.clear();
    }

    /// Iterate over all cached packages.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Package)> {
        self.packages.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Get all cached package ids.
    pub fn ids(&self) -> Vec<&str> {
        self.packages.keys().map(|s| s.as_str()).collect()
    }

    /// Find and load a package, using the cache if possible.
    ///
    /// This is the primary entry point for package resolution. It:
    ///
    /// 1. Checks the cache for the package (respecting `NO_CACHE`).
    /// 2. If not found, checks for a provider in the cache.
    /// 3. If still not found, searches the filesystem via [`Package::find()`].
    /// 4. Caches the loaded package for future lookups.
    ///
    /// Returns a reference to the cached package on success.
    pub fn find_or_load(&mut self, name: &str, client: &Client) -> Result<&Package> {
        // Check cache first
        if self.lookup(name, client).is_some() {
            // We need to return the reference, but the borrow checker won't let
            // us borrow from `self` while also potentially mutating it below.
            // So we use a key-based approach.
            return Ok(&self.packages[name]);
        }

        // Check if any cached package provides this name
        if let Some(provider_id) = self.lookup_provider(name, client).map(|p| p.id.clone()) {
            return Ok(&self.packages[&provider_id]);
        }

        // Load from the filesystem.
        // The package's id (derived from the .pc filename stem) may differ
        // from the query `name` (e.g. when `name` is a full path like
        // "/usr/lib/pkgconfig/foo.pc" but the id is "foo"). We store the
        // package under its canonical id, and if the query name differs,
        // we also store an alias so future lookups by the original name
        // will find it.
        let pkg = Package::find(client, name)?;
        let id = pkg.id.clone();
        self.add(pkg);

        // If the query name differs from the package id (e.g. a full path
        // was used), store an alias clone so the package can be found by
        // either key.
        if name != id
            && let Some(pkg) = self.packages.get(&id).cloned()
        {
            self.packages.insert(name.to_string(), pkg);
        }

        Ok(&self.packages[&id])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dependency::Dependency;
    use crate::pkg::PackageFlags;
    use crate::version::Comparator;
    use std::path::PathBuf;

    fn test_client() -> Client {
        Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .build()
    }

    fn test_client_no_cache() -> Client {
        Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .flag(ClientFlags::NO_CACHE)
            .build()
    }

    fn test_data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests")
            .join("data")
    }

    // ── Basic operations ────────────────────────────────────────────

    #[test]
    fn new_cache_is_empty() {
        let cache = Cache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn add_and_lookup() {
        let client = test_client();
        let mut cache = Cache::new();
        let pkg = Package::new_virtual("test", "1.0");
        cache.add(pkg);

        assert!(cache.contains("test"));
        assert_eq!(cache.len(), 1);

        let found = cache.lookup("test", &client).unwrap();
        assert_eq!(found.id, "test");
        assert_eq!(found.version, "1.0");
        assert!(found.flags.contains(PackageFlags::CACHED));
    }

    #[test]
    fn lookup_missing_returns_none() {
        let client = test_client();
        let cache = Cache::new();
        assert!(cache.lookup("nonexistent", &client).is_none());
    }

    #[test]
    fn remove_package() {
        let mut cache = Cache::new();
        let pkg = Package::new_virtual("test", "1.0");
        cache.add(pkg);
        assert!(cache.contains("test"));

        let removed = cache.remove("test");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id, "test");
        assert!(!cache.contains("test"));
    }

    #[test]
    fn remove_missing_returns_none() {
        let mut cache = Cache::new();
        assert!(cache.remove("nonexistent").is_none());
    }

    #[test]
    fn add_replaces_existing() {
        let client = test_client();
        let mut cache = Cache::new();

        let pkg1 = Package::new_virtual("test", "1.0");
        cache.add(pkg1);

        let pkg2 = Package::new_virtual("test", "2.0");
        cache.add(pkg2);

        assert_eq!(cache.len(), 1);
        let found = cache.lookup("test", &client).unwrap();
        assert_eq!(found.version, "2.0");
    }

    // ── NO_CACHE flag ──────────────────────────────────────────────

    #[test]
    fn no_cache_flag_skips_non_virtual() {
        let client = test_client_no_cache();
        let mut cache = Cache::new();

        // A non-virtual package should not be returned with NO_CACHE
        let mut pkg = Package::new_virtual("test", "1.0");
        pkg.flags = PackageFlags::NONE; // remove VIRTUAL
        cache.add(pkg);

        assert!(cache.lookup("test", &client).is_none());
    }

    #[test]
    fn no_cache_flag_returns_virtual() {
        let client = test_client_no_cache();
        let mut cache = Cache::new();

        // Virtual packages should still be returned with NO_CACHE
        let pkg = Package::new_virtual("pkg-config", "0.29.2");
        cache.add(pkg);

        let found = cache.lookup("pkg-config", &client);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "pkg-config");
    }

    #[test]
    fn lookup_unchecked_ignores_no_cache() {
        let mut cache = Cache::new();

        let mut pkg = Package::new_virtual("test", "1.0");
        pkg.flags = PackageFlags::NONE;
        cache.add(pkg);

        // lookup_unchecked should always return the package
        let found = cache.lookup_unchecked("test");
        assert!(found.is_some());
    }

    // ── Built-in packages ──────────────────────────────────────────

    #[test]
    fn with_builtins_has_pkg_config() {
        let client = test_client();
        let cache = Cache::with_builtins(&client);

        assert!(cache.contains("pkg-config"));
        assert!(cache.contains("pkgconf"));

        let pkg = cache.lookup("pkg-config", &client).unwrap();
        assert_eq!(pkg.version, crate::PKGCONFIG_COMPAT_VERSION);
        assert!(pkg.is_virtual());
    }

    #[test]
    fn with_builtins_has_pkgconf() {
        let client = test_client();
        let cache = Cache::with_builtins(&client);

        let pkg = cache.lookup("pkgconf", &client).unwrap();
        assert_eq!(pkg.version, crate::VERSION);
        assert!(pkg.is_virtual());
    }

    // ── Provider lookup ────────────────────────────────────────────

    #[test]
    fn lookup_provider_finds_package() {
        let client = test_client();
        let mut cache = Cache::new();

        let mut pkg = Package::new_virtual("libfoo", "1.0");
        pkg.provides.push(Dependency::with_version(
            "libfoo-compat",
            Comparator::Equal,
            "1.0",
        ));
        cache.add(pkg);

        let provider = cache.lookup_provider("libfoo-compat", &client);
        assert!(provider.is_some());
        assert_eq!(provider.unwrap().id, "libfoo");
    }

    #[test]
    fn lookup_provider_not_found() {
        let client = test_client();
        let cache = Cache::new();
        assert!(cache.lookup_provider("nonexistent", &client).is_none());
    }

    #[test]
    fn lookup_provider_skips_self_id() {
        let client = test_client();
        let mut cache = Cache::new();

        // A package should not be returned as its own provider
        let pkg = Package::new_virtual("test", "1.0");
        cache.add(pkg);

        assert!(cache.lookup_provider("test", &client).is_none());
    }

    // ── Register ───────────────────────────────────────────────────

    #[test]
    fn register_adds_package() {
        let client = test_client();
        let mut cache = Cache::new();

        let pkg = Package::new_virtual("custom", "3.0");
        cache.register(pkg);

        assert!(cache.contains("custom"));
        let found = cache.lookup("custom", &client).unwrap();
        assert_eq!(found.version, "3.0");
        assert!(found.flags.contains(PackageFlags::CACHED));
    }

    // ── Clear operations ───────────────────────────────────────────

    #[test]
    fn clear_removes_all() {
        let mut cache = Cache::new();
        cache.add(Package::new_virtual("a", "1.0"));
        cache.add(Package::new_virtual("b", "2.0"));
        assert_eq!(cache.len(), 2);

        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn clear_non_virtual_preserves_virtual() {
        let mut cache = Cache::new();

        // Add a virtual package
        cache.add(Package::new_virtual("builtin", "1.0"));

        // Add a non-virtual package
        let mut non_virtual = Package::new_virtual("real-pkg", "2.0");
        non_virtual.flags = PackageFlags::NONE;
        cache.add(non_virtual);

        assert_eq!(cache.len(), 2);

        cache.clear_non_virtual();

        assert_eq!(cache.len(), 1);
        assert!(cache.contains("builtin"));
        assert!(!cache.contains("real-pkg"));
    }

    // ── Iteration ──────────────────────────────────────────────────

    #[test]
    fn iter_all_packages() {
        let mut cache = Cache::new();
        cache.add(Package::new_virtual("a", "1.0"));
        cache.add(Package::new_virtual("b", "2.0"));

        let ids: Vec<&str> = cache.iter().map(|(id, _)| id).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"a"));
        assert!(ids.contains(&"b"));
    }

    #[test]
    fn ids_returns_all_keys() {
        let mut cache = Cache::new();
        cache.add(Package::new_virtual("x", "1.0"));
        cache.add(Package::new_virtual("y", "2.0"));

        let ids = cache.ids();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"x"));
        assert!(ids.contains(&"y"));
    }

    // ── find_or_load ───────────────────────────────────────────────

    #[test]
    fn find_or_load_from_cache() {
        let client = test_client();
        let mut cache = Cache::new();
        cache.add(Package::new_virtual("cached-pkg", "1.0"));

        let pkg = cache.find_or_load("cached-pkg", &client).unwrap();
        assert_eq!(pkg.id, "cached-pkg");
        assert_eq!(pkg.version, "1.0");
    }

    #[test]
    fn find_or_load_from_filesystem() {
        let test_data = test_data_dir();
        let client = Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .with_path(test_data.to_str().unwrap())
            .build();

        let mut cache = Cache::new();
        let pkg = cache.find_or_load("zlib", &client).unwrap();
        assert_eq!(pkg.id, "zlib");
        assert_eq!(pkg.version, "1.2.13");

        // Should now be cached
        assert!(cache.contains("zlib"));
    }

    #[test]
    fn find_or_load_caches_result() {
        let test_data = test_data_dir();
        let client = Client::builder()
            .skip_env(true)
            .skip_default_paths(true)
            .with_path(test_data.to_str().unwrap())
            .build();

        let mut cache = Cache::new();

        // First load from filesystem
        let _ = cache.find_or_load("zlib", &client).unwrap();
        assert!(cache.contains("zlib"));

        // Second load should come from cache
        let pkg = cache.find_or_load("zlib", &client).unwrap();
        assert_eq!(pkg.id, "zlib");
        assert!(pkg.flags.contains(PackageFlags::CACHED));
    }

    #[test]
    fn find_or_load_not_found() {
        let client = test_client();
        let mut cache = Cache::new();

        let result = cache.find_or_load("nonexistent", &client);
        assert!(result.is_err());
    }

    #[test]
    fn find_or_load_via_provider() {
        let client = test_client();
        let mut cache = Cache::new();

        // Register a package that provides "libfoo-compat"
        let mut pkg = Package::new_virtual("libfoo", "1.2.3");
        pkg.provides.push(Dependency::with_version(
            "libfoo-compat",
            Comparator::Equal,
            "1.2.3",
        ));
        cache.register(pkg);

        // Looking up "libfoo-compat" should find the provider
        let found = cache.find_or_load("libfoo-compat", &client).unwrap();
        assert_eq!(found.id, "libfoo");
    }
}
