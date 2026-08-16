//! Dependency graph solver, traversal, and fragment collection.
//!
//! This module implements the core dependency resolution algorithm, modelled
//! after pkgconf's queue/solver system. The main entry point is [`solve()`],
//! which takes a set of package queries and resolves them into a fully-loaded
//! dependency graph rooted at a synthetic "world" package.
//!
//! After solving, use [`collect_cflags()`] and [`collect_libs()`] to gather
//! compiler/linker flags from the resolved graph, or [`traverse()`] for
//! generic depth-first traversal with a callback.
//!
//! # Example
//!
//! ```rust,no_run
//! use libpkgconf::cache::Cache;
//! use libpkgconf::client::Client;
//! use libpkgconf::queue;
//!
//! let client = Client::new();
//! let mut cache = Cache::with_builtins(&client);
//!
//! // Solve for "zlib >= 1.2"
//! let world = queue::solve(&mut cache, &client, &["zlib >= 1.2".to_string()]).unwrap();
//!
//! // Collect flags from the resolved graph
//! let cflags = queue::collect_cflags(&cache, &client, &world);
//! let libs = queue::collect_libs(&cache, &client, &world);
//! ```

use std::collections::HashSet;

use crate::cache::Cache;
use crate::client::{Client, ClientFlags};
use crate::dependency::{Dependency, DependencyList};
use crate::error::{Error, Result};
use crate::fragment::FragmentList;
use crate::pkg::{Package, PackageFlags};

// ── Queue ───────────────────────────────────────────────────────────────

/// An ordered queue of package queries to resolve.
///
/// The queue accumulates package specifications (names with optional version
/// constraints) and then compiles them into a dependency list that is attached
/// to a synthetic "world" package for resolution.
#[derive(Debug, Clone, Default)]
pub struct Queue {
    /// Raw package query strings (e.g. `"glib-2.0 >= 2.50"`, `"zlib"`).
    entries: Vec<String>,
}

impl Queue {
    /// Create a new, empty queue.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Push a raw package query string onto the queue.
    ///
    /// The string is parsed lazily when [`compile()`](Queue::compile) or
    /// [`solve()`](Queue::solve) is called.
    pub fn push(&mut self, query: impl Into<String>) {
        self.entries.push(query.into());
    }

    /// Push a pre-parsed dependency onto the queue.
    pub fn push_dependency(&mut self, dep: &Dependency) {
        self.entries.push(format!("{dep}"));
    }

    /// Get the number of entries in the queue.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get the raw entries.
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Compile the queue entries into a world package with a populated
    /// dependency list.
    ///
    /// This parses all query strings into [`Dependency`] entries and
    /// attaches them to a new world package's `requires` list.
    pub fn compile(&self) -> Package {
        let mut world = crate::pkg::world_package();
        let query = self.entries.join(", ");
        if !query.is_empty() {
            world.requires = DependencyList::parse(&query);
        }
        world
    }

    /// Solve the dependency graph.
    ///
    /// This is a convenience method that calls [`compile()`](Queue::compile)
    /// followed by [`solve_world()`].
    pub fn solve(&self, cache: &mut Cache, client: &Client) -> Result<Package> {
        let world = self.compile();
        solve_world(cache, client, world)
    }

    /// Validate that all packages in the queue exist and parse correctly,
    /// without fully resolving the dependency graph.
    ///
    /// Returns `Ok(())` if all top-level packages are valid, or an error
    /// describing the first problem found.
    pub fn validate(&self, cache: &mut Cache, client: &Client) -> Result<()> {
        let world = self.compile();

        for dep in world.requires.iter() {
            let pkg = load_package(cache, client, dep)?;

            // Check version constraint
            verify_version(dep, pkg)?;
        }

        Ok(())
    }
}

// ── Top-level solver API ────────────────────────────────────────────────

/// Solve the dependency graph for the given package queries.
///
/// This is the main entry point for dependency resolution. It:
///
/// 1. Parses the query strings into a world package's dependency list.
/// 2. Recursively loads and caches all required packages.
/// 3. Verifies version constraints.
/// 4. Checks for conflicts (unless disabled).
///
/// Returns the world package on success. Use [`collect_cflags()`] and
/// [`collect_libs()`] to extract flags from the resolved graph.
///
/// # Arguments
///
/// * `cache` — The package cache (will be populated with loaded packages).
/// * `client` — The client providing search paths and configuration.
/// * `queries` — Package query strings (e.g. `["glib-2.0 >= 2.50", "zlib"]`).
pub fn solve(cache: &mut Cache, client: &Client, queries: &[String]) -> Result<Package> {
    let mut queue = Queue::new();
    for q in queries {
        queue.push(q);
    }
    queue.solve(cache, client)
}

