//! Performance benchmarks for libpkgconf.
//!
//! Run with: cargo bench -p libpkgconf
//!
//! Covers:
//! - .pc file parsing
//! - Variable expansion / resolution
//! - Version comparison
//! - Fragment parsing, deduplication, and filtering
//! - Dependency list parsing
//! - Solver (queue-based resolution)
//! - Cache lookup
//! - Search path operations

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use libpkgconf::cache::Cache;
use libpkgconf::client::{Client, ClientBuilder, ClientFlags};
use libpkgconf::dependency::DependencyList;
use libpkgconf::fragment::FragmentList;
use libpkgconf::parser::{self, PcFile};
use libpkgconf::path::SearchPath;
use libpkgconf::pkg::Package;
use libpkgconf::queue::Queue;
use libpkgconf::version;

/// Returns the absolute path to the workspace-level `tests/data/` directory.
fn test_data_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    workspace_root.join("tests").join("data")
}

fn make_test_client() -> Client {
    ClientBuilder::new()
        .with_path(test_data_dir().to_str().unwrap())
        .skip_env(true)
        .skip_default_paths(true)
        .build()
}

// ============================================================================
// Parser benchmarks
// ============================================================================

fn bench_parse_simple_pc(c: &mut Criterion) {
    let path = test_data_dir().join("simple.pc");

    c.bench_function("parse/simple.pc", |b| {
        b.iter(|| {
            let pc = PcFile::from_path(black_box(&path)).unwrap();
            black_box(pc);
        });
    });
}

fn bench_parse_libbar_pc(c: &mut Criterion) {
    let path = test_data_dir().join("libbar.pc");

    c.bench_function("parse/libbar.pc (complex)", |b| {
        b.iter(|| {
            let pc = PcFile::from_path(black_box(&path)).unwrap();
            black_box(pc);
        });
    });
}

fn bench_parse_zlib_pc(c: &mut Criterion) {
    let path = test_data_dir().join("zlib.pc");

    c.bench_function("parse/zlib.pc", |b| {
        b.iter(|| {
            let pc = PcFile::from_path(black_box(&path)).unwrap();
            black_box(pc);
        });
    });
}

fn bench_parse_from_string(c: &mut Criterion) {
    let content = "\
prefix=/usr/local
exec_prefix=${prefix}
libdir=${exec_prefix}/lib
includedir=${prefix}/include
datarootdir=${prefix}/share

Name: benchmark-lib
Description: A library used for benchmarking the parser
URL: https://example.com/benchmark
Version: 3.14.159
Requires: dep-a >= 1.0, dep-b
Requires.private: dep-c, dep-d >= 2.0
Conflicts: old-benchmark < 2.0
Provides: benchmark-compat = 3.14
Libs: -L${libdir} -lbenchmark -lhelper
Libs.private: -lm -lpthread -ldl
Cflags: -I${includedir}/benchmark -DBENCHMARK_VERSION=314 -DNDEBUG
Cflags.private: -DBENCHMARK_INTERNAL
";

    let source_path = Path::new("/usr/lib/pkgconfig/benchmark.pc");

    c.bench_function("parse/from_string (complex)", |b| {
        b.iter(|| {
            let pc = PcFile::from_str(black_box(content), source_path).unwrap();
            black_box(pc);
        });
    });
}

fn bench_parse_many_variables(c: &mut Criterion) {
    let mut content = String::with_capacity(10_000);
    for i in 0..100 {
        content.push_str(&format!("var{i}=value_{i}\n"));
    }
    content.push_str("computed=${var0}/${var1}/${var2}/${var3}/${var4}\n");
    content.push_str("\nName: many-vars\nDescription: test\nVersion: 1.0.0\n");
    content.push_str("Cflags: -I${computed}/include\n");

    let source_path = Path::new("/tmp/many-vars.pc");

    c.bench_function("parse/100_variables", |b| {
        b.iter(|| {
            let pc = PcFile::from_str(black_box(&content), source_path).unwrap();
            black_box(pc);
        });
    });
}

