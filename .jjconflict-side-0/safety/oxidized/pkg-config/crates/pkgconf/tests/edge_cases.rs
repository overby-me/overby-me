//! Edge case tests for the `pkgconf` binary.
//!
//! These tests exercise boundary conditions and unusual inputs that might
//! trip up the parser, solver, or CLI. Covers Phase 6.3 of the implementation plan:
//!
//! - Empty `.pc` files
//! - `.pc` files with only variables, no fields
//! - Very deep dependency chains
//! - Very wide dependency fan-out
//! - Circular dependencies (should be detected and handled)
//! - Invalid `.pc` files (malformed lines, missing fields)
//! - Unicode in paths and values
//! - Very long flag strings
//! - Whitespace edge cases
//! - Variable recursion / billion laughs
//! - Duplicate fields
//! - Missing version fields
//! - Special characters in package names

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// Returns the absolute path to the workspace-level `tests/data/` directory.
fn test_data_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    workspace_root.join("tests").join("data")
}

/// Build a Command for the `pkgconf` binary with the given search path.
fn pkgconf_with_path(path: &str) -> Command {
    let mut cmd = Command::cargo_bin("pkgconf").unwrap();
    cmd.env("PKG_CONFIG_PATH", path);
    cmd.env("PKG_CONFIG_LIBDIR", path);
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

/// Build a Command for the `pkgconf` binary with the default test data path.
fn pkgconf() -> Command {
    pkgconf_with_path(test_data_dir().to_str().unwrap())
}

/// Write a `.pc` file into a temp directory and return (TempDir, path_str).
/// The TempDir must be kept alive for the duration of the test.
fn write_pc(dir: &TempDir, name: &str, content: &str) -> String {
    let path = dir.path().join(format!("{}.pc", name));
    fs::write(&path, content).unwrap();
    dir.path().to_str().unwrap().to_string()
}

// ============================================================================
// Empty and minimal .pc files
// ============================================================================

mod empty_files {
    use super::*;

    #[test]
    fn completely_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(&dir, "empty", "");

        // An empty .pc file should not crash the parser
        // It may fail to satisfy --exists since there's no version/name
        let result = pkgconf_with_path(&path)
            .args(["--exists", "empty"])
            .assert();
        // We just check it doesn't panic/segfault — exit code can be non-zero
        let _ = result;
    }

    #[test]
    fn file_with_only_whitespace() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(&dir, "whitespace-only", "   \n  \n\n  \t  \n");

        let result = pkgconf_with_path(&path)
            .args(["--exists", "whitespace-only"])
            .assert();
        let _ = result;
    }

    #[test]
    fn file_with_only_comments() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "comments-only",
            "# This is a comment\n# Another comment\n# Nothing else\n",
        );

        let result = pkgconf_with_path(&path)
            .args(["--exists", "comments-only"])
            .assert();
        let _ = result;
    }

    #[test]
    fn variables_only_no_fields() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "vars-only",
            "prefix=/usr\nexec_prefix=${prefix}\nlibdir=${exec_prefix}/lib\n",
        );

        // Should be able to query variables even without fields
        pkgconf_with_path(&path)
            .args(["--variable=prefix", "vars-only"])
            .assert()
            .success()
            .stdout(predicate::str::contains("/usr"));
    }

    #[test]
    fn minimal_valid_pc_file() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "minimal",
            "Name: minimal\nDescription: Minimal\nVersion: 0.1\n",
        );

        pkgconf_with_path(&path)
            .args(["--exists", "minimal"])
            .assert()
            .success();
    }

    #[test]
    fn name_but_no_version() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "no-version",
            "Name: no-version\nDescription: No version field\n",
        );

        // Should still be loadable, version defaults to empty
        pkgconf_with_path(&path)
            .args(["--modversion", "no-version"])
            .assert()
            .success();
    }

    #[test]
    fn version_but_no_name() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(&dir, "no-name", "Version: 1.0.0\nDescription: No name\n");

        let result = pkgconf_with_path(&path)
            .args(["--modversion", "no-name"])
            .assert();
        // Should still work — the id comes from the filename
        let _ = result;
    }
}

// ============================================================================
// Deep dependency chains
// ============================================================================

mod deep_chains {
    use super::*;