/// Solve the dependency graph starting from a pre-built world package.
///
/// The world package's `requires` list is used as the set of top-level
/// dependencies to resolve.
pub fn solve_world(cache: &mut Cache, client: &Client, world: Package) -> Result<Package> {
    let max_depth = client.max_traversal_depth();

    // Phase 1: Recursively load all required packages into the cache.
    let requires = world.requires.clone();
    resolve_deps(cache, client, &requires, 0, max_depth)?;

    // If static linking, also resolve private dependencies.
    if client.is_static() {
        let requires_private = world.requires_private.clone();
        resolve_deps(cache, client, &requires_private, 0, max_depth)?;
    }

    // Phase 2: Verify the full dependency graph (version constraints,
    // conflicts) using only immutable access to the cache.
    verify_graph(cache, client, &world)?;

    Ok(world)
}

/// Apply a callback function to each package in the dependency graph,
/// visited in depth-first order.
///
/// The callback receives a reference to each package and the current
/// traversal depth. If the callback returns `false`, traversal of
/// that subtree is skipped (but other branches continue).
///
/// # Arguments
///
/// * `cache` — The package cache containing resolved packages.
/// * `client` — The client configuration.
/// * `world` — The root (world) package.
/// * `callback` — Called for each visited package. Return `true` to
///   continue into children, `false` to skip children.
pub fn apply<F>(cache: &Cache, client: &Client, world: &Package, mut callback: F)
where
    F: FnMut(&Package, i32) -> bool,
{
    let mut visited = HashSet::new();
    let include_private = client.is_static();
    let skip_root = client.flags().contains(ClientFlags::SKIP_ROOT_VIRTUAL);

    apply_recursive(
        cache,
        client,
        world,
        0,
        &mut visited,
        include_private,
        skip_root,
        &mut callback,
    );
}

// ── Fragment collection ─────────────────────────────────────────────────

/// Collect all cflags from the resolved dependency graph.
///
/// Traverses the graph depth-first starting from the world package,
/// collecting each package's `Cflags` fragments. When static linking
/// is enabled, `Cflags.private` fragments are also included.
///
/// The returned fragment list is **not** deduplicated or filtered —
/// call [`FragmentList::deduplicate()`] and filtering methods on the
/// result as needed.
pub fn collect_cflags(cache: &Cache, client: &Client, world: &Package) -> FragmentList {
    let mut result = FragmentList::new();
    let include_private = client.is_static()
        || client
            .flags()
            .contains(ClientFlags::MERGE_PRIVATE_FRAGMENTS);

    apply(cache, client, world, |pkg, _depth| {
        // Skip virtual packages (they have no real flags)
        if pkg.is_virtual() {
            return true;
        }

        result.append(&pkg.cflags);
        if include_private {
            result.append(&pkg.cflags_private);
        }
        true
    });

    result
}

/// Collect all libs from the resolved dependency graph.
///
/// Traverses the graph depth-first starting from the world package,
/// collecting each package's `Libs` fragments. When static linking
/// is enabled, `Libs.private` fragments are also included.
///
/// The returned fragment list is **not** deduplicated or filtered —
/// call [`FragmentList::deduplicate()`] and filtering methods on the
/// result as needed.
pub fn collect_libs(cache: &Cache, client: &Client, world: &Package) -> FragmentList {
    let mut result = FragmentList::new();
    let include_private = client.is_static()
        || client
            .flags()
            .contains(ClientFlags::MERGE_PRIVATE_FRAGMENTS);

    apply(cache, client, world, |pkg, _depth| {
        // Skip virtual packages
        if pkg.is_virtual() {
            return true;
        }

        result.append(&pkg.libs);
        if include_private {
            result.append(&pkg.libs_private);
        }
        true
    });

    result
}

/// Collect version strings for each top-level dependency.
///
/// Returns a list of `(package_name, version)` pairs for each direct
/// dependency of the world package.
pub fn collect_modversions(cache: &Cache, world: &Package) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for dep in world.requires.iter() {
        if let Some(pkg) = find_in_cache(cache, &dep.package) {
            result.push((pkg.id.clone(), pkg.version.clone()));
        }
    }
    result
}

/// Check if all packages in the resolved graph exist and satisfy their
/// version constraints. Returns `true` on success, `false` on failure.
///
/// This is used for `--exists` checks.
pub fn exists(cache: &mut Cache, client: &Client, queries: &[String]) -> bool {
    solve(cache, client, queries).is_ok()
}

/// Check whether any package in the resolved graph is uninstalled.
pub fn has_uninstalled(cache: &Cache, world: &Package) -> bool {
    let mut found = false;
    let mut visited = HashSet::new();

    check_uninstalled_recursive(cache, world, &mut visited, &mut found);
    found
}

/// Produce a graphviz dot-format representation of the dependency graph.
pub fn digraph(
    cache: &Cache,
    client: &Client,
    world: &Package,
    include_query_nodes: bool,
) -> String {
    let mut out = String::from("digraph {\n");
    let mut visited = HashSet::new();

    if include_query_nodes {
        out.push_str("  \"virtual:world\" [style=dotted];\n");
    }

    digraph_recursive(
        cache,
        client,
        world,
        &mut visited,
        &mut out,
        include_query_nodes,
    );

    out.push_str("}\n");
    out
}

/// Print the dependency solution (list of resolved packages with versions).
pub fn solution(cache: &Cache, client: &Client, world: &Package) -> Vec<(String, String)> {
    let mut result = Vec::new();
    apply(cache, client, world, |pkg, _depth| {
        if !pkg.is_virtual() {
            result.push((pkg.id.clone(), pkg.version.clone()));
        }
        true
    });
    result
}