// ============================================================================
// Variable resolution benchmarks
// ============================================================================

fn bench_resolve_variables_simple(c: &mut Criterion) {
    let path = test_data_dir().join("simple.pc");
    let pc = PcFile::from_path(&path).unwrap();
    let global_vars = HashMap::new();

    c.bench_function("resolve/variables_simple", |b| {
        b.iter(|| {
            let resolved = parser::resolve_variables(black_box(&pc), &global_vars, None).unwrap();
            black_box(resolved);
        });
    });
}

fn bench_resolve_variables_complex(c: &mut Criterion) {
    let path = test_data_dir().join("libbar.pc");
    let pc = PcFile::from_path(&path).unwrap();
    let global_vars = HashMap::new();

    c.bench_function("resolve/variables_complex", |b| {
        b.iter(|| {
            let resolved = parser::resolve_variables(black_box(&pc), &global_vars, None).unwrap();
            black_box(resolved);
        });
    });
}

fn bench_resolve_variables_with_overrides(c: &mut Criterion) {
    let path = test_data_dir().join("simple.pc");
    let pc = PcFile::from_path(&path).unwrap();
    let mut global_vars = HashMap::new();
    global_vars.insert("prefix".to_string(), "/custom/path".to_string());

    c.bench_function("resolve/variables_with_override", |b| {
        b.iter(|| {
            let resolved = parser::resolve_variables(black_box(&pc), &global_vars, None).unwrap();
            black_box(resolved);
        });
    });
}

// ============================================================================
// Version comparison benchmarks
// ============================================================================

fn bench_version_compare_equal(c: &mut Criterion) {
    c.bench_function("version/compare_equal", |b| {
        b.iter(|| {
            let r = version::compare(black_box("1.2.3"), black_box("1.2.3"));
            black_box(r);
        });
    });
}

fn bench_version_compare_numeric(c: &mut Criterion) {
    c.bench_function("version/compare_numeric", |b| {
        b.iter(|| {
            let r = version::compare(black_box("1.2.12"), black_box("1.2.3"));
            black_box(r);
        });
    });
}

fn bench_version_compare_complex(c: &mut Criterion) {
    c.bench_function("version/compare_complex", |b| {
        b.iter(|| {
            let r = version::compare(
                black_box("2.6.32-431.el6.x86_64"),
                black_box("2.6.32-431.el6.x86_64"),
            );
            black_box(r);
        });
    });
}

fn bench_version_compare_long(c: &mut Criterion) {
    c.bench_function("version/compare_long", |b| {
        b.iter(|| {
            let r = version::compare(
                black_box("1.2.3.4.5.6.7.8.9.10"),
                black_box("1.2.3.4.5.6.7.8.9.11"),
            );
            black_box(r);
        });
    });
}

fn bench_version_compare_mixed(c: &mut Criterion) {
    c.bench_function("version/compare_mixed_alpha_num", |b| {
        b.iter(|| {
            let r = version::compare(black_box("1.0rc1"), black_box("1.0"));
            black_box(r);
        });
    });
}

fn bench_version_comparator_eval(c: &mut Criterion) {
    use libpkgconf::version::Comparator;

    let comparators = vec![
        (Comparator::GreaterThanEqual, "1.0.0", "0.9.0"),
        (Comparator::LessThan, "1.0.0", "2.0.0"),
        (Comparator::Equal, "1.2.3", "1.2.3"),
        (Comparator::NotEqual, "1.0", "2.0"),
    ];

    c.bench_function("version/comparator_eval_batch", |b| {
        b.iter(|| {
            for (cmp, actual, target) in &comparators {
                let r = cmp.eval(actual, target);
                black_box(r);
            }
        });
    });
}

// ============================================================================
// Fragment benchmarks
// ============================================================================

fn bench_fragment_parse_simple(c: &mut Criterion) {
    c.bench_function("fragment/parse_simple", |b| {
        b.iter(|| {
            let list = FragmentList::parse(black_box("-I/usr/include -L/usr/lib -lfoo"));
            black_box(list);
        });
    });
}

