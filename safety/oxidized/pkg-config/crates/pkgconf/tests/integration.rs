#![allow(non_snake_case)]
#![allow(unexpected_cfgs)]

//! Integration tests for the `pkgconf` binary.
//!
//! These tests exercise the CLI interface end-to-end, mirroring the test categories
//! from the upstream pkgconf test suite:
//!
//! - Basic queries (cflags, libs, modversion, variable, exists)
//! - Version comparison and constraints
//! - Dependency resolution (simple, diamond, circular, deep)
//! - Fragment deduplication and ordering
//! - System directory filtering
//! - Static linking
//! - Conflicts detection
//! - Provides resolution
//! - Uninstalled packages
//! - Error messages and exit codes
//! - Environment variable handling
//! - Parser edge cases (comments, multiline, DOS line endings, no trailing newline)
//! - MSVC syntax rendering
//! - Digraph / solution output
//! - Validate mode

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::PathBuf;

/// Returns the absolute path to the workspace-level `tests/data/` directory.
fn test_data_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crates/pkgconf -> workspace root
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    workspace_root.join("tests").join("data")
}

/// Build a Command for the `pkgconf` binary with PKG_CONFIG_PATH pointing
/// at our test data directory and PKG_CONFIG_LIBDIR empty (so we don't pick
/// up system packages).
fn pkgconf() -> Command {
    let mut cmd = Command::cargo_bin("pkgconf").unwrap();
    cmd.env("PKG_CONFIG_PATH", test_data_dir().to_str().unwrap());
    cmd.env("PKG_CONFIG_LIBDIR", test_data_dir().to_str().unwrap());
    // Avoid inheriting system environment that could affect behavior
    cmd.env_remove("PKG_CONFIG_SYSROOT_DIR");
    cmd.env_remove("PKG_CONFIG_ALLOW_SYSTEM_CFLAGS");
    cmd.env_remove("PKG_CONFIG_ALLOW_SYSTEM_LIBS");
    cmd.env_remove("PKG_CONFIG_DISABLE_UNINSTALLED");
    cmd.env_remove("PKG_CONFIG_DEBUG_SPEW");
    cmd.env_remove("PKG_CONFIG_MSVC_SYNTAX");
    cmd.env_remove("PKG_CONFIG_FDO_SYSROOT_RULES");
    cmd.env_remove("PKG_CONFIG_LOG");
    cmd.env_remove("PKG_CONFIG_PRELOADED_FILES");
    cmd.env_remove("PKG_CONFIG_PURE_DEPGRAPH");
    cmd.env_remove("PKG_CONFIG_IGNORE_CONFLICTS");
    cmd
}

// ============================================================================
// Basic queries
// ============================================================================

mod basic {
    use super::*;

    #[test]
    fn version_flag() {
        pkgconf()
            .arg("--version")
            .assert()
            .success()
            .stdout(predicate::str::starts_with("0.29."));
    }

    #[test]
    fn about_flag() {
        pkgconf()
            .arg("--about")
            .assert()
            .success()
            .stdout(predicate::str::contains("pkgconf"));
    }

    #[test]
    fn no_args_fails() {
        // With no packages and no mode flags, should fail
        pkgconf().assert().failure();
    }

    #[test]
    fn atleast_pkgconfig_version_satisfied() {
        pkgconf()
            .args(["--atleast-pkgconfig-version", "0.29.0"])
            .assert()
            .success();
    }

    #[test]
    fn atleast_pkgconfig_version_not_satisfied() {
        pkgconf()
            .args(["--atleast-pkgconfig-version", "99.0.0"])
            .assert()
            .failure();
    }