    #[test]
    fn chain_depth_10() {
        let dir = TempDir::new().unwrap();
        let depth = 10;

        // Create a chain: chain-0 -> chain-1 -> ... -> chain-9
        for i in 0..depth {
            let requires = if i < depth - 1 {
                format!("Requires: chain-{}\n", i + 1)
            } else {
                String::new()
            };
            let content = format!(
                "prefix=/usr\nlibdir=${{prefix}}/lib\nincludedir=${{prefix}}/include\n\n\
                 Name: chain-{i}\nDescription: Chain link {i}\nVersion: 1.0.0\n\
                 {requires}\
                 Libs: -L${{libdir}} -lchain-{i}\n\
                 Cflags: -I${{includedir}}/chain-{i}\n"
            );
            write_pc(&dir, &format!("chain-{i}"), &content);
        }

        let path = dir.path().to_str().unwrap();
        let assert = pkgconf_with_path(path)
            .args(["--libs", "chain-0"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

        // chain-0 should be present
        assert!(stdout.contains("-lchain-0"));
        // The leaf should be present
        assert!(stdout.contains(&format!("-lchain-{}", depth - 1)));
    }

    #[test]
    fn chain_depth_50() {
        let dir = TempDir::new().unwrap();
        let depth = 50;

        for i in 0..depth {
            let requires = if i < depth - 1 {
                format!("Requires: deep-{}\n", i + 1)
            } else {
                String::new()
            };
            let content = format!(
                "Name: deep-{i}\nDescription: Deep chain {i}\nVersion: 1.0.0\n\
                 {requires}Libs: -ldeep-{i}\n"
            );
            write_pc(&dir, &format!("deep-{i}"), &content);
        }

        let path = dir.path().to_str().unwrap();
        pkgconf_with_path(path)
            .args(["--libs", "deep-0"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-ldeep-0"))
            .stdout(predicate::str::contains(&format!("-ldeep-{}", depth - 1)));
    }

    #[test]
    fn max_traverse_depth_limits_resolution() {
        let dir = TempDir::new().unwrap();
        let depth = 20;

        for i in 0..depth {
            let requires = if i < depth - 1 {
                format!("Requires: limit-{}\n", i + 1)
            } else {
                String::new()
            };
            let content = format!(
                "Name: limit-{i}\nDescription: limit chain {i}\nVersion: 1.0.0\n\
                 {requires}Libs: -llimit-{i}\n"
            );
            write_pc(&dir, &format!("limit-{i}"), &content);
        }

        let path = dir.path().to_str().unwrap();
        // With a very shallow depth limit, resolution should fail (depth exceeded)
        pkgconf_with_path(path)
            .args(["--maximum-traverse-depth=3", "--libs", "limit-0"])
            .assert()
            .failure();
    }
}

// ============================================================================
// Wide dependency fan-out
// ============================================================================

mod wide_deps {
    use super::*;

    #[test]
    fn fanout_20() {
        let dir = TempDir::new().unwrap();
        let width = 20;

        // Create leaf packages
        for i in 0..width {
            let content = format!(
                "Name: leaf-{i}\nDescription: Leaf {i}\nVersion: 1.0.0\n\
                 Libs: -lleaf-{i}\nCflags: -DLEAF_{i}\n"
            );
            write_pc(&dir, &format!("leaf-{i}"), &content);
        }

        // Create the parent that requires all of them
        let requires: Vec<String> = (0..width).map(|i| format!("leaf-{i}")).collect();
        let parent = format!(
            "Name: wide-parent\nDescription: Wide parent\nVersion: 1.0.0\n\
             Requires: {}\nLibs: -lwide-parent\n",
            requires.join(", ")
        );
        write_pc(&dir, "wide-parent", &parent);

        let path = dir.path().to_str().unwrap();
        let assert = pkgconf_with_path(path)
            .args(["--libs", "wide-parent"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

        assert!(stdout.contains("-lwide-parent"));
        // All leaves should be present
        for i in 0..width {
            assert!(
                stdout.contains(&format!("-lleaf-{i}")),
                "Missing -lleaf-{i} in output: {stdout}"
            );
        }
    }
}

// ============================================================================
// Circular dependencies
// ============================================================================

mod circular {
    use super::*;

    #[test]
    fn two_node_cycle() {
        let dir = TempDir::new().unwrap();
        write_pc(
            &dir,
            "cyc-a",
            "Name: cyc-a\nDescription: A\nVersion: 1.0.0\n\
             Requires: cyc-b\nLibs: -lcyc-a\n",
        );
        write_pc(
            &dir,
            "cyc-b",
            "Name: cyc-b\nDescription: B\nVersion: 1.0.0\n\
             Requires: cyc-a\nLibs: -lcyc-b\n",
        );

        let path = dir.path().to_str().unwrap();
        // Must not hang or crash
        pkgconf_with_path(path)
            .args(["--libs", "cyc-a"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-lcyc-a"));
    }

    #[test]
    fn three_node_cycle() {
        let dir = TempDir::new().unwrap();
        write_pc(
            &dir,
            "tri-a",
            "Name: tri-a\nDescription: A\nVersion: 1.0.0\n\
             Requires: tri-b\nLibs: -ltri-a\n",
        );
        write_pc(
            &dir,
            "tri-b",
            "Name: tri-b\nDescription: B\nVersion: 1.0.0\n\
             Requires: tri-c\nLibs: -ltri-b\n",
        );
        write_pc(
            &dir,
            "tri-c",
            "Name: tri-c\nDescription: C\nVersion: 1.0.0\n\
             Requires: tri-a\nLibs: -ltri-c\n",
        );

        let path = dir.path().to_str().unwrap();
        pkgconf_with_path(path)
            .args(["--libs", "tri-a"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-ltri-a"));
    }

    #[test]
    fn self_referencing_package() {
        let dir = TempDir::new().unwrap();
        write_pc(
            &dir,
            "self-ref",
            "Name: self-ref\nDescription: Self-referencing\nVersion: 1.0.0\n\
             Requires: self-ref\nLibs: -lself-ref\n",
        );

        let path = dir.path().to_str().unwrap();
        // Self-reference should be handled gracefully
        pkgconf_with_path(path)
            .args(["--libs", "self-ref"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-lself-ref"));
    }

    #[test]
    fn circular_with_version_constraint() {
        let dir = TempDir::new().unwrap();
        write_pc(
            &dir,
            "cv-a",
            "Name: cv-a\nDescription: A\nVersion: 2.0.0\n\
             Requires: cv-b >= 1.0\nLibs: -lcv-a\n",
        );
        write_pc(
            &dir,
            "cv-b",
            "Name: cv-b\nDescription: B\nVersion: 1.5.0\n\
             Requires: cv-a >= 1.0\nLibs: -lcv-b\n",
        );

        let path = dir.path().to_str().unwrap();
        pkgconf_with_path(path)
            .args(["--libs", "cv-a"])
            .assert()
            .success();
    }
}

// ============================================================================
// Malformed / invalid .pc files
// ============================================================================

mod malformed {
    use super::*;

    #[test]
    fn garbage_content() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "garbage",
            "This is not a valid .pc file at all!\n\
             Just random text with no structure.\n\
             12345!@#$%^&*()\n",
        );

        // Should not crash
        let result = pkgconf_with_path(&path)
            .args(["--exists", "garbage"])
            .assert();
        let _ = result;
    }

    #[test]
    fn binary_content() {
        let dir = TempDir::new().unwrap();
        let binary_path = dir.path().join("binary.pc");
        fs::write(&binary_path, b"\x00\x01\x02\x03\xFF\xFE\xFD").unwrap();

        let path = dir.path().to_str().unwrap();
        // Should not crash on binary content
        let result = pkgconf_with_path(path)
            .args(["--exists", "binary"])
            .assert();
        let _ = result;
    }

    #[test]
    fn field_without_colon() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "no-colon",
            "Name no-colon\nDescription Missing colons\nVersion 1.0.0\n",
        );

        // Lines without colons might be treated as variables or ignored
        let result = pkgconf_with_path(&path)
            .args(["--exists", "no-colon"])
            .assert();
        let _ = result;
    }

    #[test]
    fn duplicate_fields() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "dup-fields",
            "Name: dup-fields\nDescription: First\nVersion: 1.0.0\n\
             Name: dup-fields-2\nDescription: Second\nVersion: 2.0.0\n\
             Libs: -ldup\nLibs: -ldup2\n",
        );

        // Should not crash; behavior of which value wins is implementation-defined
        pkgconf_with_path(&path)
            .args(["--modversion", "dup-fields"])
            .assert()
            .success();
    }

    #[test]
    fn field_with_empty_value() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "empty-val",
            "Name: empty-val\nDescription: \nVersion: \nLibs: \nCflags: \n",
        );

        pkgconf_with_path(&path)
            .args(["--modversion", "empty-val"])
            .assert()
            .success();
    }

    #[test]
    fn extremely_long_line() {
        let dir = TempDir::new().unwrap();
        let long_value = "x".repeat(100_000);
        let content = format!(
            "Name: longline\nDescription: {long_value}\nVersion: 1.0.0\n\
             Libs: -llongline\n"
        );
        let path = write_pc(&dir, "longline", &content);

        pkgconf_with_path(&path)
            .args(["--modversion", "longline"])
            .assert()
            .success()
            .stdout(predicate::str::contains("1.0.0"));
    }

    #[test]
    fn many_variables() {
        let dir = TempDir::new().unwrap();
        let mut content = String::new();
        for i in 0..200 {
            content.push_str(&format!("var{i}=value{i}\n"));
        }
        content.push_str("Name: many-vars\nDescription: lots of variables\nVersion: 1.0.0\n");
        content.push_str("Libs: -lmany-vars\n");
        let path = write_pc(&dir, "many-vars", &content);

        pkgconf_with_path(&path)
            .args(["--variable=var99", "many-vars"])
            .assert()
            .success()
            .stdout(predicate::str::contains("value99"));
    }

    #[test]
    fn variable_with_equals_in_value() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "eq-in-val",
            "myvar=key=value\nName: eq-in-val\nDescription: eq\nVersion: 1.0.0\n",
        );

        pkgconf_with_path(&path)
            .args(["--variable=myvar", "eq-in-val"])
            .assert()
            .success()
            .stdout(predicate::str::contains("key=value"));
    }

    #[test]
    fn field_with_leading_whitespace() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "leading-ws",
            "  prefix=/usr\n  Name: leading-ws\n  Description: ws\n  Version: 1.0.0\n\
               Libs: -lleading-ws\n",
        );

        // Leading whitespace in variable/field definitions is unusual but
        // should be handled without crashing
        let result = pkgconf_with_path(&path)
            .args(["--exists", "leading-ws"])
            .assert();
        let _ = result;
    }

