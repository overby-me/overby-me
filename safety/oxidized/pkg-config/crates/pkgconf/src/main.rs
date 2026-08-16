//! `pkgconf` — A Rust drop-in replacement for pkg-config/pkgconf.
//!
//! This binary provides a command-line interface compatible with both
//! `pkg-config` and `pkgconf`, implementing all standard flags and
//! environment variable handling.

use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser;

use libpkgconf::audit::AuditLog;
use libpkgconf::client::{Client, ClientFlags};
use libpkgconf::personality::CrossPersonality;
use libpkgconf::version as ver;
use libpkgconf::{PKGCONFIG_COMPAT_VERSION, VERSION};

/// A package compiler and linker metadata toolkit.
///
/// pkgconf retrieves information about installed libraries and packages,
/// providing compiler and linker flags needed to use them.
#[derive(Parser, Debug)]
#[command(
    name = "pkgconf",
    about = "package compiler and linker metadata toolkit",
    long_about = None,
    disable_help_flag = false,
    disable_version_flag = true,
    args_override_self = true,
    trailing_var_arg = true,
)]
struct Cli {
    // ── Basic options ────────────────────────────────────────────────
    /// Print pkgconf version and license information.
    #[arg(long)]
    about: bool,

    /// Print supported pkg-config version.
    #[arg(long)]
    version: bool,

    /// Print additional information.
    #[arg(long)]
    verbose: bool,

    /// Check whether pkgconf is compatible with a specified pkg-config version.
    #[arg(long = "atleast-pkgconfig-version", value_name = "VERSION")]
    atleast_pkgconfig_version: Option<String>,

    /// Print all errors on stdout instead of stderr.
    #[arg(long = "errors-to-stdout")]
    errors_to_stdout: bool,

    /// Ensure all errors are printed.
    #[arg(long = "print-errors")]
    print_errors: bool,

    /// Be less verbose about some errors.
    #[arg(long = "short-errors")]
    short_errors: bool,

    /// Explicitly silence errors.
    #[arg(long = "silence-errors")]
    silence_errors: bool,

    /// List all known packages.
    #[arg(long = "list-all")]
    list_all: bool,

    /// List all known package names (one per line, no description).
    #[arg(long = "list-package-names")]
    list_package_names: bool,

    /// Simulate walking the calculated dependency graph.
    #[arg(long)]
    simulate: bool,

    /// Do not cache already seen packages when walking the dependency graph.
    #[arg(long = "no-cache")]
    no_cache: bool,

    /// Write an audit log to a specified file.
    #[arg(long = "log-file", value_name = "FILENAME")]
    log_file: Option<String>,

    /// Add a directory to the search path.
    #[arg(long = "with-path", value_name = "PATH")]
    with_path: Vec<String>,

    /// Override the prefix variable with one guessed from the .pc file location.
    #[arg(long = "define-prefix")]
    define_prefix: bool,

    /// Do not override the prefix variable under any circumstances.
    #[arg(long = "dont-define-prefix")]
    dont_define_prefix: bool,

    /// Set the name of the variable considered to be the package prefix.
    #[arg(long = "prefix-variable", value_name = "VARNAME")]
    prefix_variable: Option<String>,

    /// Relocate a path and exit (mostly for testsuite).
    #[arg(long, value_name = "PATH")]
    relocate: Option<String>,

    /// Disable path relocation support.
    #[arg(long = "dont-relocate-paths")]
    dont_relocate_paths: bool,

    // ── Cross-compilation personality ────────────────────────────────
    /// Set the cross-compilation personality.
    #[arg(long, value_name = "TRIPLET")]
    personality: Option<String>,

    /// Dump details concerning the selected personality.
    #[arg(long = "dump-personality")]
    dump_personality: bool,

    // ── Version checking ────────────────────────────────────────────
    /// Require a minimum version of a module.
    #[arg(long = "atleast-version", value_name = "VERSION")]
    atleast_version: Option<String>,

    /// Require an exact version of a module.
    #[arg(long = "exact-version", value_name = "VERSION")]
    exact_version: Option<String>,

    /// Require a maximum version of a module.
    #[arg(long = "max-version", value_name = "VERSION")]
    max_version: Option<String>,