fn bench_fragment_parse_complex(c: &mut Criterion) {
    c.bench_function("fragment/parse_complex", |b| {
        b.iter(|| {
            let list = FragmentList::parse(black_box(
                "-I/usr/include/glib-2.0 -I/usr/lib/glib-2.0/include \
                 -I/usr/include/gio-unix-2.0 -pthread \
                 -L/usr/lib -lglib-2.0 -lgobject-2.0 -lgio-2.0 \
                 -DGLIB_COMPILATION -DHAVE_CONFIG_H",
            ));
            black_box(list);
        });
    });
}

fn bench_fragment_parse_many_libs(c: &mut Criterion) {
    let libs: Vec<String> = (0..50).map(|i| format!("-llib{i}")).collect();
    let libs_str = libs.join(" ");

    c.bench_function("fragment/parse_50_libs", |b| {
        b.iter(|| {
            let list = FragmentList::parse(black_box(&libs_str));
            black_box(list);
        });
    });
}

fn bench_fragment_deduplicate(c: &mut Criterion) {
    let flags = "-I/a -I/b -I/a -I/c -I/b -lfoo -lbar -lfoo -lbaz -lbar";
    let list = FragmentList::parse(flags);

    c.bench_function("fragment/deduplicate", |b| {
        b.iter(|| {
            let deduped = black_box(&list).deduplicate();
            black_box(deduped);
        });
    });
}

fn bench_fragment_filter_system_dirs(c: &mut Criterion) {
    let list =
        FragmentList::parse("-I/usr/include -I/opt/include -L/usr/lib -L/opt/lib -lfoo -lbar");
    let sys_libdirs = vec!["/usr/lib".to_string(), "/lib".to_string()];
    let sys_incdirs = vec!["/usr/include".to_string()];

    c.bench_function("fragment/filter_system_dirs", |b| {
        b.iter(|| {
            let filtered = black_box(&list)
                .filter_system_dirs(black_box(&sys_libdirs), black_box(&sys_incdirs));
            black_box(filtered);
        });
    });
}

fn bench_fragment_render(c: &mut Criterion) {
    let list = FragmentList::parse("-I/usr/include/glib-2.0 -L/usr/lib -lglib-2.0 -lgobject-2.0");

    c.bench_function("fragment/render", |b| {
        b.iter(|| {
            let rendered = black_box(&list).render(' ');
            black_box(rendered);
        });
    });
}

fn bench_fragment_render_escaped(c: &mut Criterion) {
    let list = FragmentList::parse("-I/usr/include/glib-2.0 -L/usr/lib -lglib-2.0 -lgobject-2.0");

    c.bench_function("fragment/render_escaped", |b| {
        b.iter(|| {
            let rendered = black_box(&list).render_escaped(' ');
            black_box(rendered);
        });
    });
}

fn bench_fragment_filter_cflags_only_i(c: &mut Criterion) {
    let list =
        FragmentList::parse("-I/usr/include -I/opt/include -DFOO -DBAR -Wall -Werror -pthread");

    c.bench_function("fragment/filter_cflags_only_i", |b| {
        b.iter(|| {
            let filtered = black_box(&list).filter_cflags_only_i();
            black_box(filtered);
        });
    });
}

fn bench_fragment_filter_libs_only_libname(c: &mut Criterion) {
    let list = FragmentList::parse("-L/usr/lib -L/opt/lib -lfoo -lbar -lbaz -pthread");

    c.bench_function("fragment/filter_libs_only_libname", |b| {
        b.iter(|| {
            let filtered = black_box(&list).filter_libs_only_libname();
            black_box(filtered);
        });
    });
}

// ============================================================================
// Dependency parsing benchmarks
// ============================================================================

fn bench_dependency_parse_simple(c: &mut Criterion) {
    c.bench_function("dependency/parse_simple", |b| {
        b.iter(|| {
            let deps = DependencyList::parse(black_box("glib-2.0 >= 2.50"));
            black_box(deps);
        });
    });
}