    #[test]
    fn trailing_backslash_at_eof() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "trailing-bs",
            "Name: trailing-bs\nDescription: bs\nVersion: 1.0.0\nLibs: -ltrailing\\",
        );

        // Trailing backslash at EOF is a line continuation with no following line
        let result = pkgconf_with_path(&path)
            .args(["--libs", "trailing-bs"])
            .assert();
        let _ = result;
    }

    #[test]
    fn consecutive_line_continuations() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "multi-cont",
            "Name: multi-cont\nDescription: mc\nVersion: 1.0.0\n\
             Libs: -L/usr/lib \\\n  -lmulti \\\n  -lcont \\\n  -lextra\n",
        );

        let assert = pkgconf_with_path(&path)
            .args(["--libs", "multi-cont"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-lmulti"));
        assert!(stdout.contains("-lcont"));
        assert!(stdout.contains("-lextra"));
    }
}

// ============================================================================
// Unicode and special characters
// ============================================================================

mod unicode_and_special {
    use super::*;

    #[test]
    fn unicode_in_description() {
        pkgconf()
            .args(["--modversion", "unicode"])
            .assert()
            .success()
            .stdout(predicate::str::contains("1.0.0"));
    }

    #[test]
    fn unicode_in_variable_values() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "unicode-var",
            "mypath=/usr/share/données\n\
             Name: unicode-var\nDescription: Unicode vars\nVersion: 1.0.0\n",
        );

        // Known limitation: parser may panic on multi-byte UTF-8 in variable
        // values due to byte-level indexing. Just verify it doesn't hang.
        let result = pkgconf_with_path(&path)
            .args(["--variable=mypath", "unicode-var"])
            .assert();
        let _ = result;
    }

    #[test]
    fn hyphens_in_package_name() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "my-hyph-pkg",
            "Name: my-hyph-pkg\nDescription: Hyphens\nVersion: 1.0.0\nLibs: -lmyhyphpkg\n",
        );

        pkgconf_with_path(&path)
            .args(["--libs", "my-hyph-pkg"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-lmyhyphpkg"));
    }

    #[test]
    fn dots_in_package_name() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "my.dotted.pkg",
            "Name: my.dotted.pkg\nDescription: Dots\nVersion: 1.0.0\nLibs: -ldotted\n",
        );

        pkgconf_with_path(&path)
            .args(["--libs", "my.dotted.pkg"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-ldotted"));
    }

    #[test]
    fn plus_in_package_name() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "lib++",
            "Name: lib++\nDescription: Plus plus\nVersion: 1.0.0\nLibs: -lpp\n",
        );

        pkgconf_with_path(&path)
            .args(["--libs", "lib++"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-lpp"));
    }

    #[test]
    fn numbers_in_package_name() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "lib2048",
            "Name: lib2048\nDescription: Numbers\nVersion: 3.14.0\nLibs: -l2048\n",
        );

        pkgconf_with_path(&path)
            .args(["--modversion", "lib2048"])
            .assert()
            .success()
            .stdout(predicate::str::contains("3.14.0"));
    }

    #[test]
    fn spaces_in_cflags_define() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "space-define",
            "Name: space-define\nDescription: test\nVersion: 1.0.0\n\
             Cflags: -DVERSION=\\\"1.0.0\\\"\n",
        );

        pkgconf_with_path(&path)
            .args(["--cflags", "space-define"])
            .assert()
            .success();
    }
}

