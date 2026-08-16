source ../helpers.nu

^$env.REF info 0 o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST info 0 o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-cli info 0 (Core)"