fn bench_dependency_parse_complex(c: &mut Criterion) {
    c.bench_function("dependency/parse_complex", |b| {
        b.iter(|| {
            let deps = DependencyList::parse(black_box(
                "glib-2.0 >= 2.50, gobject-2.0 >= 2.50, \
                 gio-2.0 >= 2.50, cairo >= 1.14.0, \
                 pango >= 1.38.0, gdk-pixbuf-2.0 >= 2.30.0, \
                 atk >= 2.15.1",
            ));
            black_box(deps);
        });
    });
}

fn bench_dependency_parse_many(c: &mut Criterion) {
    let deps: Vec<String> = (0..50).map(|i| format!("dep-{i} >= {i}.0")).collect();
    let deps_str = deps.join(", ");

    c.bench_function("dependency/parse_50_deps", |b| {
        b.iter(|| {
            let deps = DependencyList::parse(black_box(&deps_str));
            black_box(deps);
        });
    });
}

fn bench_dependency_parse_no_version(c: &mut Criterion) {
    let deps: Vec<String> = (0..20).map(|i| format!("dep-{i}")).collect();
    let deps_str = deps.join(", ");

    c.bench_function("dependency/parse_20_no_version", |b| {
        b.iter(|| {
            let deps = DependencyList::parse(black_box(&deps_str));
            black_box(deps);
        });
    });
}

// ============================================================================
// Package loading benchmarks
// ============================================================================

fn bench_package_from_pc_file(c: &mut Criterion) {
    let client = make_test_client();
    let path = test_data_dir().join("simple.pc");
    let pc = PcFile::from_path(&path).unwrap();

    c.bench_function("package/from_pc_file_simple", |b| {
        b.iter(|| {
            let pkg = Package::from_pc_file(&client, black_box(&pc), "simple").unwrap();
            black_box(pkg);
        });
    });
}

fn bench_package_from_pc_file_complex(c: &mut Criterion) {
    let client = make_test_client();
    let path = test_data_dir().join("libbar.pc");
    let pc = PcFile::from_path(&path).unwrap();

    c.bench_function("package/from_pc_file_complex", |b| {
        b.iter(|| {
            let pkg = Package::from_pc_file(&client, black_box(&pc), "libbar").unwrap();
            black_box(pkg);
        });
    });
}

fn bench_package_find(c: &mut Criterion) {
    let client = make_test_client();

    c.bench_function("package/find_simple", |b| {
        b.iter(|| {
            let pkg = Package::find(&client, black_box("simple"));
            let _ = black_box(pkg);
        });
    });
}

fn bench_package_find_not_found(c: &mut Criterion) {
    let client = make_test_client();

    c.bench_function("package/find_not_found", |b| {
        b.iter(|| {
            let pkg = Package::find(&client, black_box("nonexistent-package-xyz"));
            let _ = black_box(pkg);
        });
    });
}

fn bench_package_scan_all(c: &mut Criterion) {
    let client = make_test_client();

    c.bench_function("package/scan_all", |b| {
        b.iter(|| {
            let all = Package::scan_all(&client);
            black_box(all);
        });
    });
}

// ============================================================================
// Solver / Queue benchmarks
// ============================================================================

fn bench_solve_single_no_deps(c: &mut Criterion) {
    let client = make_test_client();

    c.bench_function("solve/single_no_deps", |b| {
        b.iter(|| {
            let mut queue = Queue::new();
            queue.push("simple");
            let mut cache = Cache::with_builtins(&client);
            let world = queue.solve(&mut cache, &client).unwrap();
            black_box(world);
        });
    });
}

fn bench_solve_with_transitive_deps(c: &mut Criterion) {
    let client = make_test_client();

    c.bench_function("solve/transitive_deps (depender->simple)", |b| {
        b.iter(|| {
            let mut queue = Queue::new();
            queue.push("depender");
            let mut cache = Cache::with_builtins(&client);
            let world = queue.solve(&mut cache, &client).unwrap();
            black_box(world);
        });
    });
}

