#!/usr/bin/env nu
# build-test-binaries.nu — Compile test/test_*.mojo into standalone binaries.
#
# Each test module has a fn main() that creates one WasmInstance and
# calls all tests in that module sequentially.  Precompiling avoids
# the ~11s Mojo compilation overhead on every run.
#
# Binaries are placed in build/test-bin/.  Compilation runs in parallel
# (up to $jobs processes, default: nproc).
#
# Usage:
#   nu scripts/build-test-binaries.nu              # build all
#   nu scripts/build-test-binaries.nu -j 4         # limit to 4 jobs
#   nu scripts/build-test-binaries.nu -f            # force rebuild (ignore timestamps)
#   nu scripts/build-test-binaries.nu signals       # build only test_signals
#   nu scripts/build-test-binaries.nu signals mut   # build test_signals + test_mutations
#   nu scripts/build-test-binaries.nu -f dsl        # force rebuild test_dsl only
#
# Filter arguments are matched as substrings against source file names.
# "signals" matches "test_signals.mojo", "mut" matches "test_mutations.mojo", etc.

def main [
    --jobs (-j): int          # Max parallel compilations (default: nproc)
    --force (-f)              # Force rebuild even if binary is up-to-date
    ...filters: string        # Substring match against test module names
] {
    let script_dir = ($env.FILE_PWD)
    let project_dir = ($script_dir | path dirname)
    let core_dir = ($project_dir | path dirname | path join "core")
    let test_dir = ($core_dir | path join "test")
    let core_src_dir = ($core_dir | path join "src")
    let examples_dir = ($project_dir | path dirname | path join "examples")
    let out_dir = ($project_dir | path join "build" "test-bin")
    let wasmtime_mojo = ($project_dir | path dirname | path dirname | path join "wasmtime" "src")

    let num_jobs = if $jobs != null {
        $jobs
    } else {
        try { ^nproc | str trim | into int } catch {
            try { ^sysctl -n hw.ncpu | str trim | into int } catch { 8 }
        }
    }

    mkdir $out_dir

    # Collect source files — every test/test_*.mojo that contains fn main()
    let all_sources = (glob ($test_dir | path join "test_*.mojo")
        | where { |f| ($f | path type) == "file" }
        | where { |f| (open $f | str contains "fn main") }
    )

    # Apply filter if provided
    let sources = if ($filters | is-empty) {
        $all_sources
    } else {
        $all_sources | where { |f|
            let name = ($f | path basename)
            $filters | any { |filter| $name | str contains $filter }
        }
    }

    let total = ($sources | length)

    if $total == 0 {
        if not ($filters | is-empty) {
            print -e $"No test files matching filter\(s\): ($filters | str join ' ')"
            print -e "Available test modules:"
            for f in $all_sources {
                print -e $"  ($f | path basename | str replace '.mojo' '')"
            }
        } else {
            print -e $"No test files with fn main\(\) found in ($test_dir)"
        }
        exit 1
    }

    if not ($filters | is-empty) {
        print $"Building ($total) test binaries \(filter: ($filters | str join ' '), jobs=($num_jobs)\)..."
    } else {
        print $"Building ($total) test binaries \(jobs=($num_jobs)\)..."
    }

    # Determine which sources need building (skip up-to-date unless --force)
    let harness = ($test_dir | path join "wasm_harness.mojo")

    let to_build = $sources | where { |src|
        let name = ($src | path basename | str replace '.mojo' '')
        let bin = ($out_dir | path join $name)

        if $force {
            true
        } else if not ($bin | path exists) {
            true
        } else {
            let bin_modified = (ls -l $bin | get 0.modified)
            let needs_rebuild = [$src $harness] | where { |dep|
                ($dep | path exists) and ((ls -l $dep | get 0.modified) > $bin_modified)
            }
            ($needs_rebuild | is-not-empty)
        }
    }

    let skipped = $total - ($to_build | length)

    # Build in parallel using par-each
    let results = $to_build | par-each --threads $num_jobs { |src|
        let name = ($src | path basename | str replace '.mojo' '')
        let bin = ($out_dir | path join $name)

        let args = [-I $wasmtime_mojo -I $core_src_dir -I $examples_dir -I $test_dir -o $bin $src]
        let result = (do -i { ^mojo build ...$args } | complete)

        let combined = $"($result.stdout)($result.stderr)"

        if $result.exit_code == 0 {
            # Prefix each line of output
            if ($combined | str trim | is-not-empty) {
                $combined | lines | each { |line| $"  [($name)] ($line)" } | str join "\n" | print
            }
            { name: $name, ok: true }
        } else {
            print -e $"  FAIL: ($name)"
            if ($combined | str trim | is-not-empty) {
                $combined | lines | each { |line| $"  [($name)] ($line)" } | str join "\n" | print -e
            }
            { name: $name, ok: false }
        }
    }

    let built = ($results | where ok | length)
    let failed = ($results | where { |r| not $r.ok } | length)
    let failed_names = ($results | where { |r| not $r.ok } | get name)

    print ""
    print $"Done: ($built) built, ($skipped) skipped, ($failed) failed \(of ($total) total\)"

    if $failed > 0 {
        print ""
        print -e "Failed binaries:"
        for n in $failed_names {
            print -e $"  - ($n)"
        }
        exit 1
    }
}