    /// Check whether or not a module exists.
    #[arg(long)]
    exists: bool,

    /// Check whether or not an uninstalled module will be used.
    #[arg(long)]
    uninstalled: bool,

    /// Never use uninstalled modules when satisfying dependencies.
    #[arg(long = "no-uninstalled")]
    no_uninstalled: bool,

    /// Do not use 'provides' rules to resolve dependencies.
    #[arg(long = "no-provides")]
    no_provides: bool,

    /// Maximum allowed depth for dependency graph.
    #[arg(long = "maximum-traverse-depth", value_name = "DEPTH")]
    maximum_traverse_depth: Option<i32>,

    /// Be more aggressive when computing dependency graph (for static linking).
    #[arg(long = "static")]
    r#static: bool,

    /// Use a simplified dependency graph (usually default).
    #[arg(long)]
    shared: bool,

    /// Optimize a static dependency graph as if it were a normal dependency graph.
    #[arg(long)]
    pure: bool,

    /// Look only for package entries in PKG_CONFIG_PATH.
    #[arg(long = "env-only")]
    env_only: bool,

    /// Ignore 'conflicts' rules in modules.
    #[arg(long = "ignore-conflicts")]
    ignore_conflicts: bool,

    /// Validate specific .pc files for correctness.
    #[arg(long)]
    validate: bool,

    // ── Querying fields ─────────────────────────────────────────────
    /// Define variable 'varname' as 'value' (format: varname=value).
    #[arg(long = "define-variable", value_name = "VARNAME=VALUE")]
    define_variable: Vec<String>,

    /// Print specified variable entry.
    #[arg(long = "variable", value_name = "VARNAME")]
    variable: Option<String>,

    /// Print required CFLAGS.
    #[arg(long)]
    cflags: bool,

    /// Print required include-dir CFLAGS only.
    #[arg(long = "cflags-only-I")]
    cflags_only_i: bool,

    /// Print required non-include-dir CFLAGS only.
    #[arg(long = "cflags-only-other")]
    cflags_only_other: bool,

    /// Print required linker flags.
    #[arg(long)]
    libs: bool,

    /// Print required LDPATH linker flags only.
    #[arg(long = "libs-only-L")]
    libs_only_l_upper: bool,

    /// Print required LIBNAME linker flags only.
    #[arg(long = "libs-only-l")]
    libs_only_l_lower: bool,

    /// Print required other linker flags only.
    #[arg(long = "libs-only-other")]
    libs_only_other: bool,

    /// Print required dependency frameworks.
    #[arg(long = "print-requires")]
    print_requires: bool,

    /// Print required dependency frameworks for static linking.
    #[arg(long = "print-requires-private")]
    print_requires_private: bool,

    /// Print provided dependencies.
    #[arg(long = "print-provides")]
    print_provides: bool,

    /// Print all known variables in module.
    #[arg(long = "print-variables")]
    print_variables: bool,

    /// Print entire dependency graph in graphviz 'dot' format.
    #[arg(long)]
    digraph: bool,

    /// Also print query nodes in 'dot' format.
    #[arg(long = "print-digraph-query-nodes")]
    print_digraph_query_nodes: bool,

    /// Print dependency graph solution in a simple format.
    #[arg(long)]
    solution: bool,

    /// Keep system cflags (e.g. -I/usr/include) in output.
    #[arg(long = "keep-system-cflags")]
    keep_system_cflags: bool,

    /// Keep system libs (e.g. -L/usr/lib) in output.
    #[arg(long = "keep-system-libs")]
    keep_system_libs: bool,

    /// Show the exact filenames for any matching .pc files.
    #[arg(long)]
    path: bool,

    /// Print the specified module's version.
    #[arg(long)]
    modversion: bool,

    /// Do not filter 'internal' cflags from output.
    #[arg(long = "internal-cflags")]
    internal_cflags: bool,

    /// Print the specified module's license if known.
    #[arg(long)]
    license: bool,

    /// Print the specified module's source code location if known.
    #[arg(long)]
    source: bool,