fn bench_solve_deep_deps(c: &mut Criterion) {
    let client = make_test_client();

    c.bench_function("solve/deep_deps (deep-depender->depender->simple)", |b| {
        b.iter(|| {
            let mut queue = Queue::new();
            queue.push("deep-depender");
            let mut cache = Cache::with_builtins(&client);
            let world = queue.solve(&mut cache, &client).unwrap();
            black_box(world);
        });
    });
}

fn bench_solve_diamond_deps(c: &mut Criterion) {
    let client = make_test_client();

    c.bench_function("solve/diamond_deps", |b| {
        b.iter(|| {
            let mut queue = Queue::new();
            queue.push("diamond-a");
            let mut cache = Cache::with_builtins(&client);
            let world = queue.solve(&mut cache, &client).unwrap();
            black_box(world);
        });
    });
}

fn bench_solve_multiple_packages(c: &mut Criterion) {
    let client = make_test_client();

    c.bench_function("solve/multiple_packages", |b| {
        b.iter(|| {
            let mut queue = Queue::new();
            queue.push("simple");
            queue.push("zlib");
            queue.push("depender");
            let mut cache = Cache::with_builtins(&client);
            let world = queue.solve(&mut cache, &client).unwrap();
            black_box(world);
        });
    });
}

fn bench_solve_with_provides(c: &mut Criterion) {
    let client = make_test_client();

    c.bench_function("solve/with_provides", |b| {
        b.iter(|| {
            let mut queue = Queue::new();
            queue.push("needs-provider");
            let mut cache = Cache::with_builtins(&client);
            let world = queue.solve(&mut cache, &client).unwrap();
            black_box(world);
        });
    });
}

fn bench_solve_exists_check(c: &mut Criterion) {
    let client = make_test_client();

    c.bench_function("solve/exists_check", |b| {
        b.iter(|| {
            let queries = vec!["simple >= 1.0".to_string()];
            let mut cache = Cache::with_builtins(&client);
            let result = libpkgconf::queue::exists(&mut cache, &client, &queries);
            black_box(result);
        });
    });
}

// ============================================================================
// Collect flags benchmarks
// ============================================================================

fn bench_collect_cflags(c: &mut Criterion) {
    let client = make_test_client();
    let mut queue = Queue::new();
    queue.push("diamond-a");
    let mut cache = Cache::with_builtins(&client);
    let world = queue.solve(&mut cache, &client).unwrap();

    c.bench_function("collect/cflags_diamond", |b| {
        b.iter(|| {
            let cflags = libpkgconf::queue::collect_cflags(&cache, &client, black_box(&world));
            black_box(cflags);
        });
    });
}

fn bench_collect_libs(c: &mut Criterion) {
    let client = make_test_client();
    let mut queue = Queue::new();
    queue.push("diamond-a");
    let mut cache = Cache::with_builtins(&client);
    let world = queue.solve(&mut cache, &client).unwrap();

    c.bench_function("collect/libs_diamond", |b| {
        b.iter(|| {
            let libs = libpkgconf::queue::collect_libs(&cache, &client, black_box(&world));
            black_box(libs);
        });
    });
}

// ============================================================================
// Search path benchmarks
// ============================================================================

fn bench_search_path_from_delimited(c: &mut Criterion) {
    let path_str = "/usr/lib/pkgconfig:/usr/share/pkgconfig:\
                    /usr/local/lib/pkgconfig:/usr/local/share/pkgconfig:\
                    /opt/lib/pkgconfig:/opt/share/pkgconfig";

    c.bench_function("searchpath/from_delimited", |b| {
        b.iter(|| {
            let sp = SearchPath::from_delimited(black_box(path_str), ':');
            black_box(sp);
        });
    });
}

