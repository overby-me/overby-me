source ../helpers.nu

^$env.REF info 4 o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST info 4 o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-cli info 4 (Factory, rich daemon)"
