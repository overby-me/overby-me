# Shared helpers for the rust-binutils nushell test fixtures.
#
# testsuite.nix copies each fixture one directory below this file, so
# fixtures start with `source ../helpers.nu` (the same layout as this
# repository: tests/helpers.nu next to tests/<tool>/<name>.nu).

# Rewrite unstable output (nix store paths, trailing whitespace) in place.
def normalize [file: string] {
    open --raw $file
    | str replace -a -r '/nix/store/[a-z0-9]{32}-[^/\s]+/bin/[a-z+]+' TOOL
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
        print "--- expected (GNU reference) ---"
        print (open --raw $ref_out)
        print "--- actual (rust-binutils) ---"
        print (open --raw $rust_out)
        exit 1
    }
}