fn bench_search_path_match_list(c: &mut Criterion) {
    let mut sp = SearchPath::new();
    sp.add("/usr/lib");
    sp.add("/usr/lib64");
    sp.add("/usr/local/lib");
    sp.add("/lib");
    sp.add("/lib64");
    sp.add("/opt/lib");

    c.bench_function("searchpath/match_list_found", |b| {
        b.iter(|| {
            let result = sp.match_list(black_box("/usr/local/lib"));
            black_box(result);
        });
    });

    c.bench_function("searchpath/match_list_not_found", |b| {
        b.iter(|| {
            let result = sp.match_list(black_box("/nonexistent/path"));
            black_box(result);
        });
    });
}

// ============================================================================
// Cache benchmarks
// ============================================================================

fn bench_cache_lookup(c: &mut Criterion) {
    let client = make_test_client();
    let mut cache = Cache::with_builtins(&client);

    // Pre-populate cache by solving
    let mut queue = Queue::new();
    queue.push("diamond-a");
    let _ = queue.solve(&mut cache, &client);

    c.bench_function("cache/lookup_existing", |b| {
        b.iter(|| {
            let pkg = cache.lookup(black_box("simple"), &client);
            black_box(pkg);
        });
    });

    c.bench_function("cache/lookup_missing", |b| {
        b.iter(|| {
            let pkg = cache.lookup(black_box("nonexistent-xyz"), &client);
            black_box(pkg);
        });
    });
}

// ============================================================================
// Client construction benchmarks
// ============================================================================

fn bench_client_builder(c: &mut Criterion) {
    c.bench_function("client/builder_basic", |b| {
        b.iter(|| {
            let client = ClientBuilder::new()
                .skip_env(true)
                .skip_default_paths(true)
                .with_path("/usr/lib/pkgconfig")
                .with_path("/usr/share/pkgconfig")
                .build();
            black_box(client);
        });
    });
}

fn bench_client_builder_full(c: &mut Criterion) {
    c.bench_function("client/builder_full", |b| {
        b.iter(|| {
            let client = ClientBuilder::new()
                .skip_env(true)
                .skip_default_paths(true)
                .with_path("/usr/lib/pkgconfig")
                .with_path("/usr/share/pkgconfig")
                .with_path("/usr/local/lib/pkgconfig")
                .with_path("/usr/local/share/pkgconfig")
                .filter_libdirs(vec!["/usr/lib".into(), "/lib".into()])
                .filter_includedirs(vec!["/usr/include".into()])
                .define_variable("prefix".into(), "/custom".into())
                .flag(ClientFlags::SEARCH_PRIVATE)
                .max_traversal_depth(1000)
                .build();
            black_box(client);
        });
    });
}

// ============================================================================
// Argv splitting benchmarks
// ============================================================================

fn bench_argv_split_simple(c: &mut Criterion) {
    c.bench_function("argv/split_simple", |b| {
        b.iter(|| {
            let result = parser::argv_split(black_box("-I/usr/include -L/usr/lib -lfoo"));
            black_box(result);
        });
    });
}

fn bench_argv_split_quoted(c: &mut Criterion) {
    c.bench_function("argv/split_quoted", |b| {
        b.iter(|| {
            let result = parser::argv_split(black_box(
                r#"-I"/usr/include/path with spaces" -DVERSION=\"1.0\" -lfoo"#,
            ));
            black_box(result);
        });
    });
}

fn bench_argv_split_many(c: &mut Criterion) {
    let args: Vec<String> = (0..100).map(|i| format!("-DFLAG_{i}={i}")).collect();
    let arg_str = args.join(" ");

    c.bench_function("argv/split_100_args", |b| {
        b.iter(|| {
            let result = parser::argv_split(black_box(&arg_str));
            black_box(result);
        });
    });
}

// ============================================================================
// End-to-end benchmarks
// ============================================================================

fn bench_end_to_end_simple(c: &mut Criterion) {
    c.bench_function("e2e/simple_cflags_libs", |b| {
        b.iter(|| {
            let client = make_test_client();
            let mut queue = Queue::new();
            queue.push("simple");
            let mut cache = Cache::with_builtins(&client);
            let world = queue.solve(&mut cache, &client).unwrap();
            let cflags = libpkgconf::queue::collect_cflags(&cache, &client, &world);
            let libs = libpkgconf::queue::collect_libs(&cache, &client, &world);
            let cflags = cflags.deduplicate();
            let libs = libs.deduplicate();
            black_box((cflags.render(' '), libs.render(' ')));
        });
    });
}