// ── Internal: dependency resolution ─────────────────────────────────────

/// Recursively resolve all dependencies, loading packages into the cache.
fn resolve_deps(
    cache: &mut Cache,
    client: &Client,
    deps: &DependencyList,
    depth: i32,
    max_depth: i32,
) -> Result<()> {
    if depth > max_depth {
        return Err(Error::MaxDepthExceeded {
            name: "dependency graph".to_string(),
            depth: max_depth,
        });
    }

    for dep in deps.iter() {
        // Skip if already in the cache (either by id or via provider)
        if cache.contains(&dep.package) || cache.lookup_provider(&dep.package, client).is_some() {
            // Still verify the version constraint even for cached packages
            let pkg = find_in_cache_or_provider(cache, client, &dep.package);
            if let Some(pkg) = pkg {
                verify_version(dep, pkg)?;
            }
            continue;
        }

        // Load the package (this may find it via provides in the cache)
        let pkg = load_package(cache, client, dep)?;

        // Verify version constraint
        verify_version(dep, pkg)?;

        // Clone the dependency lists before recursing (borrow checker)
        let sub_requires = pkg.requires.clone();
        let sub_requires_private = pkg.requires_private.clone();

        // Recursively resolve public dependencies
        resolve_deps(cache, client, &sub_requires, depth + 1, max_depth)?;

        // Resolve private dependencies when static linking
        if client.is_static() {
            resolve_deps(cache, client, &sub_requires_private, depth + 1, max_depth)?;
        }
    }

    Ok(())
}

/// Load a package into the cache, returning a reference to it.
///
/// Handles provider resolution: if a package `foo` is not found directly
/// but another cached package provides `foo`, the provider is returned.
fn load_package<'a>(
    cache: &'a mut Cache,
    client: &Client,
    dep: &Dependency,
) -> Result<&'a Package> {
    // Two-phase approach to satisfy the borrow checker:
    // Phase 1: Try to load the package, capturing only whether it succeeded
    //          or what kind of error occurred (without holding a borrow).
    // Phase 2: If it succeeded, return the cached reference. If it failed
    //          with PackageNotFound, try provider resolution.

    // When dep.package is a file path (e.g. "/usr/lib/pkgconfig/foo.pc"),
    // find_or_load stores the package under its id (the file stem, e.g. "foo"),
    // not the full path. We capture the actual id so we can look it up.
    let load_result: std::result::Result<String, Error> =
        match cache.find_or_load(&dep.package, client) {
            Ok(pkg) => Ok(pkg.id.clone()),
            Err(e) => Err(e),
        };

    match load_result {
        Ok(pkg_id) => {
            // Package was loaded successfully — return the cached reference.
            // Use the actual package id (which may differ from dep.package
            // when querying by file path).
            Ok(cache.lookup_unchecked(&pkg_id).unwrap())
        }
        Err(Error::PackageNotFound { .. })
            if !client.flags().contains(ClientFlags::SKIP_PROVIDES) =>
        {
            // The package wasn't found directly. Before giving up, scan the
            // filesystem for a package that provides this name.
            //
            // This is the "expensive" path — we only hit it when a package
            // is referenced by an alias rather than its .pc filename.
            find_provider(cache, client, &dep.package)
        }
        Err(e) => Err(e),
    }
}

/// Search for a package that provides the given name.
///
/// This loads packages from the search path and checks their Provides
/// lists to find one that satisfies the requested name.
fn find_provider<'a>(cache: &'a mut Cache, client: &Client, name: &str) -> Result<&'a Package> {
    // First check if any already-cached package provides this name
    if let Some(provider) = cache.lookup_provider(name, client) {
        let id = provider.id.clone();
        return Ok(cache.lookup_unchecked(&id).unwrap());
    }

    // Scan all .pc files in search paths looking for a provider
    let all_pc = client.dir_list().list_all_pc_files();
    for (pkg_name, _path) in &all_pc {
        // Skip if already loaded
        if cache.contains(pkg_name) {
            continue;
        }

        // Try loading this package
        if let Ok(pkg) = Package::find(client, pkg_name) {
            let satisfies = pkg.satisfies_name(name);
            let id = pkg.id.clone();
            cache.add(pkg);

            if satisfies {
                return Ok(cache.lookup_unchecked(&id).unwrap());
            }
        }
    }

    Err(Error::PackageNotFound {
        name: name.to_string(),
    })
}

/// Verify that a package satisfies a dependency's version constraint.
fn verify_version(dep: &Dependency, pkg: &Package) -> Result<()> {
    if let Some(ref required) = dep.version
        && !dep.compare.eval(&pkg.version, required)
    {
        return Err(Error::VersionMismatch {
            name: dep.package.clone(),
            found: pkg.version.clone(),
            required: required.clone(),
            comparator: dep.compare.as_str().to_string(),
        });
    }
    Ok(())
}

// ── Internal: graph verification ────────────────────────────────────────

