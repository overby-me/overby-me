#!/usr/bin/env nu
# run-test-binaries.nu — Run precompiled test binaries in parallel.
#
# Executes all binaries in build/test-bin/ concurrently, waits for each
# in order, and reports pass/fail.  Returns non-zero if any binary fails.
#
# Usage:
#   nu scripts/run-test-binaries.nu              # run all
#   nu scripts/run-test-binaries.nu -v            # verbose (show all output)
#   nu scripts/run-test-binaries.nu signals       # run only test_signals
#   nu scripts/run-test-binaries.nu signals mut   # run test_signals + test_mutations
#   nu scripts/run-test-binaries.nu -v dsl        # verbose, only test_dsl
#
# Filter arguments are matched as substrings against binary names.
# "signals" matches "test_signals", "mut" matches "test_mutations", etc.
#
# Or via justfile:
#   just test-run
#   just test-run signals

def main [
    --verbose (-v)  # Show full output from each binary
    ...filters: string  # Substring match against test module names
] {
    let script_dir = ($env.FILE_PWD)
    let project_dir = ($script_dir | path dirname)
    let bin_dir = ($project_dir | path join "build" "test-bin")

    if not ($bin_dir | path exists) {
        print -e "No test binaries found.  Run 'just test-build' first."
        exit 1
    }

    # Collect binaries, applying filter if provided
    let all_binaries = (glob ($bin_dir | path join "test_*")
        | where { |bin| ($bin | path type) == "file" }
        | where { |bin|
            # Check executable permission
            let mode = (ls -l $bin | get 0.mode)
            $mode =~ "x"
        }
    )

    let binaries = if ($filters | is-empty) {
        $all_binaries
    } else {
        $all_binaries | where { |bin|
            let name = ($bin | path basename)
            $filters | any { |filter| $name =~ $filter }
        }
    }

    let total = ($binaries | length)

    if $total == 0 {
        if ($filters | is-not-empty) {
            print -e $"No test binaries matching filter\(s\): ($filters | str join ' ')"
            print -e "Available binaries:"
            glob ($bin_dir | path join "test_*")
                | where { |bin|
                    let mode = (ls -l $bin | get 0.mode)
                    $mode =~ "x"
                }
                | each { |bin| print -e $"  ($bin | path basename)" }
        } else {
            print -e $"No executable test binaries found in ($bin_dir)"
            print -e "Run 'just test-build' first."
        }
        exit 1
    }

    if ($filters | is-not-empty) {
        print $"Running ($total) test binaries \(filter: ($filters | str join ' ')\)..."
    } else {
        print $"Running ($total) test binaries..."
    }
    print ""

    let start_time = (date now)

    # Run all binaries in parallel using par-each, collecting results
    let results = ($binaries | par-each { |bin|
        let name = ($bin | path basename)
        # Run from the project directory so "build/out.wasm" paths resolve correctly
        let result = do { cd $project_dir; ^$bin } | complete
        {
            name: $name
            exit_code: $result.exit_code
            output: $"($result.stdout)($result.stderr)"
        }
    })

    # Report results in order
    mut passed = 0
    mut failed = 0
    mut failed_names = []

    for result in $results {
        if $result.exit_code == 0 {
            $passed += 1
            if $verbose {
                print $"  ✓ ($result.name)"
                $result.output | lines | each { |line| print $"    ($line)" }
            } else {
                let non_empty_lines = ($result.output
                    | lines
                    | where { |line| ($line | str trim) != "" }
                )
                let summary = if ($non_empty_lines | is-empty) {
                    "ok"
                } else {
                    $non_empty_lines | last
                }
                print $"  ✓ ($result.name) — ($summary)"
            }
        } else {
            $failed += 1
            $failed_names = ($failed_names | append $result.name)
            print $"  ✗ ($result.name) — FAILED"
            $result.output | lines | each { |line| print $"    ($line)" }
        }
    }

    let end_time = (date now)
    let elapsed = ($end_time - $start_time)

    print ""

    let elapsed_ms = ($elapsed | into int | $in // 1_000_000)
    if $elapsed_ms < 1000 {
        print $"Completed in ($elapsed_ms)ms: ($passed) passed, ($failed) failed \(of ($total) total\)"
    } else {
        let elapsed_s = ($elapsed_ms | into float) / 1000.0
        let elapsed_str = ($elapsed_s | math round --precision 1)
        print $"Completed in ($elapsed_str)s: ($passed) passed, ($failed) failed \(of ($total) total\)"
    }

    if $failed > 0 {
        print ""
        print -e "Failed:"
        for name in $failed_names {
            print -e $"  - ($name)"
        }
        exit 1
    }
}