fn bench_end_to_end_diamond(c: &mut Criterion) {
    c.bench_function("e2e/diamond_cflags_libs", |b| {
        b.iter(|| {
            let client = make_test_client();
            let mut queue = Queue::new();
            queue.push("diamond-a");
            let mut cache = Cache::with_builtins(&client);
            let world = queue.solve(&mut cache, &client).unwrap();
            let cflags = libpkgconf::queue::collect_cflags(&cache, &client, &world);
            let libs = libpkgconf::queue::collect_libs(&cache, &client, &world);
            let cflags = cflags.deduplicate();
            let libs = libs.deduplicate();
            black_box((cflags.render(' '), libs.render(' ')));
        });
    });
}

fn bench_end_to_end_complex(c: &mut Criterion) {
    c.bench_function("e2e/libfoo_full_chain", |b| {
        b.iter(|| {
            let client = make_test_client();
            let mut queue = Queue::new();
            queue.push("libfoo");
            let mut cache = Cache::with_builtins(&client);
            let world = queue.solve(&mut cache, &client).unwrap();
            let cflags = libpkgconf::queue::collect_cflags(&cache, &client, &world);
            let libs = libpkgconf::queue::collect_libs(&cache, &client, &world);
            let cflags = cflags.deduplicate();
            let libs = libs.deduplicate();
            black_box((cflags.render(' '), libs.render(' ')));
        });
    });
}

// ============================================================================
// Scaling benchmarks
// ============================================================================

fn bench_solve_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("solve_scaling");

    for depth in [1, 5, 10, 20] {
        group.bench_with_input(
            BenchmarkId::new("chain_depth", depth),
            &depth,
            |b, &depth| {
                // Create a temporary directory with a chain of .pc files
                let dir = tempfile::TempDir::new().unwrap();
                for i in 0..depth {
                    let requires = if i < depth - 1 {
                        format!("Requires: scale-chain-{}\n", i + 1)
                    } else {
                        String::new()
                    };
                    let content = format!(
                        "Name: scale-chain-{i}\nDescription: chain {i}\nVersion: 1.0.0\n\
                         {requires}Libs: -lscale-chain-{i}\n"
                    );
                    let path = dir.path().join(format!("scale-chain-{i}.pc"));
                    fs::write(&path, content).unwrap();
                }

                let client = ClientBuilder::new()
                    .skip_env(true)
                    .skip_default_paths(true)
                    .with_path(dir.path().to_str().unwrap())
                    .build();

                b.iter(|| {
                    let mut queue = Queue::new();
                    queue.push("scale-chain-0");
                    let mut cache = Cache::with_builtins(&client);
                    let world = queue.solve(&mut cache, &client).unwrap();
                    black_box(world);
                });
            },
        );
    }

    for width in [1, 5, 10, 20, 50] {
        group.bench_with_input(BenchmarkId::new("fan_width", width), &width, |b, &width| {
            let dir = tempfile::TempDir::new().unwrap();

            // Create leaf packages
            for i in 0..width {
                let content = format!(
                    "Name: scale-leaf-{i}\nDescription: leaf {i}\nVersion: 1.0.0\n\
                         Libs: -lscale-leaf-{i}\n"
                );
                let path = dir.path().join(format!("scale-leaf-{i}.pc"));
                fs::write(&path, content).unwrap();
            }

            // Create parent
            let requires: Vec<String> = (0..width).map(|i| format!("scale-leaf-{i}")).collect();
            let parent = format!(
                "Name: scale-parent\nDescription: parent\nVersion: 1.0.0\n\
                     Requires: {}\nLibs: -lscale-parent\n",
                requires.join(", ")
            );
            let path = dir.path().join("scale-parent.pc");
            fs::write(&path, parent).unwrap();

            let client = ClientBuilder::new()
                .skip_env(true)
                .skip_default_paths(true)
                .with_path(dir.path().to_str().unwrap())
                .build();

            b.iter(|| {
                let mut queue = Queue::new();
                queue.push("scale-parent");
                let mut cache = Cache::with_builtins(&client);
                let world = queue.solve(&mut cache, &client).unwrap();
                black_box(world);
            });
        });
    }

    group.finish();
}

