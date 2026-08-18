source ../helpers.nu

# pw-dump -V (short version flag): same output as --version.
^$env.REF -V o+e> ($env.TMPDIR | path join expected)
^$env.RUST -V o+e> ($env.TMPDIR | path join actual)
compare "pw-dump/version-short"