    /// Add -DHAVE_FOO fragments to cflags for each found module.
    #[arg(long = "exists-cflags")]
    exists_cflags: bool,

    // ── Output filtering ────────────────────────────────────────────
    /// Print translatable fragments in MSVC syntax.
    #[arg(long = "msvc-syntax")]
    msvc_syntax: bool,

    /// Filter output fragments to the specified types.
    #[arg(long = "fragment-filter", value_name = "TYPES")]
    fragment_filter: Option<String>,

    /// Print output as shell-compatible environmental variables.
    #[arg(long = "env", value_name = "PREFIX")]
    env: Option<String>,

    /// Visualize printed CFLAGS/LIBS fragments as a tree.
    #[arg(long = "fragment-tree")]
    fragment_tree: bool,

    /// Use newlines for whitespace between fragments.
    #[arg(long)]
    newlines: bool,

    /// Enable debug output.
    #[arg(long)]
    debug: bool,

    // ── Positional ──────────────────────────────────────────────────
    /// Package names (and optional version constraints) to query.
    #[arg(trailing_var_arg = true)]
    packages: Vec<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Only print the error if we're not silencing errors
            if !cli.silence_errors {
                if cli.errors_to_stdout {
                    println!("{e:#}");
                } else {
                    eprintln!("{e:#}");
                }
            }
            ExitCode::FAILURE
        }
    }
}

/// Resolve a personality: from --personality flag, argv[0] deduction, or default.
fn resolve_personality(cli: &Cli) -> CrossPersonality {
    // Explicit --personality flag takes priority
    if let Some(ref triplet) = cli.personality {
        if let Some(p) = CrossPersonality::find(triplet) {
            return p;
        }
        // If no personality file found, create a minimal one with the given name
        return CrossPersonality::new(triplet);
    }

    // Try deducing from argv[0]
    if let Some(argv0) = std::env::args().next()
        && let Some(p) = CrossPersonality::from_argv0(&argv0)
    {
        return p;
    }

    // Fall back to the default (native) personality
    CrossPersonality::default_personality()
}

/// Build a [`Client`] from CLI arguments, personality, and environment.
fn build_client(cli: &Cli, personality: &CrossPersonality) -> Result<Client> {
    let mut builder = Client::builder();

    // --with-path (highest priority search paths)
    for p in &cli.with_path {
        builder = builder.with_path(p);
    }

    // --define-variable
    for def in &cli.define_variable {
        if let Some((key, value)) = def.split_once('=') {
            builder = builder.define_variable(key, value);
        } else {
            bail!("Invalid --define-variable format: '{def}' (expected varname=value)");
        }
    }

    // --keep-system-cflags
    if cli.keep_system_cflags {
        builder = builder.keep_system_cflags(true);
    }

    // --keep-system-libs
    if cli.keep_system_libs {
        builder = builder.keep_system_libs(true);
    }

    // --static
    if cli.r#static || personality.want_default_static {
        builder = builder.enable_static(true);
    }

    // --shared (disables static)
    if cli.shared {
        builder = builder.enable_static(false);
    }

    // --pure
    if cli.pure || personality.want_default_pure {
        builder = builder.pure(true);
    }

    // --env-only
    if cli.env_only {
        builder = builder.flag(ClientFlags::ENV_ONLY);
    }

    // --no-cache
    if cli.no_cache {
        builder = builder.flag(ClientFlags::NO_CACHE);
    }

    // --no-uninstalled
    if cli.no_uninstalled {
        builder = builder.flag(ClientFlags::NO_UNINSTALLED);
    }

    // --no-provides
    if cli.no_provides {
        builder = builder.flag(ClientFlags::SKIP_PROVIDES);
    }

    // --ignore-conflicts
    if cli.ignore_conflicts {
        builder = builder.flag(ClientFlags::IGNORE_CONFLICTS);
    }

    // --define-prefix
    if cli.define_prefix {
        builder = builder.flag(ClientFlags::DEFINE_PREFIX);
    }

    // --dont-define-prefix
    if cli.dont_define_prefix {
        builder = builder.flag(ClientFlags::DONT_DEFINE_PREFIX);
    }

    // --dont-relocate-paths
    if cli.dont_relocate_paths {
        builder = builder.flag(ClientFlags::DONT_RELOCATE_PATHS);
    }

    // --msvc-syntax
    if cli.msvc_syntax {
        builder = builder.flag(ClientFlags::MSVC_SYNTAX);
    }

    // --prefix-variable
    if let Some(ref var) = cli.prefix_variable {
        builder = builder.prefix_variable(var);
    }

    // --maximum-traverse-depth
    if let Some(depth) = cli.maximum_traverse_depth {
        builder = builder.max_traversal_depth(depth);
    }

    // --log-file
    if let Some(ref path) = cli.log_file {
        builder = builder.log_file(path);
    }

    // --debug
    if cli.debug {
        builder = builder.debug(true);
    }

    // Apply personality sysroot if set (can be overridden by env)
    if let Some(ref sysroot) = personality.sysroot_dir {
        builder = builder.sysroot_dir(sysroot);
    }

    // Apply personality filter dirs (the builder defaults are used unless
    // the personality specifies them)
    if !personality.filter_libdirs.is_empty() {
        let dirs: Vec<String> = personality
            .filter_libdirs
            .dirs()
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        builder = builder.filter_libdirs(dirs);
    }

    if !personality.filter_includedirs.is_empty() {
        let dirs: Vec<String> = personality
            .filter_includedirs
            .dirs()
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        builder = builder.filter_includedirs(dirs);
    }

    Ok(builder.build())
}

