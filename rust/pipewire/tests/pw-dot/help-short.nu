source ../helpers.nu

# pw-dot -h (short help flag): same output as --help.
^$env.REF -h o+e> ($env.TMPDIR | path join expected)
^$env.RUST -h o+e> ($env.TMPDIR | path join actual)
compare "pw-dot/help-short"