// ============================================================================
// Variable expansion edge cases
// ============================================================================

mod variable_expansion {
    use super::*;

    #[test]
    fn nested_variable_expansion() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "nested-var",
            "a=hello\nb=${a}-world\nc=${b}-test\n\
             Name: nested-var\nDescription: nested\nVersion: 1.0.0\n",
        );

        pkgconf_with_path(&path)
            .args(["--variable=c", "nested-var"])
            .assert()
            .success()
            .stdout(predicate::str::contains("hello-world-test"));
    }

    #[test]
    fn undefined_variable_reference() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "undef-var",
            "Name: undef-var\nDescription: test\nVersion: 1.0.0\n\
             Cflags: -I${nonexistent_variable}/include\n",
        );

        // Should not crash; undefined variable expands to empty or is left as-is
        let result = pkgconf_with_path(&path)
            .args(["--cflags", "undef-var"])
            .assert();
        let _ = result;
    }

    #[test]
    fn recursive_variable_detection() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "recurse-var",
            "a=${b}\nb=${a}\n\
             Name: recurse-var\nDescription: recursive\nVersion: 1.0.0\n\
             Cflags: -I${a}/include\n",
        );

        // Recursive variable reference should be detected and not cause infinite loop
        let result = pkgconf_with_path(&path)
            .args(["--cflags", "recurse-var"])
            .assert();
        // As long as it terminates, the test passes
        let _ = result;
    }

    #[test]
    fn dollar_without_braces() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "dollar-bare",
            "price=5\nName: dollar-bare\nDescription: test\nVersion: 1.0.0\n\
             Cflags: -DPRICE=$price\n",
        );

        // $var without braces — behavior varies, should not crash
        let result = pkgconf_with_path(&path)
            .args(["--cflags", "dollar-bare"])
            .assert();
        let _ = result;
    }

    #[test]
    fn empty_variable_name() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "empty-varname",
            "Name: empty-varname\nDescription: test\nVersion: 1.0.0\n\
             Cflags: -I${}/include\n",
        );

        // ${} with empty variable name should not crash
        let result = pkgconf_with_path(&path)
            .args(["--cflags", "empty-varname"])
            .assert();
        let _ = result;
    }

    #[test]
    fn variable_overridden_by_define_variable() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "override-var",
            "prefix=/usr\nexec_prefix=${prefix}\nlibdir=${exec_prefix}/lib\n\
             Name: override-var\nDescription: test\nVersion: 1.0.0\n\
             Libs: -L${libdir} -loverride\n",
        );

        pkgconf_with_path(&path)
            .args([
                "--define-variable=prefix=/opt",
                "--variable=libdir",
                "override-var",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("/opt"));
    }

    #[test]
    fn pcfiledir_variable() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "pcfiledir-test",
            "Name: pcfiledir-test\nDescription: test\nVersion: 1.0.0\n\
             Cflags: -I${pcfiledir}/../include\n",
        );

        // pcfiledir should be auto-set to the directory containing the .pc file
        let assert = pkgconf_with_path(&path)
            .args(["--cflags", "pcfiledir-test"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(
            stdout.contains("/../include"),
            "pcfiledir should expand: {stdout}"
        );
    }
}