/// Apply sysroot to a single path string for --relocate.
fn relocate_path(path: &str, sysroot: Option<&str>) -> String {
    match sysroot {
        Some(sr) if !sr.is_empty() && path.starts_with('/') && !path.starts_with(sr) => {
            format!("{sr}{path}")
        }
        _ => path.to_string(),
    }
}

/// Load preloaded packages from `PKG_CONFIG_PRELOADED_FILES` environment variable.
fn load_preloaded_packages(cache: &mut libpkgconf::cache::Cache, client: &Client) {
    if let Ok(preloaded) = std::env::var(libpkgconf::ENV_PKG_CONFIG_PRELOADED_FILES) {
        for path_str in preloaded.split(libpkgconf::path::PATH_SEPARATOR) {
            let path_str = path_str.trim();
            if path_str.is_empty() {
                continue;
            }
            let path = std::path::Path::new(path_str);
            if !path.is_file() {
                continue;
            }
            if let Ok(pc) = libpkgconf::parser::PcFile::from_path(path) {
                let id = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if id.is_empty() {
                    continue;
                }
                if let Ok(pkg) = libpkgconf::pkg::Package::from_pc_file(client, &pc, &id) {
                    cache.add(pkg);
                }
            }
        }
    }
}

/// Render a fragment tree for visualization.
fn render_fragment_tree(
    label: &str,
    frags: &libpkgconf::fragment::FragmentList,
    msvc: bool,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("{label}:\n"));

    for (i, frag) in frags.iter().enumerate() {
        let is_last = i + 1 == frags.len();
        let prefix = if is_last { "└── " } else { "├── " };
        let rendered = if msvc {
            frag.render_msvc()
        } else {
            frag.render()
        };

        let type_label = match frag.frag_type() {
            Some('I') => "include",
            Some('L') => "libpath",
            Some('l') => "libname",
            Some('D') => "define",
            Some('U') => "undef",
            Some('W') => "warning",
            Some(c) => {
                out.push_str(&format!("{prefix}[flag({c})] {rendered}\n"));
                continue;
            }
            None => "other",
        };

        out.push_str(&format!("{prefix}[{type_label}] {rendered}\n"));
    }

    out
}

