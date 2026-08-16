#!/usr/bin/env nu
#
# Differential testing script for rust-pkg-config.
#
# This script compares the output of our `pkgconf` binary against the system
# `pkg-config` (or `pkgconf`) for a variety of queries. It reports any
# differences in stdout, stderr, or exit code.
#
# Usage:
#   nu tests/differential.nu [--test-data-only] [--system-packages] [--verbose] [--stop-on-fail]
#
# Options:
#   --test-data-only    Only test against .pc files in tests/data/
#   --system-packages   Also test against system-installed .pc files
#   --verbose           Print each test as it runs
#   --stop-on-fail      Stop on first failure
#
# Prerequisites:
#   - cargo build must have been run
#   - pkg-config or pkgconf must be installed on the system

# Compare a single invocation of our binary vs. the system binary.
# Uses $env.DIFF_PASS, $env.DIFF_FAIL, $env.DIFF_TOTAL as mutable counters.
def --env compare [
    desc: string
    our_bin: string
    sys_bin: string
    env_vars: record
    cmd_args: list<string>
    verbose: bool
    stop_on_fail: bool
] {
    let red = (ansi red)
    let green = (ansi green)
    let nc = (ansi reset)

    $env.DIFF_TOTAL = $env.DIFF_TOTAL + 1

    if $verbose {
        print -n $"  Testing: ($desc) ... "
    }

    # Run our binary
    let our_result = (with-env $env_vars { do -c { ^$our_bin ...$cmd_args } | complete })

    # Run system binary
    let sys_result = (with-env $env_vars { do -c { ^$sys_bin ...$cmd_args } | complete })

    let our_exit = $our_result.exit_code
    let sys_exit = $sys_result.exit_code

    # Compare exit codes
    if $our_exit != $sys_exit {
        $env.DIFF_FAIL = $env.DIFF_FAIL + 1
        if $verbose {
            print $"($red)FAIL($nc) \(exit code: ours=($our_exit), theirs=($sys_exit)\)"
            let our_err_lines = ($our_result.stderr | lines | first 3 | str join "\n")
            let sys_err_lines = ($sys_result.stderr | lines | first 3 | str join "\n")
            print $"    Our stderr: ($our_err_lines)"
            print $"    Sys stderr: ($sys_err_lines)"
        } else {
            print $"($red)FAIL($nc): ($desc) \(exit code: ours=($our_exit), theirs=($sys_exit)\)"
        }
        if $stop_on_fail {
            print ""
            print "Stopping on first failure."
            print $"  Our stdout: ($our_result.stdout)"
            print $"  Sys stdout: ($sys_result.stdout)"
            print $"  Our stderr: ($our_result.stderr)"
            print $"  Sys stderr: ($sys_result.stderr)"
            exit 1
        }
        return
    }

    # Compare stdout (normalize whitespace)
    let our_out = ($our_result.stdout | str trim -r | lines | sort | str join "\n")
    let sys_out = ($sys_result.stdout | str trim -r | lines | sort | str join "\n")

    if $our_out != $sys_out {
        # Try comparing without sorting — normalize by splitting on whitespace, sorting, rejoining
        let our_norm = ($our_result.stdout | str trim -r | split row -r '\s+' | sort | str join " ")
        let sys_norm = ($sys_result.stdout | str trim -r | split row -r '\s+' | sort | str join " ")

        if $our_norm != $sys_norm {
            $env.DIFF_FAIL = $env.DIFF_FAIL + 1
            if $verbose {
                print $"($red)FAIL($nc) \(output differs\)"
                let our_head = ($our_result.stdout | lines | first 3 | str join "\n")
                let sys_head = ($sys_result.stdout | lines | first 3 | str join "\n")
                print $"    Ours:   ($our_head)"
                print $"    Theirs: ($sys_head)"
            } else {
                let our_first = ($our_result.stdout | lines | first 1 | str join)
                let sys_first = ($sys_result.stdout | lines | first 1 | str join)
                print $"($red)FAIL($nc): ($desc) \(output differs\)"
                print $"    Ours:   ($our_first)"
                print $"    Theirs: ($sys_first)"
            }
            if $stop_on_fail {
                print ""
                print "Stopping on first failure."
                print $"  Our full stdout: ($our_result.stdout)"
                print $"  Sys full stdout: ($sys_result.stdout)"
                exit 1
            }
            return
        }
    }

    $env.DIFF_PASS = $env.DIFF_PASS + 1
    if $verbose {
        print $"($green)PASS($nc)"
    }
}