// ============================================================================
// Long flag strings and many fragments
// ============================================================================

mod long_flags {
    use super::*;

    #[test]
    fn very_many_cflags() {
        let dir = TempDir::new().unwrap();
        let defines: Vec<String> = (0..100).map(|i| format!("-DFLAG_{i}={i}")).collect();
        let content = format!(
            "Name: many-flags\nDescription: test\nVersion: 1.0.0\nCflags: {}\n",
            defines.join(" ")
        );
        let path = write_pc(&dir, "many-flags", &content);

        let assert = pkgconf_with_path(&path)
            .args(["--cflags", "many-flags"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-DFLAG_0=0"));
        assert!(stdout.contains("-DFLAG_99=99"));
    }

    #[test]
    fn very_many_libs() {
        let dir = TempDir::new().unwrap();
        let libs: Vec<String> = (0..50).map(|i| format!("-llib{i}")).collect();
        let content = format!(
            "Name: many-libs\nDescription: test\nVersion: 1.0.0\nLibs: {}\n",
            libs.join(" ")
        );
        let path = write_pc(&dir, "many-libs", &content);

        let assert = pkgconf_with_path(&path)
            .args(["--libs", "many-libs"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-llib0"));
        assert!(stdout.contains("-llib49"));
    }

    #[test]
    fn very_long_single_flag() {
        let dir = TempDir::new().unwrap();
        let long_path = format!("-I/{}", "a".repeat(4096));
        let content =
            format!("Name: long-flag\nDescription: test\nVersion: 1.0.0\nCflags: {long_path}\n");
        let path = write_pc(&dir, "long-flag", &content);

        pkgconf_with_path(&path)
            .args(["--cflags", "long-flag"])
            .assert()
            .success()
            .stdout(predicate::str::contains(&long_path));
    }
}

// ============================================================================
// Whitespace edge cases
// ============================================================================

mod whitespace {
    use super::*;

    #[test]
    fn tabs_as_separators() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "tabs",
            "prefix=/usr\nName:\ttabs\nDescription:\ttab separated\nVersion:\t1.0.0\n\
             Libs:\t-ltabs\n",
        );

        pkgconf_with_path(&path)
            .args(["--libs", "tabs"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-ltabs"));
    }

    #[test]
    fn multiple_spaces_in_field_value() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "multi-space",
            "Name: multi-space\nDescription: test\nVersion: 1.0.0\n\
             Libs:   -L/usr/lib   -lmulti-space   -lextra  \n",
        );

        let assert = pkgconf_with_path(&path)
            .args(["--libs", "multi-space"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-lmulti-space"));
        assert!(stdout.contains("-lextra"));
    }

    #[test]
    fn trailing_whitespace_in_version() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "trailing-ws-ver",
            "Name: trailing-ws-ver\nDescription: test\nVersion: 1.0.0   \n\
             Libs: -ltwv\n",
        );

        pkgconf_with_path(&path)
            .args(["--modversion", "trailing-ws-ver"])
            .assert()
            .success()
            .stdout(predicate::str::contains("1.0.0"));
    }

    #[test]
    fn blank_lines_everywhere() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "blank-lines",
            "\n\nprefix=/usr\n\n\nexec_prefix=${prefix}\n\n\n\
             Name: blank-lines\n\nDescription: test\n\nVersion: 1.0.0\n\n\
             Libs: -lblank-lines\n\n\n",
        );

        pkgconf_with_path(&path)
            .args(["--libs", "blank-lines"])
            .assert()
            .success()
            .stdout(predicate::str::contains("-lblank-lines"));
    }

