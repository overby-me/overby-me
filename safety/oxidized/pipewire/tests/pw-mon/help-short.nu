source ../helpers.nu

# pw-mon -h (short help flag): same output as --help.
^$env.REF -h o+e> ($env.TMPDIR | path join expected)
^$env.RUST -h o+e> ($env.TMPDIR | path join actual)
compare "pw-mon/help-short"