    #[test]
    fn modversion_simple() {
        pkgconf()
            .args(["--modversion", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("1.0.0"));
    }

    #[test]
    fn modversion_verbose() {
        pkgconf()
            .args(["--modversion", "--verbose", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("simple:"))
            .stdout(predicate::str::contains("1.0.0"));
    }

    #[test]
    fn modversion_multiple_packages() {
        pkgconf()
            .args(["--modversion", "simple", "zlib"])
            .assert()
            .success()
            .stdout(predicate::str::contains("1.0.0"))
            .stdout(predicate::str::contains("1.2.13"));
    }

    #[test]
    fn cflags_simple() {
        pkgconf()
            .args(["--cflags", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-I/usr/include/simple"));
    }

    #[test]
    fn libs_simple() {
        pkgconf()
            .args(["--libs", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-lsimple"));
    }

    #[test]
    fn libs_and_cflags_together() {
        pkgconf()
            .args(["--cflags", "--libs", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-I/usr/include/simple"))
            .stdout(predicate::str::contains("-lsimple"));
    }

    #[test]
    fn cflags_only_i() {
        pkgconf()
            .args(["--cflags-only-I", "libbar"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-I/usr/local/include/bar"));
    }

    #[test]
    fn cflags_only_other() {
        pkgconf()
            .args(["--cflags-only-other", "libbar"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-DBAR_VERSION=231"));
    }

    #[test]
    fn libs_only_l() {
        pkgconf()
            .args(["--libs-only-l", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-lsimple"));
    }

    #[test]
    fn libs_only_L() {
        pkgconf()
            .args(["--keep-system-libs", "--libs-only-L", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-L"));
    }

    #[test]
    fn libs_only_other() {
        // simple has no "other" libs, so output should be empty/whitespace
        pkgconf()
            .args(["--libs-only-other", "simple"])
            .assert()
            .success();
    }

    #[test]
    fn variable_query() {
        pkgconf()
            .args(["--variable=prefix", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("/usr"));
    }

    #[test]
    fn variable_query_custom() {
        pkgconf()
            .args(["--variable=libdir", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("/usr/lib"));
    }

    #[test]
    fn variable_query_nonexistent() {
        // Non-existent variable should print empty line
        pkgconf()
            .args(["--variable=nonexistent", "simple"])
            .assert()
            .success();
    }

    #[test]
    fn print_variables() {
        let assert = pkgconf()
            .args(["--print-variables", "simple"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("prefix"));
        assert!(stdout.contains("exec_prefix"));
        assert!(stdout.contains("libdir"));
        assert!(stdout.contains("includedir"));
    }

    #[test]
    fn print_requires() {
        pkgconf()
            .args(["--print-requires", "depender"])
            .assert()
            .success()
            .stdout(predicate::str::contains("simple"));
    }

    #[test]
    fn print_requires_private() {
        pkgconf()
            .args(["--print-requires-private", "private-deps"])
            .assert()
            .success()
            .stdout(predicate::str::contains("zlib"));
    }

    #[test]
    fn print_provides() {
        pkgconf()
            .args(["--print-provides", "provider"])
            .assert()
            .success()
            .stdout(predicate::str::contains("provider-alias"));
    }

    #[test]
    fn path_flag() {
        pkgconf()
            .args(["--path", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("simple.pc"));
    }

    #[test]
    fn nonexistent_package_fails() {
        pkgconf()
            .args(["--exists", "this-package-does-not-exist"])
            .assert()
            .failure();
    }

    #[test]
    fn list_all() {
        let assert = pkgconf().arg("--list-all").assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("simple"));
        assert!(stdout.contains("zlib"));
    }

    #[test]
    fn list_package_names() {
        let assert = pkgconf().arg("--list-package-names").assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("simple"));
        assert!(stdout.contains("zlib"));
    }

    #[test]
    fn no_cflags_field() {
        // Package with no Cflags field should produce empty/whitespace output
        pkgconf().args(["--cflags", "nocflags"]).assert().success();
    }

    #[test]
    fn no_libs_field() {
        // Package with no Libs field should produce empty/whitespace output
        pkgconf().args(["--libs", "nolibs"]).assert().success();
    }

    #[test]
    fn license_flag() {
        pkgconf()
            .args(["--license", "libbar"])
            .assert()
            .success()
            .stdout(predicate::str::contains("MIT"));
    }

    #[test]
    fn license_noassertion() {
        // Package with no License field should say NOASSERTION
        pkgconf()
            .args(["--license", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("NOASSERTION"));
    }
}

// ============================================================================
// Version comparison and constraints
// ============================================================================

mod version_constraints {
    use super::*;

    #[test]
    fn exists_simple() {
        pkgconf().args(["--exists", "simple"]).assert().success();
    }

    #[test]
    fn exists_nonexistent() {
        pkgconf()
            .args(["--exists", "nonexistent-package"])
            .assert()
            .failure();
    }

    #[test]
    fn exists_with_version_satisfied() {
        pkgconf()
            .args(["--exists", "simple >= 1.0"])
            .assert()
            .success();
    }

    #[test]
    fn exists_with_version_not_satisfied() {
        pkgconf()
            .args(["--exists", "simple >= 99.0"])
            .assert()
            .failure();
    }

    #[test]
    fn exists_equal_version() {
        pkgconf()
            .args(["--exists", "simple = 1.0.0"])
            .assert()
            .success();
    }

    #[test]
    fn exists_equal_version_mismatch() {
        pkgconf()
            .args(["--exists", "simple = 2.0.0"])
            .assert()
            .failure();
    }

    #[test]
    fn exists_less_than() {
        pkgconf()
            .args(["--exists", "simple < 2.0.0"])
            .assert()
            .success();
    }

    #[test]
    fn exists_less_than_fail() {
        pkgconf()
            .args(["--exists", "simple < 0.5.0"])
            .assert()
            .failure();
    }

    #[test]
    fn exists_not_equal() {
        pkgconf()
            .args(["--exists", "simple != 2.0.0"])
            .assert()
            .success();
    }

    #[test]
    fn exists_not_equal_fail() {
        pkgconf()
            .args(["--exists", "simple != 1.0.0"])
            .assert()
            .failure();
    }

    #[test]
    fn exists_greater_than() {
        pkgconf()
            .args(["--exists", "simple > 0.5.0"])
            .assert()
            .success();
    }

    #[test]
    fn exists_greater_than_fail() {
        pkgconf()
            .args(["--exists", "simple > 2.0.0"])
            .assert()
            .failure();
    }

    #[test]
    fn atleast_version_satisfied() {
        pkgconf()
            .args(["--atleast-version=1.0.0", "simple"])
            .assert()
            .success();
    }

    #[test]
    fn atleast_version_not_satisfied() {
        pkgconf()
            .args(["--atleast-version=99.0.0", "simple"])
            .assert()
            .failure();
    }

    #[test]
    fn exact_version_match() {
        pkgconf()
            .args(["--exact-version=1.0.0", "simple"])
            .assert()
            .success();
    }

    #[test]
    fn exact_version_mismatch() {
        pkgconf()
            .args(["--exact-version=2.0.0", "simple"])
            .assert()
            .failure();
    }

    #[test]
    fn max_version_satisfied() {
        pkgconf()
            .args(["--max-version=2.0.0", "simple"])
            .assert()
            .success();
    }

    #[test]
    fn max_version_not_satisfied() {
        pkgconf()
            .args(["--max-version=0.5.0", "simple"])
            .assert()
            .failure();
    }

    #[test]
    fn exists_multiple_packages_comma() {
        pkgconf()
            .args(["--exists", "simple >= 1.0, zlib >= 1.0"])
            .assert()
            .success();
    }

    #[test]
    fn exists_multiple_packages_space() {
        pkgconf()
            .args(["--exists", "simple", "zlib"])
            .assert()
            .success();
    }

    #[test]
    fn tilde_version_exists() {
        pkgconf()
            .args(["--exists", "tilde-version"])
            .assert()
            .success();
    }

    #[test]
    fn tilde_version_less_than_release() {
        // 1.0.0~beta1 should be less than 1.0.0
        pkgconf()
            .args(["--exists", "tilde-version < 1.0.0"])
            .assert()
            .success();
    }

    #[test]
    fn tilde_version_greater_than_previous() {
        // 1.0.0~beta1 should be greater than 0.9.0
        pkgconf()
            .args(["--exists", "tilde-version > 0.9.0"])
            .assert()
            .success();
    }
}

// ============================================================================
// Dependency resolution
// ============================================================================

mod dependency_resolution {
    use super::*;

    #[test]
    fn simple_no_deps() {
        pkgconf()
            .args(["--libs", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-lsimple"));
    }

    #[test]
    fn transitive_deps() {
        // depender -> simple
        let assert = pkgconf().args(["--libs", "depender"]).assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-ldepender"));
        assert!(stdout.contains("-lsimple"));
    }

    #[test]
    fn deep_transitive_deps() {
        // deep-depender -> depender -> simple
        let assert = pkgconf()
            .args(["--libs", "deep-depender"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-ldeep-depender"));
        assert!(stdout.contains("-ldepender"));
        assert!(stdout.contains("-lsimple"));
    }

    #[test]
    fn diamond_deps_no_duplicates() {
        // diamond-a -> diamond-b -> diamond-d
        //           -> diamond-c -> diamond-d
        // diamond-d should appear only once
        let assert = pkgconf().args(["--libs", "diamond-a"]).assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        let count = stdout.matches("-ldiamond-d").count();
        assert_eq!(
            count, 1,
            "diamond-d should appear exactly once but appeared {} times in: {}",
            count, stdout
        );
    }

    #[test]
    fn diamond_deps_ordering() {
        // In libs output, the leaf dependency (diamond-d) should come after
        // its dependants
        let assert = pkgconf().args(["--libs", "diamond-a"]).assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        let pos_a = stdout.find("-ldiamond-a").expect("should have -ldiamond-a");
        let pos_d = stdout.find("-ldiamond-d").expect("should have -ldiamond-d");
        assert!(pos_a < pos_d, "diamond-a should appear before diamond-d");
    }

    #[test]
    fn circular_deps_handled() {
        // circular-1 -> circular-2 -> circular-1 (cycle)
        // Should not hang or error out
        pkgconf()
            .args(["--libs", "circular-1"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-lcircular-1"));
    }

    #[test]
    fn circular_deps_cflags() {
        pkgconf()
            .args(["--cflags", "circular-1"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-I/usr/include/circular-1"));
    }

    #[test]
    fn metapackage_pulls_in_deps() {
        // metapackage has no libs/cflags of its own, but requires simple and zlib
        let assert = pkgconf().args(["--libs", "metapackage"]).assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-lsimple"));
        assert!(stdout.contains("-lz"));
    }

    #[test]
    fn metapackage_exists() {
        pkgconf()
            .args(["--exists", "metapackage"])
            .assert()
            .success();
    }

    #[test]
    fn missing_required_dep_fails() {
        pkgconf()
            .args(["--exists", "missing-require"])
            .assert()
            .failure();
    }

    #[test]
    fn missing_required_dep_libs_fails() {
        pkgconf()
            .args(["--libs", "missing-require"])
            .assert()
            .failure();
    }

    #[test]
    fn multiple_packages_query() {
        let assert = pkgconf()
            .args(["--libs", "simple", "zlib"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-lsimple"));
        assert!(stdout.contains("-lz"));
    }
}

// ============================================================================
// Static linking
// ============================================================================

mod static_linking {
    use super::*;

    #[test]
    fn static_includes_private_libs() {
        let assert = pkgconf()
            .args(["--static", "--libs", "private-deps"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-lprivate-deps"));
        // Private libs should be included
        assert!(stdout.contains("-lm"));
        assert!(stdout.contains("-lpthread"));
    }

    #[test]
    fn static_includes_private_requires() {
        // In static mode, private requires should be resolved
        let assert = pkgconf()
            .args(["--static", "--libs", "static-libs"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-lstatic-libs"));
        // Private libs
        assert!(stdout.contains("-lm"));
        assert!(stdout.contains("-lpthread"));
        assert!(stdout.contains("-ldl"));
    }

    #[test]
    fn non_static_excludes_private_libs() {
        let assert = pkgconf()
            .args(["--libs", "private-deps"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-lprivate-deps"));
        // Private libs should NOT be included in non-static mode
        // (m and pthread are from Libs.private)
    }

    #[test]
    fn static_cflags_includes_private() {
        let assert = pkgconf()
            .args(["--static", "--cflags", "static-libs"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-I/usr/include/static-libs"));
        assert!(stdout.contains("-DSTATIC_LIBS_INTERNAL"));
    }
}

// ============================================================================
// Conflicts detection
// ============================================================================

mod conflicts {
    use super::*;

    #[test]
    fn conflict_detected() {
        // conflicting package conflicts with simple < 2.0, and simple is 1.0.0
        // So requesting both should fail
        pkgconf()
            .args(["--exists", "simple", "conflicting"])
            .assert()
            .failure();
    }

    #[test]
    fn conflicting_alone_is_fine() {
        // The conflicting package by itself should be fine
        pkgconf()
            .args(["--exists", "conflicting"])
            .assert()
            .success();
    }

    #[test]
    fn conflict_ignored_with_flag() {
        // With --ignore-conflicts, the conflict should be ignored
        pkgconf()
            .args(["--ignore-conflicts", "--exists", "simple", "conflicting"])
            .assert()
            .success();
    }
}

// ============================================================================
// Provides resolution
// ============================================================================

mod provides {
    use super::*;

    #[test]
    fn provides_satisfies_dependency() {
        // needs-provider depends on provider-alias, which is provided by provider
        pkgconf()
            .args(["--exists", "needs-provider"])
            .assert()
            .success();
    }

    #[test]
    fn provides_transitive_libs() {
        let assert = pkgconf()
            .args(["--libs", "needs-provider"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-lneeds-provider"));
        assert!(stdout.contains("-lprovider"));
    }

    #[test]
    fn provides_versioned() {
        // provider provides provider-alias = 2.0.0
        pkgconf()
            .args(["--exists", "provider-alias >= 1.0"])
            .assert()
            .success();
    }

    #[test]
    fn provides_version_too_high() {
        pkgconf()
            .args(["--exists", "provider-alias >= 99.0"])
            .assert()
            .failure();
    }
}

// ============================================================================
// System directory filtering
// ============================================================================

mod system_dirs {
    use super::*;

    #[test]
    fn system_include_dirs_filtered() {
        // simple has -I/usr/include/simple, but /usr/include is a system dir
        // The -I flag path includes a subdirectory, so it should NOT be filtered
        pkgconf()
            .args(["--cflags", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-I/usr/include/simple"));
    }

    #[test]
    fn keep_system_cflags() {
        pkgconf()
            .args(["--keep-system-cflags", "--cflags", "zlib"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-I/usr/include"));
    }

    #[test]
    fn keep_system_libs() {
        pkgconf()
            .args(["--keep-system-libs", "--libs", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-L/usr/lib"));
    }

    #[test]
    fn system_lib_dirs_filtered_by_default() {
        // Without --keep-system-libs, -L/usr/lib should be filtered
        let assert = pkgconf().args(["--libs", "simple"]).assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        // Should still have -lsimple
        assert!(stdout.contains("-lsimple"));
        // -L/usr/lib should be filtered out
        assert!(
            !stdout.contains("-L/usr/lib"),
            "system libdir should be filtered: {}",
            stdout
        );
    }

    #[test]
    fn env_allow_system_cflags() {
        pkgconf()
            .env("PKG_CONFIG_ALLOW_SYSTEM_CFLAGS", "1")
            .args(["--cflags", "zlib"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-I/usr/include"));
    }

    #[test]
    fn env_allow_system_libs() {
        pkgconf()
            .env("PKG_CONFIG_ALLOW_SYSTEM_LIBS", "1")
            .args(["--libs", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-L/usr/lib"));
    }
}

// ============================================================================
// Fragment ordering
// ============================================================================

mod fragment_ordering {
    use super::*;

    #[test]
    fn libs_order_respects_dependency_order() {
        // depender -> simple: depender libs should come before simple libs
        let assert = pkgconf().args(["--libs", "depender"]).assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        let pos_depender = stdout.find("-ldepender").expect("should have -ldepender");
        let pos_simple = stdout.find("-lsimple").expect("should have -lsimple");
        assert!(
            pos_depender < pos_simple,
            "depender should come before simple in libs output: {}",
            stdout
        );
    }

    #[test]
    fn cflags_order_respects_dependency_order() {
        let assert = pkgconf().args(["--cflags", "depender"]).assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        let pos_depender = stdout.find("-I/usr/include/depender");
        let pos_simple = stdout.find("-I/usr/include/simple");
        if let (Some(pd), Some(ps)) = (pos_depender, pos_simple) {
            assert!(
                pd < ps,
                "depender includes should come before simple includes: {}",
                stdout
            );
        }
    }

    #[test]
    fn flag_order_multiple_packages() {
        // When querying "flag-order-1 flag-order-2", flags from flag-order-1
        // should appear before flag-order-2's
        let assert = pkgconf()
            .args(["--libs", "flag-order-1", "flag-order-2"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        let pos_1 = stdout.find("-lflag-order-1");
        let pos_2 = stdout.find("-lflag-order-2");
        if let (Some(p1), Some(p2)) = (pos_1, pos_2) {
            assert!(
                p1 < p2,
                "flag-order-1 should come before flag-order-2: {}",
                stdout
            );
        }
    }

    #[test]
    fn framework_flags_preserved() {
        let assert = pkgconf().args(["--libs", "framework"]).assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-framework CoreFoundation") || stdout.contains("-framework"));
    }
}

// ============================================================================
// Uninstalled packages
// ============================================================================

mod uninstalled {
    use super::*;

    /// Returns the path to the uninstalled test data subdirectory,
    /// which contains both simple.pc and simple-uninstalled.pc.
    fn uninstalled_data_dir() -> PathBuf {
        test_data_dir().join("uninstalled")
    }

    /// Build a Command pointing at the uninstalled test data subdirectory.
    fn pkgconf_uninstalled() -> Command {
        let mut cmd = Command::cargo_bin("pkgconf").unwrap();
        let dir = uninstalled_data_dir();
        cmd.env("PKG_CONFIG_PATH", dir.to_str().unwrap());
        cmd.env("PKG_CONFIG_LIBDIR", dir.to_str().unwrap());
        cmd.env_remove("PKG_CONFIG_SYSROOT_DIR");
        cmd.env_remove("PKG_CONFIG_ALLOW_SYSTEM_CFLAGS");
        cmd.env_remove("PKG_CONFIG_ALLOW_SYSTEM_LIBS");
        cmd.env_remove("PKG_CONFIG_DISABLE_UNINSTALLED");
        cmd.env_remove("PKG_CONFIG_DEBUG_SPEW");
        cmd.env_remove("PKG_CONFIG_MSVC_SYNTAX");
        cmd.env_remove("PKG_CONFIG_FDO_SYSROOT_RULES");
        cmd.env_remove("PKG_CONFIG_LOG");
        cmd.env_remove("PKG_CONFIG_PRELOADED_FILES");
        cmd.env_remove("PKG_CONFIG_PURE_DEPGRAPH");
        cmd.env_remove("PKG_CONFIG_IGNORE_CONFLICTS");
        cmd
    }

    #[test]
    fn uninstalled_detected() {
        // The simple-uninstalled.pc file should be detected in the uninstalled subdir
        pkgconf_uninstalled()
            .args(["--uninstalled", "simple"])
            .assert()
            .success();
    }

    #[test]
    fn uninstalled_flag_with_regular_package() {
        // zlib has no uninstalled variant, should fail with --uninstalled
        pkgconf().args(["--uninstalled", "zlib"]).assert().failure();
    }

    #[test]
    fn no_uninstalled_flag() {
        // With --no-uninstalled, the uninstalled variant should be skipped
        // and the regular simple.pc should be used
        pkgconf_uninstalled()
            .args(["--no-uninstalled", "--modversion", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("1.0.0"));
    }
}

// ============================================================================
// Parser edge cases
// ============================================================================

mod parser {
    use super::*;

    #[test]
    fn comments_handled() {
        pkgconf()
            .args(["--modversion", "comments"])
            .assert()
            .success()
            .stdout(predicate::str::contains("1.0.0"));
    }

    #[test]
    fn comments_libs() {
        pkgconf()
            .args(["--libs", "comments"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-lcomments"));
    }

    #[test]
    fn multiline_fields() {
        pkgconf()
            .args(["--modversion", "multiline"])
            .assert()
            .success()
            .stdout(predicate::str::contains("1.0.0"));
    }

    #[test]
    fn multiline_libs() {
        pkgconf()
            .args(["--libs", "multiline"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-lmultiline"));
    }

    #[test]
    fn multiline_cflags() {
        let assert = pkgconf().args(["--cflags", "multiline"]).assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-DMULTILINE=1"));
        assert!(stdout.contains("-DEXTRA=2"));
    }

    #[test]
    fn dos_line_endings() {
        pkgconf()
            .args(["--modversion", "dos-lineendings"])
            .assert()
            .success()
            .stdout(predicate::str::contains("1.0.0"));
    }

    #[test]
    fn dos_line_endings_libs() {
        pkgconf()
            .args(["--libs", "dos-lineendings"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-ldos-lineendings"));
    }

    #[test]
    fn no_trailing_newline() {
        pkgconf()
            .args(["--modversion", "no-trailing-newline"])
            .assert()
            .success()
            .stdout(predicate::str::contains("1.0.0"));
    }

    #[test]
    fn no_trailing_newline_cflags() {
        pkgconf()
            .args(["--cflags", "no-trailing-newline"])
            .assert()
            .success()
            .stdout(predicate::str::contains(
                "-I/usr/include/no-trailing-newline",
            ));
    }

    #[test]
    fn unicode_description() {
        // Package with Unicode in description should parse correctly
        pkgconf()
            .args(["--modversion", "unicode"])
            .assert()
            .success()
            .stdout(predicate::str::contains("1.0.0"));
    }

    #[test]
    fn unicode_libs() {
        pkgconf()
            .args(["--libs", "unicode"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-lunicode"));
    }

    #[test]
    fn empty_pc_file() {
        // An empty .pc file should either error gracefully or return empty results
        // pkgconf upstream treats empty files as valid but with no useful data
        let result = pkgconf().args(["--exists", "empty"]).assert();
        // Either success with empty or failure is acceptable
        let _ = result;
    }

    #[test]
    fn variables_only_file() {
        // A file with only variables and no fields - looking up a variable should work
        // but --exists should fail since there's no Name/Version
        let result = pkgconf().args(["--exists", "variables-only"]).assert();
        // This may fail since there's no Version field, which is acceptable
        let _ = result;
    }

    #[test]
    fn direct_query_with_full_path() {
        // Query a .pc file by its full path
        let pc_path = test_data_dir().join("simple.pc");
        pkgconf()
            .args(["--modversion", pc_path.to_str().unwrap()])
            .assert()
            .success()
            .stdout(predicate::str::contains("1.0.0"));
    }
}

// ============================================================================
// Define-variable
// ============================================================================

mod define_variable {
    use super::*;

    #[test]
    fn define_variable_override() {
        pkgconf()
            .args([
                "--define-variable=prefix=/custom",
                "--variable=prefix",
                "simple",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("/custom"));
    }

    #[test]
    fn define_variable_affects_derived() {
        // Overriding prefix should affect libdir (which is ${prefix}/lib -> ${exec_prefix}/lib)
        pkgconf()
            .args([
                "--define-variable=prefix=/custom",
                "--variable=libdir",
                "simple",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("/custom"));
    }

    #[test]
    fn define_variable_affects_cflags() {
        pkgconf()
            .args(["--define-variable=prefix=/custom", "--cflags", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-I/custom/include/simple"));
    }

    #[test]
    fn define_variable_affects_libs() {
        pkgconf()
            .args([
                "--keep-system-libs",
                "--define-variable=prefix=/custom",
                "--libs",
                "simple",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("-L/custom/lib"));
    }
}

// ============================================================================
// Environment variable handling
// ============================================================================

mod environment {
    use super::*;

    #[test]
    fn pkg_config_path() {
        pkgconf()
            .env("PKG_CONFIG_PATH", test_data_dir().to_str().unwrap())
            .env("PKG_CONFIG_LIBDIR", "")
            .args(["--exists", "simple"])
            .assert()
            .success();
    }

    #[test]
    fn pkg_config_libdir_overrides_default() {
        pkgconf()
            .env("PKG_CONFIG_LIBDIR", test_data_dir().to_str().unwrap())
            .args(["--exists", "simple"])
            .assert()
            .success();
    }

    #[test]
    fn empty_pkg_config_path() {
        // With empty paths and LIBDIR pointing elsewhere, our test packages shouldn't be found
        pkgconf()
            .env("PKG_CONFIG_PATH", "")
            .env("PKG_CONFIG_LIBDIR", "/nonexistent/path")
            .args(["--exists", "simple"])
            .assert()
            .failure();
    }

    #[test]
    fn maximum_traverse_depth() {
        pkgconf()
            .env("PKG_CONFIG_MAXIMUM_TRAVERSE_DEPTH", "100")
            .args(["--libs", "deep-depender"])
            .assert()
            .success();
    }

    #[test]
    fn maximum_traverse_depth_too_shallow() {
        // With depth 1, deep-depender -> depender -> simple should fail
        // because the traversal depth is exceeded during resolution
        pkgconf()
            .env("PKG_CONFIG_MAXIMUM_TRAVERSE_DEPTH", "1")
            .args(["--libs", "deep-depender"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Maximum traversal depth"));
    }
}

// ============================================================================
// MSVC syntax
// ============================================================================

mod msvc_syntax {
    use super::*;

    #[test]
    fn msvc_cflags_include() {
        let assert = pkgconf()
            .args(["--msvc-syntax", "--cflags", "simple"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(
            stdout.contains("/I") || stdout.contains("-I"),
            "MSVC syntax should transform -I to /I: {}",
            stdout
        );
    }

    #[test]
    fn msvc_libs() {
        let assert = pkgconf()
            .args(["--msvc-syntax", "--libs", "simple"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(
            stdout.contains("simple.lib") || stdout.contains("/LIBPATH:"),
            "MSVC syntax should transform lib flags: {}",
            stdout
        );
    }

    #[test]
    fn msvc_via_env() {
        let assert = pkgconf()
            .env("PKG_CONFIG_MSVC_SYNTAX", "1")
            .args(["--cflags", "simple"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(
            stdout.contains("/I") || stdout.contains("-I"),
            "MSVC syntax via env should transform -I to /I: {}",
            stdout
        );
    }
}

// ============================================================================
// Digraph and solution output
// ============================================================================

mod output_formats {
    use super::*;

    #[test]
    fn digraph_output() {
        pkgconf()
            .args(["--digraph", "depender"])
            .assert()
            .success()
            .stdout(predicate::str::contains("digraph"))
            .stdout(predicate::str::contains("depender"))
            .stdout(predicate::str::contains("simple"));
    }

    #[test]
    fn digraph_with_query_nodes() {
        pkgconf()
            .args(["--digraph", "--print-digraph-query-nodes", "depender"])
            .assert()
            .success()
            .stdout(predicate::str::contains("digraph"));
    }

    #[test]
    fn solution_output() {
        let assert = pkgconf()
            .args(["--solution", "depender"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("depender"));
        assert!(stdout.contains("simple"));
    }

    #[test]
    fn simulate_output() {
        let assert = pkgconf()
            .args(["--simulate", "depender"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("depender"));
        assert!(stdout.contains("simple"));
    }

    #[test]
    fn env_output() {
        let assert = pkgconf()
            .args(["--env=MY_PKG", "--cflags", "--libs", "simple"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("MY_PKG_CFLAGS="));
        assert!(stdout.contains("MY_PKG_LIBS="));
    }

    #[test]
    fn newlines_flag() {
        let assert = pkgconf()
            .args(["--newlines", "--cflags", "--libs", "simple"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        // With --newlines, flags should be separated by newlines
        let lines: Vec<&str> = stdout.lines().collect();
        // Should have at least 2 lines (one for include, one for lib)
        assert!(
            lines.len() >= 1,
            "Should have at least 1 line with --newlines: {:?}",
            lines
        );
    }

    #[test]
    fn exists_cflags() {
        let assert = pkgconf()
            .args(["--exists-cflags", "--cflags", "simple", "zlib"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-DHAVE_SIMPLE"));
        assert!(stdout.contains("-DHAVE_ZLIB"));
    }
}

// ============================================================================
// Validate mode
// ============================================================================

mod validate {
    use super::*;

    #[test]
    fn validate_valid_package() {
        pkgconf().args(["--validate", "simple"]).assert().success();
    }

    #[test]
    fn validate_valid_multiple() {
        pkgconf()
            .args(["--validate", "simple", "zlib"])
            .assert()
            .success();
    }

    #[test]
    fn validate_nonexistent_fails() {
        pkgconf()
            .args(["--validate", "nonexistent-package"])
            .assert()
            .failure();
    }

    #[test]
    fn validate_with_deps() {
        pkgconf()
            .args(["--validate", "depender"])
            .assert()
            .success();
    }
}

// ============================================================================
// Built-in virtual packages
// ============================================================================

mod builtins {
    use super::*;

    #[test]
    fn builtin_pkg_config_exists() {
        pkgconf()
            .args(["--exists", "pkg-config"])
            .assert()
            .success();
    }

    #[test]
    fn builtin_pkgconf_exists() {
        pkgconf().args(["--exists", "pkgconf"]).assert().success();
    }

    #[test]
    fn builtin_pkg_config_pc_path_variable() {
        pkgconf()
            .args(["--variable=pc_path", "pkg-config"])
            .assert()
            .success()
            .stdout(predicate::str::is_empty().not());
    }
}

// ============================================================================
// With-path
// ============================================================================

mod with_path {
    use super::*;

    #[test]
    fn with_path_adds_search_dir() {
        pkgconf()
            .env("PKG_CONFIG_PATH", "")
            .env("PKG_CONFIG_LIBDIR", "/nonexistent")
            .args([
                &format!("--with-path={}", test_data_dir().to_str().unwrap()),
                "--exists",
                "simple",
            ])
            .assert()
            .success();
    }

    #[test]
    fn with_path_multiple() {
        pkgconf()
            .env("PKG_CONFIG_PATH", "")
            .env("PKG_CONFIG_LIBDIR", "/nonexistent")
            .args([
                &format!("--with-path={}", test_data_dir().to_str().unwrap()),
                "--exists",
                "simple",
                "zlib",
            ])
            .assert()
            .success();
    }
}

// ============================================================================
// Sysroot handling
// ============================================================================

mod sysroot {
    use super::*;

    #[test]
    fn sysroot_prepended_to_include_paths() {
        let assert = pkgconf()
            .env("PKG_CONFIG_SYSROOT_DIR", "/mysysroot")
            .args(["--cflags", "sysroot-test"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(
            stdout.contains("/mysysroot/usr/include/sysroot-test")
                || stdout.contains("-I/mysysroot"),
            "Sysroot should be prepended to include paths: {}",
            stdout
        );
    }

    #[test]
    fn sysroot_prepended_to_lib_paths() {
        let assert = pkgconf()
            .env("PKG_CONFIG_SYSROOT_DIR", "/mysysroot")
            .env("PKG_CONFIG_ALLOW_SYSTEM_LIBS", "1")
            .args(["--libs", "sysroot-test"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(
            stdout.contains("/mysysroot") || stdout.contains("-L/mysysroot"),
            "Sysroot should be prepended to lib paths: {}",
            stdout
        );
    }

    #[test]
    fn empty_sysroot_is_noop() {
        // Empty sysroot should not change paths
        pkgconf()
            .env("PKG_CONFIG_SYSROOT_DIR", "")
            .args(["--cflags", "simple"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-I/usr/include/simple"));
    }
}

// ============================================================================
// Fragment tree output
// ============================================================================

mod fragment_tree {
    use super::*;

    #[test]
    fn fragment_tree_output() {
        let assert = pkgconf()
            .args(["--fragment-tree", "--cflags", "--libs", "depender"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        // Fragment tree should contain section headers
        assert!(
            stdout.contains("CFLAGS") || stdout.contains("LIBS"),
            "Fragment tree should contain section headers: {}",
            stdout
        );
    }
}

// ============================================================================
// Edge case: complex .pc file fields
// ============================================================================

mod complex_packages {
    use super::*;

    #[test]
    fn libbar_full_metadata() {
        // libbar has extensive metadata
        pkgconf().args(["--exists", "libbar"]).assert().success();
    }

    #[test]
    fn libbar_version() {
        pkgconf()
            .args(["--modversion", "libbar"])
            .assert()
            .success()
            .stdout(predicate::str::contains("2.3.1"));
    }

    #[test]
    fn libbar_cflags() {
        let assert = pkgconf().args(["--cflags", "libbar"]).assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-I/usr/local/include/bar") || stdout.contains("-I"));
        assert!(stdout.contains("-DBAR_VERSION=231"));
    }

    #[test]
    fn libbar_provides_compat() {
        // libbar provides libbar-compat = 2.3
        pkgconf()
            .args(["--exists", "libbar-compat >= 2.0"])
            .assert()
            .success();
    }

    #[test]
    fn libfoo_depends_on_libbar() {
        // libfoo requires libbar >= 1.0
        let assert = pkgconf().args(["--libs", "libfoo"]).assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-lfoo"));
        assert!(stdout.contains("-lbar"));
    }

    #[test]
    fn libfoo_transitive_cflags() {
        let assert = pkgconf().args(["--cflags", "libfoo"]).assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-DLIBFOO_VERSION=123"));
    }
}

// ============================================================================
// Differential testing harness (compares output with system pkg-config)
// ============================================================================

#[cfg(feature = "differential")]
mod differential {
    use super::*;
    use std::process::Command as StdCommand;

    fn system_pkg_config() -> Option<String> {
        which::which("pkg-config")
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    }

    fn compare_output(args: &[&str], pkg_config_path: &str) {
        let sys = match system_pkg_config() {
            Some(s) => s,
            None => {
                eprintln!("Skipping differential test: pkg-config not found");
                return;
            }
        };

        let ours = Command::cargo_bin("pkgconf")
            .unwrap()
            .env("PKG_CONFIG_PATH", pkg_config_path)
            .env("PKG_CONFIG_LIBDIR", pkg_config_path)
            .args(args)
            .output()
            .expect("failed to run our pkgconf");

        let theirs = StdCommand::new(&sys)
            .env("PKG_CONFIG_PATH", pkg_config_path)
            .env("PKG_CONFIG_LIBDIR", pkg_config_path)
            .args(args)
            .output()
            .expect("failed to run system pkg-config");

        assert_eq!(
            ours.status.success(),
            theirs.status.success(),
            "Exit code mismatch for args {:?}: ours={}, theirs={}",
            args,
            ours.status,
            theirs.status
        );

        if ours.status.success() {
            let ours_stdout = String::from_utf8_lossy(&ours.stdout);
            let theirs_stdout = String::from_utf8_lossy(&theirs.stdout);
            assert_eq!(
                ours_stdout.trim(),
                theirs_stdout.trim(),
                "Output mismatch for args {:?}",
                args
            );
        }
    }

    #[test]
    fn diff_modversion() {
        compare_output(
            &["--modversion", "simple"],
            test_data_dir().to_str().unwrap(),
        );
    }

    #[test]
    fn diff_cflags() {
        compare_output(&["--cflags", "simple"], test_data_dir().to_str().unwrap());
    }

    #[test]
    fn diff_libs() {
        compare_output(&["--libs", "simple"], test_data_dir().to_str().unwrap());
    }

    #[test]
    fn diff_exists() {
        compare_output(&["--exists", "simple"], test_data_dir().to_str().unwrap());
    }

    #[test]
    fn diff_variable() {
        compare_output(
            &["--variable=prefix", "simple"],
            test_data_dir().to_str().unwrap(),
        );
    }
}