/// Verify the entire dependency graph for version constraints and conflicts.
fn verify_graph(cache: &Cache, client: &Client, world: &Package) -> Result<()> {
    let mut visited = HashSet::new();
    let mut errors = Vec::new();
    let include_private = client.is_static();
    let check_conflicts = !client.flags().contains(ClientFlags::SKIP_CONFLICTS)
        && !client.flags().contains(ClientFlags::IGNORE_CONFLICTS);

    verify_recursive(
        cache,
        client,
        world,
        &mut visited,
        &mut errors,
        include_private,
        check_conflicts,
    );

    if errors.is_empty() {
        Ok(())
    } else if errors.len() == 1 {
        Err(errors.into_iter().next().unwrap())
    } else {
        Err(Error::Multiple(errors))
    }
}

/// Recursively verify the graph rooted at `pkg`.
fn verify_recursive(
    cache: &Cache,
    client: &Client,
    pkg: &Package,
    visited: &mut HashSet<String>,
    errors: &mut Vec<Error>,
    include_private: bool,
    check_conflicts: bool,
) {
    if !visited.insert(pkg.id.clone()) {
        return;
    }

    // Check conflicts
    if check_conflicts && !pkg.conflicts.is_empty() {
        for conflict in pkg.conflicts.iter() {
            if let Some(conflicting) = find_in_cache(cache, &conflict.package)
                && conflict.version_satisfied_by(&conflicting.version)
            {
                errors.push(Error::PackageConflict {
                    name: pkg.id.clone(),
                    conflicts_with: format!("{conflict}"),
                });
            }
        }
    }

    // Verify and recurse into public dependencies
    verify_dep_list(
        cache,
        client,
        &pkg.requires,
        visited,
        errors,
        include_private,
        check_conflicts,
    );

    // Verify and recurse into private dependencies when in static mode
    if include_private {
        verify_dep_list(
            cache,
            client,
            &pkg.requires_private,
            visited,
            errors,
            include_private,
            check_conflicts,
        );
    }
}

/// Verify a list of dependencies and recurse into each.
fn verify_dep_list(
    cache: &Cache,
    client: &Client,
    deps: &DependencyList,
    visited: &mut HashSet<String>,
    errors: &mut Vec<Error>,
    include_private: bool,
    check_conflicts: bool,
) {
    for dep in deps.iter() {
        let pkg = find_in_cache_or_provider(cache, client, &dep.package);
        match pkg {
            Some(pkg) => {
                // Verify version
                if let Err(e) = verify_version(dep, pkg)
                    && !client.flags().contains(ClientFlags::SKIP_ERRORS)
                {
                    errors.push(e);
                }
                // Clone the id to avoid borrow issues
                let id = pkg.id.clone();
                if let Some(pkg_ref) = cache.lookup_unchecked(&id) {
                    verify_recursive(
                        cache,
                        client,
                        pkg_ref,
                        visited,
                        errors,
                        include_private,
                        check_conflicts,
                    );
                }
            }
            None => {
                if !client.flags().contains(ClientFlags::SKIP_ERRORS) {
                    errors.push(Error::PackageNotFound {
                        name: dep.package.clone(),
                    });
                }
            }
        }
    }
}

// ── Internal: traversal ─────────────────────────────────────────────────

/// Recursive depth-first traversal with callback.
#[allow(clippy::too_many_arguments)]
fn apply_recursive<F>(
    cache: &Cache,
    client: &Client,
    pkg: &Package,
    depth: i32,
    visited: &mut HashSet<String>,
    include_private: bool,
    skip_root: bool,
    callback: &mut F,
) where
    F: FnMut(&Package, i32) -> bool,
{
    if !visited.insert(pkg.id.clone()) {
        return;
    }

    // Call the callback (skip the root virtual world package if requested)
    let descend = if skip_root && pkg.flags.contains(PackageFlags::VIRTUAL) && depth == 0 {
        true
    } else {
        callback(pkg, depth)
    };

    if !descend {
        return;
    }

    // Traverse public dependencies
    traverse_dep_list(
        cache,
        client,
        &pkg.requires,
        depth + 1,
        visited,
        include_private,
        skip_root,
        callback,
    );

    // Traverse private dependencies when static
    if include_private {
        traverse_dep_list(
            cache,
            client,
            &pkg.requires_private,
            depth + 1,
            visited,
            include_private,
            skip_root,
            callback,
        );
    }
}

/// Traverse a dependency list, looking up each package in the cache.
#[allow(clippy::too_many_arguments)]
fn traverse_dep_list<F>(
    cache: &Cache,
    client: &Client,
    deps: &DependencyList,
    depth: i32,
    visited: &mut HashSet<String>,
    include_private: bool,
    skip_root: bool,
    callback: &mut F,
) where
    F: FnMut(&Package, i32) -> bool,
{
    for dep in deps.iter() {
        if let Some(pkg) = find_in_cache_or_provider(cache, client, &dep.package) {
            // Clone the package id to look it up again (avoid borrowing issues
            // with the mutable callback)
            let id = pkg.id.clone();
            if let Some(pkg_ref) = cache.lookup_unchecked(&id) {
                apply_recursive(
                    cache,
                    client,
                    pkg_ref,
                    depth,
                    visited,
                    include_private,
                    skip_root,
                    callback,
                );
            }
        }
    }
}

