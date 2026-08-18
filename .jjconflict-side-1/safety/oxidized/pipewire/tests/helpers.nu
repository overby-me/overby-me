# Shared helpers for the oxidized-pipewire nushell test fixtures.
#
# The testsuite harnesses copy each fixture one directory below this file,
# mirroring the repository layout (tests/helpers.nu next to
# tests/<tool>/<name>.nu), so fixtures start with `source ../helpers.nu`.

# Rewrite unstable output (nix store paths, trailing whitespace) in place.
def normalize [file: string] {
    open --raw $file
    | str replace -a -r '/nix/store/[a-z0-9]{32}-[^/\s]+/bin/[^\s]+' TOOL
    | str replace -a -r '/nix/store/[a-z0-9]{32}-[^\s]+' NIXPATH
    | str replace -a -r '(?m)[ \t\r]+$' ''
    | save -f --raw $file
}

# Diff the normalized reference and rust outputs; exit 1 on mismatch.
def compare [label: string] {
    let ref_out = $env.TMPDIR | path join expected
    let rust_out = $env.TMPDIR | path join actual
    normalize $ref_out
    normalize $rust_out
    let res = do { ^diff --text $rust_out $ref_out } | complete
    if $res.exit_code == 0 {
        print $"PASS: ($label)"
    } else {
        print $res.stdout
        print $"FAIL: ($label)"
        print "--- expected (C reference) ---"
        print (open --raw $ref_out)
        print "--- actual (oxidized-pipewire) ---"
        print (open --raw $rust_out)
        exit 1
    }
}

# Tool-agnostic structural assertion for tests that don't byte-match.
def assert_grep [label: string, pattern: string, file: string] {
    if (do { ^grep -qE $pattern $file } | complete).exit_code == 0 {
        print $"PASS: ($label)"
    } else {
        print $"FAIL: ($label) \(pattern ($pattern) not found in ($file))"
        print "--- contents ---"
        print (open --raw $file)
        exit 1
    }
}

# Show the tail of the daemon log (daemon suites only).
def print_daemon_log [] {
    print "--- daemon log (tail) ---"
    try {
        print (open --raw ($env.TMPDIR | path join daemon.log) | lines | last 40 | str join "\n")
    }
}