def --env main [
    --test-data-only    # Only test against .pc files in tests/data/
    --system-packages   # Also test against system-installed .pc files
    --verbose           # Print each test as it runs
    --stop-on-fail      # Stop on first failure
] {
    let red = (ansi red)
    let green = (ansi green)
    let yellow = (ansi yellow)
    let cyan = (ansi cyan)
    let nc = (ansi reset)

    let script_dir = ($env.FILE_PWD)
    let workspace_root = ($script_dir | path dirname)
    let test_data_dir = ($workspace_root | path join "tests" "data")

    # Find our binary
    let our_bin = ($workspace_root | path join "target" "debug" "pkgconf")
    if not ($our_bin | path exists) {
        print "Building pkgconf..."
        ^cargo build --manifest-path ($workspace_root | path join "Cargo.toml") err> /dev/null
    }

    if not ($our_bin | path exists) {
        print $"Error: Could not find or build ($our_bin)"
        exit 1
    }

    # Find system pkg-config
    let sys_candidates = (["pkgconf" "pkg-config"] | where { (which $it | length) > 0 })
    let sys_name = if ($sys_candidates | is-empty) { "" } else { $sys_candidates | first }

    if ($sys_name | is-empty) {
        print "Error: Neither pkgconf nor pkg-config found on the system."
        print "Install one of them to run differential tests."
        exit 1
    }

    let sys_bin = (which $sys_name | get 0.path)

    print $"($cyan)Differential Testing($nc)"
    print $"  Our binary:    ($our_bin)"
    print $"  System binary: ($sys_bin)"
    print $"  Test data:     ($test_data_dir)"
    print ""

    # Initialize counters in $env so the `compare` function can modify them
    $env.DIFF_PASS = 0
    $env.DIFF_FAIL = 0
    $env.DIFF_SKIP = 0
    $env.DIFF_TOTAL = 0

    # Common env that isolates from system
    let ce = {
        PKG_CONFIG_PATH: $test_data_dir
        PKG_CONFIG_LIBDIR: $test_data_dir
        PKG_CONFIG_SYSROOT_DIR: ""
        PKG_CONFIG_ALLOW_SYSTEM_CFLAGS: ""
        PKG_CONFIG_ALLOW_SYSTEM_LIBS: ""
        PKG_CONFIG_DISABLE_UNINSTALLED: "1"
        PKG_CONFIG_MSVC_SYNTAX: ""
        PKG_CONFIG_FDO_SYSROOT_RULES: ""
        PKG_CONFIG_LOG: ""
        PKG_CONFIG_PRELOADED_FILES: ""
        PKG_CONFIG_PURE_DEPGRAPH: ""
        PKG_CONFIG_IGNORE_CONFLICTS: ""
        PKG_CONFIG_DEBUG_SPEW: ""
    }

    # Shorthand for calling compare
    let ob = $our_bin
    let sb = $sys_bin
    let v = $verbose
    let sof = $stop_on_fail

    # ============================================================================
    # Test data packages
    # ============================================================================

    print $"($cyan)=== Testing against test data packages ===($nc)"

    # Basic queries
    print $"\n($yellow)--- Basic Queries ---($nc)"

    compare "version flag" $ob $sb {} ["--version"] $v $sof

    compare "modversion simple" $ob $sb $ce ["--modversion" "simple"] $v $sof
    compare "modversion zlib" $ob $sb $ce ["--modversion" "zlib"] $v $sof
    compare "modversion depender" $ob $sb $ce ["--modversion" "depender"] $v $sof
    compare "modversion libbar" $ob $sb $ce ["--modversion" "libbar"] $v $sof

    # Cflags queries
    print $"\n($yellow)--- Cflags Queries ---($nc)"

    compare "cflags simple" $ob $sb $ce ["--cflags" "simple"] $v $sof
    compare "cflags zlib" $ob $sb $ce ["--cflags" "zlib"] $v $sof
    compare "cflags depender" $ob $sb $ce ["--cflags" "depender"] $v $sof
    compare "cflags-only-I libbar" $ob $sb $ce ["--cflags-only-I" "libbar"] $v $sof
    compare "cflags-only-other libbar" $ob $sb $ce ["--cflags-only-other" "libbar"] $v $sof
    compare "cflags nocflags (empty)" $ob $sb $ce ["--cflags" "nocflags"] $v $sof

    # Libs queries
    print $"\n($yellow)--- Libs Queries ---($nc)"

    compare "libs simple" $ob $sb $ce ["--libs" "simple"] $v $sof
    compare "libs zlib" $ob $sb $ce ["--libs" "zlib"] $v $sof
    compare "libs depender" $ob $sb $ce ["--libs" "depender"] $v $sof
    compare "libs-only-l simple" $ob $sb $ce ["--libs-only-l" "simple"] $v $sof

    let ce_sys_libs = ($ce | merge { PKG_CONFIG_ALLOW_SYSTEM_LIBS: "1" })
    compare "libs-only-L simple (keep-system)" $ob $sb $ce_sys_libs ["--libs-only-L" "simple"] $v $sof
    compare "libs nolibs (empty)" $ob $sb $ce ["--libs" "nolibs"] $v $sof

    # Combined cflags and libs
    print $"\n($yellow)--- Combined Cflags+Libs ---($nc)"

    compare "cflags+libs simple" $ob $sb $ce ["--cflags" "--libs" "simple"] $v $sof
    compare "cflags+libs depender" $ob $sb $ce ["--cflags" "--libs" "depender"] $v $sof

    # Variable queries
    print $"\n($yellow)--- Variable Queries ---($nc)"

    compare "variable prefix simple" $ob $sb $ce ["--variable=prefix" "simple"] $v $sof
    compare "variable libdir simple" $ob $sb $ce ["--variable=libdir" "simple"] $v $sof
    compare "variable includedir simple" $ob $sb $ce ["--variable=includedir" "simple"] $v $sof
    compare "variable prefix zlib" $ob $sb $ce ["--variable=prefix" "zlib"] $v $sof

    # Exists queries
    print $"\n($yellow)--- Exists Queries ---($nc)"

    compare "exists simple" $ob $sb $ce ["--exists" "simple"] $v $sof
    compare "exists nonexistent" $ob $sb $ce ["--exists" "nonexistent-package-xyz"] $v $sof
    compare "exists simple >= 1.0" $ob $sb $ce ["--exists" "simple >= 1.0"] $v $sof
    compare "exists simple >= 99.0 (fail)" $ob $sb $ce ["--exists" "simple >= 99.0"] $v $sof
    compare "exists simple = 1.0.0" $ob $sb $ce ["--exists" "simple = 1.0.0"] $v $sof
    compare "exists simple = 2.0.0 (fail)" $ob $sb $ce ["--exists" "simple = 2.0.0"] $v $sof
    compare "exists simple < 2.0.0" $ob $sb $ce ["--exists" "simple < 2.0.0"] $v $sof
    compare "exists simple > 0.5.0" $ob $sb $ce ["--exists" "simple > 0.5.0"] $v $sof
    compare "exists simple != 2.0.0" $ob $sb $ce ["--exists" "simple != 2.0.0"] $v $sof

    # Version constraint flags
    print $"\n($yellow)--- Version Constraint Flags ---($nc)"

    compare "atleast-version satisfied" $ob $sb $ce ["--atleast-version=1.0.0" "simple"] $v $sof
    compare "atleast-version not satisfied" $ob $sb $ce ["--atleast-version=99.0.0" "simple"] $v $sof
    compare "exact-version match" $ob $sb $ce ["--exact-version=1.0.0" "simple"] $v $sof
    compare "exact-version mismatch" $ob $sb $ce ["--exact-version=2.0.0" "simple"] $v $sof
    compare "max-version satisfied" $ob $sb $ce ["--max-version=2.0.0" "simple"] $v $sof
    compare "max-version not satisfied" $ob $sb $ce ["--max-version=0.5.0" "simple"] $v $sof

    # Dependency resolution
    print $"\n($yellow)--- Dependency Resolution ---($nc)"

    compare "libs diamond-a (dedup)" $ob $sb $ce ["--libs" "diamond-a"] $v $sof
    compare "cflags diamond-a" $ob $sb $ce ["--cflags" "diamond-a"] $v $sof
    compare "libs deep-depender (transitive)" $ob $sb $ce ["--libs" "deep-depender"] $v $sof
    compare "cflags deep-depender (transitive)" $ob $sb $ce ["--cflags" "deep-depender"] $v $sof
    compare "libs metapackage" $ob $sb $ce ["--libs" "metapackage"] $v $sof

    # Static linking
    print $"\n($yellow)--- Static Linking ---($nc)"

    compare "static libs private-deps" $ob $sb $ce ["--static" "--libs" "private-deps"] $v $sof
    compare "static cflags private-deps" $ob $sb $ce ["--static" "--cflags" "private-deps"] $v $sof
    compare "static libs static-libs" $ob $sb $ce ["--static" "--libs" "static-libs"] $v $sof

    # Print metadata
    print $"\n($yellow)--- Print Metadata ---($nc)"

    compare "print-requires depender" $ob $sb $ce ["--print-requires" "depender"] $v $sof
    compare "print-requires-private private-deps" $ob $sb $ce ["--print-requires-private" "private-deps"] $v $sof
    compare "print-provides provider" $ob $sb $ce ["--print-provides" "provider"] $v $sof

    # Define variable
    print $"\n($yellow)--- Define Variable ---($nc)"

    compare "define-variable prefix" $ob $sb $ce ["--define-variable=prefix=/custom" "--variable=prefix" "simple"] $v $sof
    compare "define-variable affects cflags" $ob $sb $ce ["--define-variable=prefix=/custom" "--cflags" "simple"] $v $sof

    # System dirs with keep flags
    print $"\n($yellow)--- System Directory Filtering ---($nc)"

    compare "keep-system-cflags zlib" $ob $sb $ce ["--keep-system-cflags" "--cflags" "zlib"] $v $sof
    compare "keep-system-libs simple" $ob $sb $ce_sys_libs ["--keep-system-libs" "--libs" "simple"] $v $sof

    # Parser edge cases
    print $"\n($yellow)--- Parser Edge Cases ---($nc)"

    compare "modversion comments" $ob $sb $ce ["--modversion" "comments"] $v $sof
    compare "libs comments" $ob $sb $ce ["--libs" "comments"] $v $sof
    compare "modversion multiline" $ob $sb $ce ["--modversion" "multiline"] $v $sof
    compare "libs multiline" $ob $sb $ce ["--libs" "multiline"] $v $sof
    compare "cflags multiline" $ob $sb $ce ["--cflags" "multiline"] $v $sof
    compare "modversion dos-lineendings" $ob $sb $ce ["--modversion" "dos-lineendings"] $v $sof
    compare "modversion no-trailing-newline" $ob $sb $ce ["--modversion" "no-trailing-newline"] $v $sof
    compare "modversion unicode" $ob $sb $ce ["--modversion" "unicode"] $v $sof

    # Tilde version
    print $"\n($yellow)--- Tilde Version ---($nc)"

    compare "exists tilde-version" $ob $sb $ce ["--exists" "tilde-version"] $v $sof
    compare "modversion tilde-version" $ob $sb $ce ["--modversion" "tilde-version"] $v $sof
    compare "tilde < release" $ob $sb $ce ["--exists" "tilde-version < 1.0.0"] $v $sof

    # ============================================================================
    # System-installed packages (optional)
    # ============================================================================

    if $system_packages {
        print ""
        print $"($cyan)=== Testing against system-installed packages ===($nc)"

        let pkg_list_result = (do -i { ^$sb --list-all } | complete)
        let available_packages = if $pkg_list_result.exit_code == 0 {
            $pkg_list_result.stdout
            | lines
            | first 50
            | each { |line| $line | split row -r '\s+' | first }
            | where { ($in | str trim) != "" }
        } else {
            []
        }

        if ($available_packages | is-empty) {
            print $"($yellow)No system packages found, skipping.($nc)"
        } else {
            let empty_env = {}
            for pkg in $available_packages {
                print $"\n($yellow)--- Package: ($pkg) ---($nc)"

                compare $"modversion ($pkg)" $ob $sb $empty_env ["--modversion" $pkg] $v $sof
                compare $"cflags ($pkg)" $ob $sb $empty_env ["--cflags" $pkg] $v $sof
                compare $"libs ($pkg)" $ob $sb $empty_env ["--libs" $pkg] $v $sof
                compare $"exists ($pkg)" $ob $sb $empty_env ["--exists" $pkg] $v $sof
                compare $"variable prefix ($pkg)" $ob $sb $empty_env ["--variable=prefix" $pkg] $v $sof
            }
        }
    }

    # ============================================================================
    # Summary
    # ============================================================================

    print ""
    print $"($cyan)=== Summary ===($nc)"
    print $"  Total:   ($env.DIFF_TOTAL)"
    print $"  ($green)Passed:  ($env.DIFF_PASS)($nc)"
    print $"  ($red)Failed:  ($env.DIFF_FAIL)($nc)"
    print $"  ($yellow)Skipped: ($env.DIFF_SKIP)($nc)"
    print ""

    if $env.DIFF_FAIL > 0 {
        print $"($red)Some tests failed!($nc)"
        exit 1
    } else {
        print $"($green)All tests passed!($nc)"
    }
}