fn bench_fragment_dedup_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("fragment_dedup_scaling");

    for count in [10, 50, 100, 500] {
        group.bench_with_input(
            BenchmarkId::new("fragment_count", count),
            &count,
            |b, &count| {
                let flags: Vec<String> = (0..count)
                    .map(|i| {
                        format!(
                            "-I/path/{} -l lib{}",
                            i % (count / 3 + 1),
                            i % (count / 2 + 1)
                        )
                    })
                    .collect();
                let flag_str = flags.join(" ");
                let frags = FragmentList::parse(&flag_str);

                b.iter(|| {
                    let deduped = black_box(&frags).deduplicate();
                    black_box(deduped);
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Criterion groups
// ============================================================================

criterion_group!(
    parser_benches,
    bench_parse_simple_pc,
    bench_parse_libbar_pc,
    bench_parse_zlib_pc,
    bench_parse_from_string,
    bench_parse_many_variables,
);

criterion_group!(
    resolve_benches,
    bench_resolve_variables_simple,
    bench_resolve_variables_complex,
    bench_resolve_variables_with_overrides,
);

criterion_group!(
    version_benches,
    bench_version_compare_equal,
    bench_version_compare_numeric,
    bench_version_compare_complex,
    bench_version_compare_long,
    bench_version_compare_mixed,
    bench_version_comparator_eval,
);

criterion_group!(
    fragment_benches,
    bench_fragment_parse_simple,
    bench_fragment_parse_complex,
    bench_fragment_parse_many_libs,
    bench_fragment_deduplicate,
    bench_fragment_filter_system_dirs,
    bench_fragment_render,
    bench_fragment_render_escaped,
    bench_fragment_filter_cflags_only_i,
    bench_fragment_filter_libs_only_libname,
);

criterion_group!(
    dependency_benches,
    bench_dependency_parse_simple,
    bench_dependency_parse_complex,
    bench_dependency_parse_many,
    bench_dependency_parse_no_version,
);

criterion_group!(
    package_benches,
    bench_package_from_pc_file,
    bench_package_from_pc_file_complex,
    bench_package_find,
    bench_package_find_not_found,
    bench_package_scan_all,
);

criterion_group!(
    solver_benches,
    bench_solve_single_no_deps,
    bench_solve_with_transitive_deps,
    bench_solve_deep_deps,
    bench_solve_diamond_deps,
    bench_solve_multiple_packages,
    bench_solve_with_provides,
    bench_solve_exists_check,
);

criterion_group!(collect_benches, bench_collect_cflags, bench_collect_libs,);

criterion_group!(
    path_benches,
    bench_search_path_from_delimited,
    bench_search_path_match_list,
);

criterion_group!(cache_benches, bench_cache_lookup,);

criterion_group!(
    client_benches,
    bench_client_builder,
    bench_client_builder_full,
);

criterion_group!(
    argv_benches,
    bench_argv_split_simple,
    bench_argv_split_quoted,
    bench_argv_split_many,
);

criterion_group!(
    e2e_benches,
    bench_end_to_end_simple,
    bench_end_to_end_diamond,
    bench_end_to_end_complex,
);

criterion_group!(
    scaling_benches,
    bench_solve_scaling,
    bench_fragment_dedup_scaling,
);

criterion_main!(
    parser_benches,
    resolve_benches,
    version_benches,
    fragment_benches,
    dependency_benches,
    package_benches,
    solver_benches,
    collect_benches,
    path_benches,
    cache_benches,
    client_benches,
    argv_benches,
    e2e_benches,
    scaling_benches,
);