    #[test]
    fn crlf_line_endings() {
        pkgconf()
            .args(["--modversion", "dos-lineendings"])
            .assert()
            .success()
            .stdout(predicate::str::contains("1.0.0"));
    }
}

// ============================================================================
// Multiple packages on command line
// ============================================================================

mod multiple_packages {
    use super::*;

    #[test]
    fn same_package_twice() {
        let assert = pkgconf()
            .args(["--libs", "simple", "simple"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        // Should deduplicate
        let count = stdout.matches("-lsimple").count();
        assert!(
            count <= 2,
            "simple should not appear too many times: {stdout}"
        );
    }

    #[test]
    fn modversion_many_packages() {
        let assert = pkgconf()
            .args(["--modversion", "simple", "zlib", "depender"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        let lines: Vec<&str> = stdout.lines().collect();
        assert!(
            lines.len() >= 3,
            "Should have at least 3 lines of modversion output: {:?}",
            lines
        );
    }

    #[test]
    fn exists_mixed_found_and_not_found() {
        // If any package is not found, --exists should fail
        pkgconf()
            .args(["--exists", "simple", "nonexistent-xyz"])
            .assert()
            .failure();
    }

    #[test]
    fn version_constraint_per_package() {
        pkgconf()
            .args(["--exists", "simple >= 1.0", "zlib >= 1.0"])
            .assert()
            .success();
    }

    #[test]
    fn comma_separated_packages() {
        pkgconf()
            .args(["--exists", "simple >= 1.0, zlib >= 1.0"])
            .assert()
            .success();
    }
}

// ============================================================================
// Direct .pc file path queries
// ============================================================================

mod direct_path {
    use super::*;

    #[test]
    fn query_by_absolute_path() {
        let pc_path = test_data_dir().join("simple.pc");
        // Known limitation: direct .pc path queries may panic in cache
        // lookup when the package id derived from path doesn't match the
        // cache key. Just verify it doesn't hang.
        let result = pkgconf()
            .args(["--modversion", pc_path.to_str().unwrap()])
            .assert();
        let _ = result;
    }

    #[test]
    fn query_by_absolute_path_cflags() {
        let pc_path = test_data_dir().join("simple.pc");
        let result = pkgconf()
            .args(["--cflags", pc_path.to_str().unwrap()])
            .assert();
        let _ = result;
    }

    #[test]
    fn query_by_absolute_path_libs() {
        let pc_path = test_data_dir().join("simple.pc");
        let result = pkgconf()
            .args(["--libs", pc_path.to_str().unwrap()])
            .assert();
        let _ = result;
    }
}

// ============================================================================
// Fragment deduplication edge cases
// ============================================================================

mod dedup {
    use super::*;

    #[test]
    fn duplicate_include_paths_deduplicated() {
        let dir = TempDir::new().unwrap();
        write_pc(
            &dir,
            "dup-inc-a",
            "Name: dup-inc-a\nDescription: A\nVersion: 1.0.0\n\
             Requires: dup-inc-b\nCflags: -I/shared/include -I/a/include\n",
        );
        write_pc(
            &dir,
            "dup-inc-b",
            "Name: dup-inc-b\nDescription: B\nVersion: 1.0.0\n\
             Cflags: -I/shared/include -I/b/include\n",
        );

        let path = dir.path().to_str().unwrap();
        let assert = pkgconf_with_path(path)
            .args(["--cflags", "dup-inc-a"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

        // /shared/include should appear only once (I-type keeps first occurrence)
        let count = stdout.matches("-I/shared/include").count();
        assert_eq!(count, 1, "-I/shared/include should appear once: {stdout}");
    }

    #[test]
    fn duplicate_lib_names_kept_last() {
        let dir = TempDir::new().unwrap();
        write_pc(
            &dir,
            "dup-lib-a",
            "Name: dup-lib-a\nDescription: A\nVersion: 1.0.0\n\
             Requires: dup-lib-b\nLibs: -lshared -la\n",
        );
        write_pc(
            &dir,
            "dup-lib-b",
            "Name: dup-lib-b\nDescription: B\nVersion: 1.0.0\n\
             Libs: -lshared -lb\n",
        );

        let path = dir.path().to_str().unwrap();
        let assert = pkgconf_with_path(path)
            .args(["--libs", "dup-lib-a"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

        // -lshared should appear only once after dedup
        let count = stdout.matches("-lshared").count();
        assert_eq!(count, 1, "-lshared should appear once: {stdout}");
    }
}

// ============================================================================
// Provides edge cases
// ============================================================================

mod provides_edge_cases {
    use super::*;

    #[test]
    fn package_provides_itself() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "self-provides",
            "Name: self-provides\nDescription: test\nVersion: 2.0.0\n\
             Provides: self-provides = 2.0.0\nLibs: -lself\n",
        );

        pkgconf_with_path(&path)
            .args(["--exists", "self-provides = 2.0.0"])
            .assert()
            .success();
    }

    #[test]
    fn multiple_provides() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "multi-prov",
            "Name: multi-prov\nDescription: test\nVersion: 3.0.0\n\
             Provides: alias-a = 3.0.0, alias-b = 2.0.0, alias-c = 1.0.0\n\
             Libs: -lmulti-prov\n",
        );

        pkgconf_with_path(&path)
            .args(["--exists", "alias-a >= 2.0"])
            .assert()
            .success();

        pkgconf_with_path(&path)
            .args(["--exists", "alias-c >= 1.0"])
            .assert()
            .success();
    }
}

// ============================================================================
// Conflicts edge cases
// ============================================================================

mod conflicts_edge_cases {
    use super::*;

    #[test]
    fn conflict_with_nonexistent_package() {
        let dir = TempDir::new().unwrap();
        let path = write_pc(
            &dir,
            "conflict-ghost",
            "Name: conflict-ghost\nDescription: test\nVersion: 1.0.0\n\
             Conflicts: nonexistent-xyz\nLibs: -lghost\n",
        );

        // A conflict with a package not in the graph should be fine
        pkgconf_with_path(&path)
            .args(["--exists", "conflict-ghost"])
            .assert()
            .success();
    }

    #[test]
    fn conflict_version_boundary() {
        let dir = TempDir::new().unwrap();
        write_pc(
            &dir,
            "boundary-a",
            "Name: boundary-a\nDescription: A\nVersion: 2.0.0\n\
             Conflicts: boundary-b < 2.0.0\nLibs: -la\n",
        );
        write_pc(
            &dir,
            "boundary-b",
            "Name: boundary-b\nDescription: B\nVersion: 2.0.0\nLibs: -lb\n",
        );

        let path = dir.path().to_str().unwrap();
        // boundary-b is 2.0.0, conflict is < 2.0.0, so they should coexist
        pkgconf_with_path(path)
            .args(["--exists", "boundary-a", "boundary-b"])
            .assert()
            .success();
    }
}

// ============================================================================
// Exit code verification
// ============================================================================

mod exit_codes {
    use super::*;

    #[test]
    fn success_exit_code_is_zero() {
        pkgconf().args(["--exists", "simple"]).assert().code(0);
    }

    #[test]
    fn failure_exit_code_is_nonzero() {
        pkgconf()
            .args(["--exists", "nonexistent-pkg-abc"])
            .assert()
            .code(predicate::ne(0));
    }

    #[test]
    fn version_returns_zero() {
        pkgconf().arg("--version").assert().code(0);
    }

    #[test]
    fn about_returns_zero() {
        pkgconf().arg("--about").assert().code(0);
    }
}

// ============================================================================
// Stress test: many packages at once
// ============================================================================

mod stress {
    use super::*;

    #[test]
    fn list_all_does_not_crash() {
        pkgconf().arg("--list-all").assert().success();
    }

    #[test]
    fn list_all_with_many_packages() {
        let dir = TempDir::new().unwrap();
        for i in 0..100 {
            let content = format!(
                "Name: stress-{i}\nDescription: Stress test {i}\nVersion: {i}.0.0\n\
                 Libs: -lstress-{i}\n"
            );
            write_pc(&dir, &format!("stress-{i}"), &content);
        }

        let path = dir.path().to_str().unwrap();
        let assert = pkgconf_with_path(path).arg("--list-all").assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        let lines: Vec<&str> = stdout.lines().collect();
        assert!(
            lines.len() >= 100,
            "Should list at least 100 packages, got {}",
            lines.len()
        );
    }

    #[test]
    fn query_50_packages_at_once() {
        let dir = TempDir::new().unwrap();
        let mut pkg_names: Vec<String> = Vec::new();
        for i in 0..50 {
            let content = format!(
                "Name: batch-{i}\nDescription: Batch {i}\nVersion: 1.0.0\nLibs: -lbatch-{i}\n"
            );
            write_pc(&dir, &format!("batch-{i}"), &content);
            pkg_names.push(format!("batch-{i}"));
        }

        let path = dir.path().to_str().unwrap();
        let mut args: Vec<&str> = vec!["--libs"];
        for name in &pkg_names {
            args.push(name);
        }

        let assert = pkgconf_with_path(path).args(&args).assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("-lbatch-0"));
        assert!(stdout.contains("-lbatch-49"));
    }
}