// ── Internal: helpers ───────────────────────────────────────────────────

/// Look up a package in the cache by name, checking both direct id and
/// provider resolution.
fn find_in_cache_or_provider<'a>(
    cache: &'a Cache,
    client: &Client,
    name: &str,
) -> Option<&'a Package> {
    cache
        .lookup_unchecked(name)
        .or_else(|| cache.lookup_provider(name, client))
}

/// Look up a package in the cache by direct id only.
fn find_in_cache<'a>(cache: &'a Cache, name: &str) -> Option<&'a Package> {
    cache.lookup_unchecked(name)
}

/// Check if any package in the graph is uninstalled (recursive helper).
fn check_uninstalled_recursive(
    cache: &Cache,
    pkg: &Package,
    visited: &mut HashSet<String>,
    found: &mut bool,
) {
    if *found || !visited.insert(pkg.id.clone()) {
        return;
    }

    if pkg.is_uninstalled() {
        *found = true;
        return;
    }

    for dep in pkg.requires.iter() {
        if let Some(dep_pkg) = cache.lookup_unchecked(&dep.package) {
            check_uninstalled_recursive(cache, dep_pkg, visited, found);
        }
    }
}

/// Recursive helper for digraph output.
fn digraph_recursive(
    cache: &Cache,
    client: &Client,
    pkg: &Package,
    visited: &mut HashSet<String>,
    out: &mut String,
    include_query_nodes: bool,
) {
    if !visited.insert(pkg.id.clone()) {
        return;
    }

    let is_virtual = pkg.is_virtual();

    // Print node
    if !is_virtual || include_query_nodes {
        let label = if let Some(ref name) = pkg.realname {
            format!("{name} {}", pkg.version)
        } else {
            format!("{} {}", pkg.id, pkg.version)
        };
        if is_virtual {
            out.push_str(&format!(
                "  \"{}\" [label=\"{}\", style=dotted];\n",
                pkg.id, label
            ));
        } else {
            out.push_str(&format!("  \"{}\" [label=\"{}\"];\n", pkg.id, label));
        }
    }

    // Print edges for public dependencies
    for dep in pkg.requires.iter() {
        if let Some(dep_pkg) = find_in_cache_or_provider(cache, client, &dep.package) {
            if !is_virtual || include_query_nodes {
                out.push_str(&format!("  \"{}\" -> \"{}\";\n", pkg.id, dep_pkg.id));
            }
            let id = dep_pkg.id.clone();
            if let Some(p) = cache.lookup_unchecked(&id) {
                digraph_recursive(cache, client, p, visited, out, include_query_nodes);
            }
        }
    }

    // Print edges for private dependencies
    if client.is_static() {
        for dep in pkg.requires_private.iter() {
            if let Some(dep_pkg) = find_in_cache_or_provider(cache, client, &dep.package) {
                if !is_virtual || include_query_nodes {
                    out.push_str(&format!(
                        "  \"{}\" -> \"{}\" [style=dashed];\n",
                        pkg.id, dep_pkg.id
                    ));
                }
                let id = dep_pkg.id.clone();
                if let Some(p) = cache.lookup_unchecked(&id) {
                    digraph_recursive(cache, client, p, visited, out, include_query_nodes);
                }
            }
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ClientBuilder;
    use std::path::PathBuf;

    /// Get the path to the test data directory.
    fn test_data_dir() -> PathBuf {
        let manifest = env!("CARGO_MANIFEST_DIR");
        PathBuf::from(manifest)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests")
            .join("data")
    }

    /// Build a client configured to use only the test data directory.
    fn test_client() -> Client {
        ClientBuilder::new()
            .skip_env(true)
            .skip_default_paths(true)
            .with_path(test_data_dir())
            .build()
    }

    /// Build a static-mode client for the test data directory.
    fn test_client_static() -> Client {
        ClientBuilder::new()
            .skip_env(true)
            .skip_default_paths(true)
            .with_path(test_data_dir())
            .enable_static(true)
            .build()
    }

    // ── Queue tests ─────────────────────────────────────────────────

    #[test]
    fn queue_new_is_empty() {
        let q = Queue::new();
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn queue_push_and_len() {
        let mut q = Queue::new();
        q.push("zlib");
        q.push("glib-2.0 >= 2.50");
        assert_eq!(q.len(), 2);
        assert!(!q.is_empty());
    }

    #[test]
    fn queue_push_dependency() {
        let mut q = Queue::new();
        let dep =
            Dependency::with_version("zlib", crate::version::Comparator::GreaterThanEqual, "1.2");
        q.push_dependency(&dep);
        assert_eq!(q.len(), 1);
        assert!(q.entries()[0].contains("zlib"));
    }

    #[test]
    fn queue_compile_creates_world() {
        let mut q = Queue::new();
        q.push("simple");
        q.push("zlib");

        let world = q.compile();
        assert_eq!(world.id, "virtual:world");
        assert!(world.is_virtual());
        assert_eq!(world.requires.len(), 2);
    }

    #[test]
    fn queue_compile_empty() {
        let q = Queue::new();
        let world = q.compile();
        assert!(world.requires.is_empty());
    }

    // ── Solve tests ─────────────────────────────────────────────────

    #[test]
    fn solve_single_no_deps() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(&mut cache, &client, &["zlib".to_string()]).unwrap();
        assert_eq!(world.requires.len(), 1);
        assert!(cache.contains("zlib"));
    }

    #[test]
    fn solve_single_with_version() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let result = solve(&mut cache, &client, &["zlib >= 1.2".to_string()]);
        assert!(result.is_ok());
    }

    #[test]
    fn solve_version_mismatch() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let result = solve(&mut cache, &client, &["zlib >= 99.0".to_string()]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, Error::VersionMismatch { .. }),
            "Expected VersionMismatch, got: {err:?}"
        );
    }

    #[test]
    fn solve_not_found() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let result = solve(&mut cache, &client, &["nonexistent-package".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn solve_transitive_deps() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        // depender depends on simple
        let world = solve(&mut cache, &client, &["depender".to_string()]).unwrap();
        assert_eq!(world.requires.len(), 1);
        assert!(cache.contains("depender"));
        assert!(
            cache.contains("simple"),
            "Transitive dep 'simple' should be in cache"
        );
    }

    #[test]
    fn solve_deep_transitive_deps() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        // deep-depender -> depender -> simple
        let result = solve(&mut cache, &client, &["deep-depender".to_string()]);
        assert!(result.is_ok(), "Failed: {:?}", result.err());
        assert!(cache.contains("deep-depender"));
        assert!(cache.contains("depender"));
        assert!(cache.contains("simple"));
    }

    #[test]
    fn solve_diamond_deps() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        // diamond-a -> diamond-b, diamond-c -> diamond-d
        let result = solve(&mut cache, &client, &["diamond-a".to_string()]);
        assert!(result.is_ok(), "Failed: {:?}", result.err());
        assert!(cache.contains("diamond-a"));
        assert!(cache.contains("diamond-b"));
        assert!(cache.contains("diamond-c"));
        assert!(cache.contains("diamond-d"));
    }

    #[test]
    fn solve_multiple_packages() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let result = solve(
            &mut cache,
            &client,
            &["simple".to_string(), "zlib".to_string()],
        );
        assert!(result.is_ok());
        assert!(cache.contains("simple"));
        assert!(cache.contains("zlib"));
    }

    #[test]
    fn solve_private_deps_not_resolved_by_default() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        // private-deps has Requires.private: zlib
        let result = solve(&mut cache, &client, &["private-deps".to_string()]);
        assert!(result.is_ok(), "Failed: {:?}", result.err());
        assert!(cache.contains("private-deps"));
        assert!(cache.contains("simple")); // public dep
        // zlib might or might not be in cache — it's a private dep
        // In non-static mode, we don't necessarily resolve private deps
    }

    #[test]
    fn solve_private_deps_resolved_in_static() {
        let client = test_client_static();
        let mut cache = Cache::with_builtins(&client);

        let result = solve(&mut cache, &client, &["private-deps".to_string()]);
        assert!(result.is_ok(), "Failed: {:?}", result.err());
        assert!(cache.contains("private-deps"));
        assert!(cache.contains("simple"));
        assert!(
            cache.contains("zlib"),
            "Private dep 'zlib' should be resolved in static mode"
        );
    }

    #[test]
    fn solve_circular_deps_handled() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        // libfoo requires libbar, libbar requires libfoo — circular!
        // The solver should handle this via the cache (second lookup finds it already loaded)
        let result = solve(&mut cache, &client, &["libfoo".to_string()]);
        // This should not stack overflow or error — the cycle is broken by the cache check
        assert!(
            result.is_ok(),
            "Circular deps should be handled gracefully: {:?}",
            result.err()
        );
    }

    #[test]
    fn solve_virtual_package() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let result = solve(&mut cache, &client, &["pkg-config".to_string()]);
        assert!(result.is_ok());
    }

    #[test]
    fn solve_conflict_detected() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        // conflicting conflicts with simple < 2.0, and simple is 1.0.0
        // Load both: requesting both should trigger a conflict
        let result = solve(
            &mut cache,
            &client,
            &["simple".to_string(), "conflicting".to_string()],
        );
        assert!(result.is_err(), "Should detect conflict");
        let err = result.unwrap_err();
        match &err {
            Error::PackageConflict { .. } => {}
            Error::Multiple(errs) => {
                assert!(
                    errs.iter()
                        .any(|e| matches!(e, Error::PackageConflict { .. })),
                    "Expected at least one PackageConflict, got: {err:?}"
                );
            }
            _ => panic!("Expected PackageConflict, got: {err:?}"),
        }
    }

    #[test]
    fn solve_conflict_ignored_with_flag() {
        let client = ClientBuilder::new()
            .skip_env(true)
            .skip_default_paths(true)
            .with_path(test_data_dir())
            .flag(ClientFlags::IGNORE_CONFLICTS)
            .build();
        let mut cache = Cache::with_builtins(&client);

        let result = solve(
            &mut cache,
            &client,
            &["simple".to_string(), "conflicting".to_string()],
        );
        assert!(
            result.is_ok(),
            "Conflicts should be ignored: {:?}",
            result.err()
        );
    }

    // ── Exists tests ────────────────────────────────────────────────

    #[test]
    fn exists_found() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        assert!(exists(&mut cache, &client, &["zlib".to_string()]));
    }

    #[test]
    fn exists_not_found() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        assert!(!exists(&mut cache, &client, &["nonexistent".to_string()]));
    }

    #[test]
    fn exists_version_ok() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        assert!(exists(&mut cache, &client, &["zlib >= 1.0".to_string()]));
    }

    #[test]
    fn exists_version_fail() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        assert!(!exists(&mut cache, &client, &["zlib >= 99.0".to_string()]));
    }

    // ── Fragment collection tests ───────────────────────────────────

    #[test]
    fn collect_cflags_single() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(&mut cache, &client, &["zlib".to_string()]).unwrap();
        let cflags = collect_cflags(&cache, &client, &world);

        let rendered = cflags.render(' ');
        assert!(
            rendered.contains("-I/usr/include"),
            "Expected -I/usr/include in: {rendered}"
        );
    }

    #[test]
    fn collect_libs_single() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(&mut cache, &client, &["zlib".to_string()]).unwrap();
        let libs = collect_libs(&cache, &client, &world);

        let rendered = libs.render(' ');
        assert!(rendered.contains("-lz"), "Expected -lz in: {rendered}");
    }

    #[test]
    fn collect_cflags_transitive() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(&mut cache, &client, &["depender".to_string()]).unwrap();
        let cflags = collect_cflags(&cache, &client, &world);

        let rendered = cflags.render(' ');
        // Should include both depender and simple cflags
        assert!(
            rendered.contains("-I/usr/include/depender"),
            "Expected depender cflags in: {rendered}"
        );
        assert!(
            rendered.contains("-I/usr/include/simple"),
            "Expected simple cflags in: {rendered}"
        );
    }

    #[test]
    fn collect_libs_transitive() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(&mut cache, &client, &["depender".to_string()]).unwrap();
        let libs = collect_libs(&cache, &client, &world);

        let rendered = libs.render(' ');
        assert!(
            rendered.contains("-ldepender"),
            "Expected -ldepender in: {rendered}"
        );
        assert!(
            rendered.contains("-lsimple"),
            "Expected -lsimple in: {rendered}"
        );
    }

    #[test]
    fn collect_libs_diamond_no_duplicates() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(&mut cache, &client, &["diamond-a".to_string()]).unwrap();
        let libs = collect_libs(&cache, &client, &world);
        let deduped = libs.deduplicate();

        let rendered = deduped.render(' ');
        // diamond-d should appear exactly once after deduplication
        let count = rendered.matches("-ldiamond-d").count();
        assert_eq!(
            count, 1,
            "diamond-d should appear once after dedup, got {count} in: {rendered}"
        );
    }

    #[test]
    fn collect_libs_with_private_in_static() {
        let client = test_client_static();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(&mut cache, &client, &["private-deps".to_string()]).unwrap();
        let libs = collect_libs(&cache, &client, &world);

        let rendered = libs.render(' ');
        assert!(
            rendered.contains("-lprivate-deps"),
            "Expected -lprivate-deps in: {rendered}"
        );
        // Private libs should be included in static mode
        assert!(
            rendered.contains("-lm"),
            "Expected private lib -lm in: {rendered}"
        );
        assert!(
            rendered.contains("-lpthread"),
            "Expected private lib -lpthread in: {rendered}"
        );
    }

    #[test]
    fn collect_cflags_with_private_in_static() {
        let client = test_client_static();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(&mut cache, &client, &["private-deps".to_string()]).unwrap();
        let cflags = collect_cflags(&cache, &client, &world);

        let rendered = cflags.render(' ');
        assert!(
            rendered.contains("-DPRIVATE_INTERNAL"),
            "Expected -DPRIVATE_INTERNAL in: {rendered}"
        );
    }

    // ── Traversal tests ─────────────────────────────────────────────

    #[test]
    fn apply_visits_all_packages() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(&mut cache, &client, &["diamond-a".to_string()]).unwrap();

        let mut visited = Vec::new();
        apply(&cache, &client, &world, |pkg, _depth| {
            visited.push(pkg.id.clone());
            true
        });

        // Should include world + diamond-a, b, c, d
        assert!(visited.contains(&"virtual:world".to_string()));
        assert!(visited.contains(&"diamond-a".to_string()));
        assert!(visited.contains(&"diamond-b".to_string()));
        assert!(visited.contains(&"diamond-c".to_string()));
        assert!(visited.contains(&"diamond-d".to_string()));
    }

    #[test]
    fn apply_skip_root_virtual() {
        let client = ClientBuilder::new()
            .skip_env(true)
            .skip_default_paths(true)
            .with_path(test_data_dir())
            .flag(ClientFlags::SKIP_ROOT_VIRTUAL)
            .build();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(&mut cache, &client, &["simple".to_string()]).unwrap();

        let mut visited = Vec::new();
        apply(&cache, &client, &world, |pkg, _depth| {
            visited.push(pkg.id.clone());
            true
        });

        // World should not be in the visited list
        assert!(
            !visited.contains(&"virtual:world".to_string()),
            "World should be skipped: {visited:?}"
        );
        assert!(visited.contains(&"simple".to_string()));
    }

    #[test]
    fn apply_callback_can_skip_subtree() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(&mut cache, &client, &["deep-depender".to_string()]).unwrap();

        let mut visited = Vec::new();
        apply(&cache, &client, &world, |pkg, _depth| {
            visited.push(pkg.id.clone());
            // Stop descending at depender
            pkg.id != "depender"
        });

        assert!(visited.contains(&"deep-depender".to_string()));
        assert!(visited.contains(&"depender".to_string()));
        // simple should NOT be visited because we stopped at depender
        assert!(
            !visited.contains(&"simple".to_string()),
            "simple should not be visited: {visited:?}"
        );
    }

    // ── Modversion collection tests ─────────────────────────────────

    #[test]
    fn collect_modversions_single() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(&mut cache, &client, &["zlib".to_string()]).unwrap();
        let versions = collect_modversions(&cache, &world);

        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].0, "zlib");
        assert_eq!(versions[0].1, "1.2.13");
    }

    #[test]
    fn collect_modversions_multiple() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(
            &mut cache,
            &client,
            &["simple".to_string(), "zlib".to_string()],
        )
        .unwrap();
        let versions = collect_modversions(&cache, &world);

        assert_eq!(versions.len(), 2);
        let names: Vec<&str> = versions.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"simple"));
        assert!(names.contains(&"zlib"));
    }

    // ── Has uninstalled test ────────────────────────────────────────

    #[test]
    fn has_uninstalled_false_for_installed() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(&mut cache, &client, &["simple".to_string()]).unwrap();
        assert!(!has_uninstalled(&cache, &world));
    }

    // ── Solution tests ──────────────────────────────────────────────

    #[test]
    fn solution_lists_resolved_packages() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(&mut cache, &client, &["depender".to_string()]).unwrap();
        let sol = solution(&cache, &client, &world);

        let names: Vec<&str> = sol.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            names.contains(&"depender"),
            "Solution should include depender: {names:?}"
        );
        assert!(
            names.contains(&"simple"),
            "Solution should include simple: {names:?}"
        );
    }

    // ── Digraph tests ───────────────────────────────────────────────

    #[test]
    fn digraph_output() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(&mut cache, &client, &["depender".to_string()]).unwrap();
        let dot = digraph(&cache, &client, &world, false);

        assert!(dot.starts_with("digraph {\n"));
        assert!(dot.ends_with("}\n"));
        assert!(
            dot.contains("depender"),
            "dot output should mention depender: {dot}"
        );
        assert!(
            dot.contains("simple"),
            "dot output should mention simple: {dot}"
        );
        assert!(dot.contains("->"), "dot output should contain edges: {dot}");
    }

    #[test]
    fn digraph_with_query_nodes() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let world = solve(&mut cache, &client, &["simple".to_string()]).unwrap();
        let dot = digraph(&cache, &client, &world, true);

        assert!(
            dot.contains("virtual:world"),
            "dot output should include virtual:world: {dot}"
        );
    }

    // ── Validate tests ──────────────────────────────────────────────

    #[test]
    fn validate_valid_packages() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let mut q = Queue::new();
        q.push("zlib");
        q.push("simple");

        assert!(q.validate(&mut cache, &client).is_ok());
    }

    #[test]
    fn validate_missing_package() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let mut q = Queue::new();
        q.push("nonexistent-pkg");

        assert!(q.validate(&mut cache, &client).is_err());
    }

    #[test]
    fn validate_version_mismatch() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        let mut q = Queue::new();
        q.push("zlib >= 999.0");

        let result = q.validate(&mut cache, &client);
        assert!(result.is_err());
    }

    // ── Provider resolution tests ───────────────────────────────────

    #[test]
    fn solve_via_provider() {
        let client = test_client();
        let mut cache = Cache::with_builtins(&client);

        // provider.pc provides provider-alias
        // needs-provider.pc requires provider-alias
        let result = solve(&mut cache, &client, &["needs-provider".to_string()]);
        assert!(
            result.is_ok(),
            "Should resolve provider-alias via provider: {:?}",
            result.err()
        );
        assert!(cache.contains("provider"), "provider should be in cache");
    }

    // ── Depth limit tests ───────────────────────────────────────────

    #[test]
    fn solve_respects_max_depth() {
        let client = ClientBuilder::new()
            .skip_env(true)
            .skip_default_paths(true)
            .with_path(test_data_dir())
            .max_traversal_depth(0)
            .build();
        let mut cache = Cache::with_builtins(&client);

        // deep-depender -> depender -> simple, which requires depth > 0
        let result = solve(&mut cache, &client, &["deep-depender".to_string()]);
        assert!(result.is_err(), "Should fail with depth 0");
    }
}
