source ../helpers.nu

# Filter by name: pw-metadata --list -n settings should print only
# the matching metadata. Both binaries identical.
^$env.REF --list -n settings o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join "expected.err")
^$env.RUST --list -n settings o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join "actual.err")
compare "pw-metadata --list -n settings (rich daemon)"