fn run(cli: &Cli) -> Result<()> {
    // --about
    if cli.about {
        print_about();
        return Ok(());
    }

    // --version
    if cli.version {
        println!("{PKGCONFIG_COMPAT_VERSION}");
        return Ok(());
    }

    // --atleast-pkgconfig-version
    if let Some(ref required) = cli.atleast_pkgconfig_version {
        if ver::compare(PKGCONFIG_COMPAT_VERSION, required) >= 0 {
            return Ok(());
        } else {
            bail!(
                "pkg-config compatibility version {PKGCONFIG_COMPAT_VERSION} is less than required {required}"
            );
        }
    }

    // Resolve personality early (needed for sysroot in --relocate and build_client)
    let personality = resolve_personality(cli);

    // --relocate: apply sysroot relocation to a path and exit
    if let Some(ref path) = cli.relocate {
        // Get sysroot from env or personality
        let sysroot_env = std::env::var(libpkgconf::ENV_PKG_CONFIG_SYSROOT_DIR)
            .ok()
            .filter(|s| !s.is_empty());
        let effective_sysroot = sysroot_env
            .as_deref()
            .or(personality.sysroot_dir.as_deref())
            .unwrap_or("");

        let relocated = relocate_path(path, Some(effective_sysroot));
        println!("{relocated}");
        return Ok(());
    }

    // Build the client from CLI + personality + environment
    let client = build_client(cli, &personality)?;

    // Open audit log if configured
    let audit = client.log_file().and_then(|p| AuditLog::open(p).ok());

    // --dump-personality
    if cli.dump_personality {
        // Build an effective personality reflecting CLI overrides
        let effective = CrossPersonality {
            name: personality.name.clone(),
            dir_list: {
                // Use the client's resolved dir_list which includes env and CLI paths
                let mut sp = libpkgconf::path::SearchPath::new();
                for d in client.dir_list().dirs() {
                    sp.add(d.clone());
                }
                sp
            },
            filter_libdirs: {
                let mut sp = libpkgconf::path::SearchPath::new();
                for d in client.filter_libdirs().dirs() {
                    sp.add(d.clone());
                }
                sp
            },
            filter_includedirs: {
                let mut sp = libpkgconf::path::SearchPath::new();
                for d in client.filter_includedirs().dirs() {
                    sp.add(d.clone());
                }
                sp
            },
            sysroot_dir: client.sysroot_dir().map(|s| s.to_string()),
            want_default_static: client.is_static(),
            want_default_pure: client.flags().contains(ClientFlags::PURE_DEPGRAPH),
        };
        print!("{}", effective.dump());
        return Ok(());
    }

    // --list-all
    if cli.list_all {
        let all = client.list_all();
        for (name, description, version) in &all {
            let desc = if description.is_empty() {
                "No description available"
            } else {
                description
            };
            if version.is_empty() {
                println!("{name:40} {desc}");
            } else {
                println!("{name:40} {desc} - {version}");
            }
        }
        return Ok(());
    }

    // --list-package-names
    if cli.list_package_names {
        let names = client.list_package_names();
        for name in &names {
            println!("{name}");
        }
        return Ok(());
    }

    // All other operations require at least one package name
    if cli.packages.is_empty() {
        bail!("Please specify at least one package name on the command line");
    }

    // Build a queue from positional args and apply version constraint overrides
    let mut queue = libpkgconf::queue::Queue::new();

    // Build the combined query string from positional args
    let query = cli.packages.join(" ");

    // Parse as dependencies for version constraint handling
    let mut deps = libpkgconf::dependency::DependencyList::parse(&query);

    // Apply --atleast-version / --exact-version / --max-version to unconstrained deps
    if let Some(ref ver_str) = cli.atleast_version {
        for dep in deps.entries_mut() {
            if dep.compare == libpkgconf::version::Comparator::Any {
                dep.compare = libpkgconf::version::Comparator::GreaterThanEqual;
                dep.version = Some(ver_str.clone());
            }
        }
    }
    if let Some(ref ver_str) = cli.exact_version {
        for dep in deps.entries_mut() {
            if dep.compare == libpkgconf::version::Comparator::Any {
                dep.compare = libpkgconf::version::Comparator::Equal;
                dep.version = Some(ver_str.clone());
            }
        }
    }
    if let Some(ref ver_str) = cli.max_version {
        for dep in deps.entries_mut() {
            if dep.compare == libpkgconf::version::Comparator::Any {
                dep.compare = libpkgconf::version::Comparator::LessThanEqual;
                dep.version = Some(ver_str.clone());
            }
        }
    }

    // Push the (possibly modified) dependencies into the queue
    for dep in deps.iter() {
        queue.push_dependency(dep);
    }

    // Initialize the cache with built-in virtual packages
    let mut cache = libpkgconf::cache::Cache::with_builtins(&client);

    // Load preloaded packages from PKG_CONFIG_PRELOADED_FILES
    load_preloaded_packages(&mut cache, &client);

    // Log the solve start
    if let Some(ref log) = audit {
        log.log_solve_start(&cli.packages);
    }

    // --validate: just check that the top-level packages parse and load OK
    if cli.validate {
        let result = queue
            .validate(&mut cache, &client)
            .with_context(|| "Validation failed");

        if let Some(ref log) = audit {
            log.log_solve_end(result.is_ok());
        }

        return result;
    }

    // Solve the full dependency graph (recursive resolution, version checks,
    // conflict detection).
    let world = match queue.solve(&mut cache, &client) {
        Ok(w) => {
            if let Some(ref log) = audit {
                log.log_solve_end(true);
            }
            w
        }
        Err(e) => {
            if let Some(ref log) = audit {
                log.log_solve_end(false);
            }
            return Err(e).with_context(|| {
                format!("Failed to resolve package(s): {}", cli.packages.join(", "))
            });
        }
    };

    // Log resolved packages
    if let Some(ref log) = audit {
        let sol = libpkgconf::queue::solution(&cache, &client, &world);
        for (name, version) in &sol {
            log.log_found(name, version, None);
        }
    }

    // --exists / --atleast-version / --exact-version / --max-version:
    // If only checking existence/version, the fact that solve() succeeded
    // means all constraints are satisfied.
    if cli.exists
        || cli.atleast_version.is_some()
        || cli.exact_version.is_some()
        || cli.max_version.is_some()
    {
        return Ok(());
    }

    // --uninstalled: check if any resolved package is uninstalled
    if cli.uninstalled {
        if libpkgconf::queue::has_uninstalled(&cache, &world) {
            return Ok(());
        } else {
            bail!("None of the requested packages are uninstalled");
        }
    }

    // Per-package metadata queries: these iterate over the top-level
    // packages (not the full transitive graph) since the user asked
    // about specific packages.
    if cli.modversion
        || cli.variable.is_some()
        || cli.print_variables
        || cli.print_requires
        || cli.print_requires_private
        || cli.print_provides
        || cli.path
        || cli.license
        || cli.source
    {
        for dep in world.requires.iter() {
            let pkg = cache.lookup_unchecked(&dep.package);
            // Try provider resolution if direct lookup fails
            let pkg = pkg.or_else(|| cache.lookup_provider(&dep.package, &client));
            let pkg = match pkg {
                Some(p) => p,
                None => continue,
            };

            // --modversion
            if cli.modversion {
                if cli.verbose {
                    print!("{}: ", pkg.display_name());
                }
                println!("{}", pkg.version);
                continue;
            }

            // --variable
            if let Some(ref var_name) = cli.variable {
                if let Some(val) = pkg.get_variable(var_name) {
                    print!("{val}");
                }
                println!();
                continue;
            }

            // --print-variables
            if cli.print_variables {
                for name in pkg.variable_names() {
                    println!("{name}");
                }
                continue;
            }

            // --print-requires
            if cli.print_requires {
                for d in pkg.requires.iter() {
                    println!("{d}");
                }
                continue;
            }

            // --print-requires-private
            if cli.print_requires_private {
                for d in pkg.requires_private.iter() {
                    println!("{d}");
                }
                continue;
            }

            // --print-provides
            if cli.print_provides {
                for d in pkg.provides.iter() {
                    println!("{d}");
                }
                continue;
            }

            // --path
            if cli.path {
                if let Some(ref p) = pkg.filename {
                    println!("{}", p.display());
                }
                continue;
            }

            // --license
            if cli.license {
                let lic = pkg.license.as_deref().unwrap_or("NOASSERTION");
                println!("{}: {lic}", pkg.display_name());
                continue;
            }

            // --source
            if cli.source {
                let src = pkg.source.as_deref().unwrap_or("");
                println!("{}: {src}", pkg.display_name());
                continue;
            }
        }
        return Ok(());
    }

    // --simulate / --solution: print the resolved dependency list
    if cli.simulate || cli.solution {
        let sol = libpkgconf::queue::solution(&cache, &client, &world);
        for (name, version) in &sol {
            println!("{name} {version}");
        }
        return Ok(());
    }

    // --digraph: output dependency graph in graphviz dot format
    if cli.digraph {
        let dot =
            libpkgconf::queue::digraph(&cache, &client, &world, cli.print_digraph_query_nodes);
        print!("{dot}");
        return Ok(());
    }

    // Determine whether to use MSVC syntax
    let use_msvc = cli.msvc_syntax || client.flags().contains(ClientFlags::MSVC_SYNTAX);

    // System filter dirs from the client
    let system_libdirs = client.system_libdirs();
    let system_includedirs = client.system_includedirs();

    // Sysroot for fragment-level application
    let sysroot = client.sysroot_dir().map(|s| s.to_string());
    let use_fdo_sysroot = client.flags().contains(ClientFlags::FDO_SYSROOT_RULES);

    let delim = if cli.newlines { '\n' } else { ' ' };

    // --env output mode: collect flags and print as shell variables
    if let Some(ref env_prefix) = cli.env {
        let mut cflags = libpkgconf::queue::collect_cflags(&cache, &client, &world);
        let mut libs = libpkgconf::queue::collect_libs(&cache, &client, &world);

        // Apply sysroot to fragments if needed
        if let Some(ref sr) = sysroot
            && use_fdo_sysroot
        {
            cflags.apply_sysroot_fdo(sr);
            libs.apply_sysroot_fdo(sr);
        }
        // Note: non-FDO sysroot is already applied at the variable level

        // Filter system dirs
        if !client.keep_system_cflags() {
            cflags = cflags.filter_system_dirs(&system_libdirs, &system_includedirs);
        }
        if !client.keep_system_libs() {
            libs = libs.filter_system_dirs(&system_libdirs, &system_includedirs);
        }

        // Deduplicate
        let cflags = cflags.deduplicate();
        let libs = libs.deduplicate();

        // Render
        let cflags_str = if use_msvc {
            cflags.render_msvc_escaped(' ')
        } else {
            cflags.render_escaped(' ')
        };
        let libs_str = if use_msvc {
            libs.render_msvc_escaped(' ')
        } else {
            libs.render_escaped(' ')
        };

        let prefix = env_prefix.to_uppercase().replace('-', "_");
        println!("{prefix}_CFLAGS='{cflags_str}'");
        println!("{prefix}_LIBS='{libs_str}'");

        if let Some(ref log) = audit {
            log.log_flags("CFLAGS", &cflags_str);
            log.log_flags("LIBS", &libs_str);
        }

        return Ok(());
    }

    // --fragment-tree: visualize fragments as a tree
    if cli.fragment_tree {
        let want_cflags = cli.cflags
            || cli.cflags_only_i
            || cli.cflags_only_other
            || (!cli.libs
                && !cli.libs_only_l_upper
                && !cli.libs_only_l_lower
                && !cli.libs_only_other);
        let want_libs = cli.libs
            || cli.libs_only_l_upper
            || cli.libs_only_l_lower
            || cli.libs_only_other
            || (!cli.cflags && !cli.cflags_only_i && !cli.cflags_only_other);

        if want_cflags {
            let mut cflags = libpkgconf::queue::collect_cflags(&cache, &client, &world);
            if let Some(ref sr) = sysroot
                && use_fdo_sysroot
            {
                cflags.apply_sysroot_fdo(sr);
            }
            if !client.keep_system_cflags() {
                cflags = cflags.filter_system_dirs(&system_libdirs, &system_includedirs);
            }
            let cflags = cflags.deduplicate();
            print!("{}", render_fragment_tree("CFLAGS", &cflags, use_msvc));
        }

        if want_libs {
            let mut libs = libpkgconf::queue::collect_libs(&cache, &client, &world);
            if let Some(ref sr) = sysroot
                && use_fdo_sysroot
            {
                libs.apply_sysroot_fdo(sr);
            }
            if !client.keep_system_libs() {
                libs = libs.filter_system_dirs(&system_libdirs, &system_includedirs);
            }
            let libs = libs.deduplicate();
            print!("{}", render_fragment_tree("LIBS", &libs, use_msvc));
        }

        return Ok(());
    }

    // Build output fragments
    let mut output_frags = libpkgconf::fragment::FragmentList::new();

    // Collect and process cflags from the full dependency graph
    if cli.cflags || cli.cflags_only_i || cli.cflags_only_other {
        let mut cflags = libpkgconf::queue::collect_cflags(&cache, &client, &world);

        // Apply FDO sysroot rules to fragments
        if let Some(ref sr) = sysroot
            && use_fdo_sysroot
        {
            cflags.apply_sysroot_fdo(sr);
        }

        // Filter system dirs unless --keep-system-cflags
        if !client.keep_system_cflags() {
            cflags = cflags.filter_system_dirs(&system_libdirs, &system_includedirs);
        }

        // Apply fragment-filter
        if let Some(ref filter) = cli.fragment_filter {
            cflags = cflags.filter_by_types(filter);
        }

        // Sub-filter by type
        if cli.cflags_only_i {
            cflags = cflags.filter_cflags_only_i();
        } else if cli.cflags_only_other {
            cflags = cflags.filter_cflags_only_other();
        }

        output_frags.append(&cflags);
    }

    // Collect and process libs from the full dependency graph
    if cli.libs || cli.libs_only_l_upper || cli.libs_only_l_lower || cli.libs_only_other {
        let mut libs = libpkgconf::queue::collect_libs(&cache, &client, &world);

        // Apply FDO sysroot rules to fragments
        if let Some(ref sr) = sysroot
            && use_fdo_sysroot
        {
            libs.apply_sysroot_fdo(sr);
        }

        // Filter system dirs unless --keep-system-libs
        if !client.keep_system_libs() {
            libs = libs.filter_system_dirs(&system_libdirs, &system_includedirs);
        }

        // Apply fragment-filter
        if let Some(ref filter) = cli.fragment_filter {
            libs = libs.filter_by_types(filter);
        }

        // Sub-filter by type
        if cli.libs_only_l_upper {
            libs = libs.filter_libs_only_ldpath();
        } else if cli.libs_only_l_lower {
            libs = libs.filter_libs_only_libname();
        } else if cli.libs_only_other {
            libs = libs.filter_libs_only_other();
        }

        output_frags.append(&libs);
    }

    // --exists-cflags: add -DHAVE_<MODULE> for each found module
    if cli.exists_cflags {
        for dep in world.requires.iter() {
            let define_name = dep.package.to_uppercase().replace('-', "_");
            let frag = libpkgconf::fragment::Fragment::new('D', format!("HAVE_{define_name}"));
            output_frags.push(frag);
        }
    }

    // Deduplicate
    let output_frags = output_frags.deduplicate();

    // Render and print
    if !output_frags.is_empty() {
        let rendered = if use_msvc {
            output_frags.render_msvc_escaped(delim)
        } else {
            output_frags.render_escaped(delim)
        };

        // Log the output
        if let Some(ref log) = audit {
            let flag_type = if cli.cflags || cli.cflags_only_i || cli.cflags_only_other {
                "CFLAGS"
            } else if cli.libs
                || cli.libs_only_l_upper
                || cli.libs_only_l_lower
                || cli.libs_only_other
            {
                "LIBS"
            } else {
                "OUTPUT"
            };
            log.log_flags(flag_type, &rendered);
        }

        println!("{rendered}");
    }

    Ok(())
}

fn print_about() {
    println!("pkgconf (rust-pkg-config) {VERSION}");
    println!("Copyright (c) 2025 rust-pkg-config authors.");
    println!();
    println!("Permission to use, copy, modify, and/or distribute this software for any");
    println!("purpose with or without fee is hereby granted, provided that the above");
    println!("copyright notice and this permission notice appear in all copies.");
    println!();
    println!("This software is provided 'as is' and without any warranty, express or");
    println!("implied. In no event shall the authors be liable for any damages arising");
    println!("from the use of this software.");
    println!();
    println!("Report bugs at https://tangled.org/overby.me/overby.me/tree/main/rust-pkg-config");
}
