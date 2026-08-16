source ../helpers.nu

# pw-link -o with a pattern that doesn't match any output port produces
# no output (rich daemon has no output ports anyway).
^$env.REF -o nonexistent o+e> ($env.TMPDIR | path join expected)
^$env.RUST -o nonexistent o+e> ($env.TMPDIR | path join actual)
compare "pw-link -o nonexistent (rich daemon, empty)"
