source ../helpers.nu

# pw-metadata -V (short version flag): same output as --version.
^$env.REF -V o+e> ($env.TMPDIR | path join expected)
^$env.RUST -V o+e> ($env.TMPDIR | path join actual)
compare "pw-metadata/version-short"
