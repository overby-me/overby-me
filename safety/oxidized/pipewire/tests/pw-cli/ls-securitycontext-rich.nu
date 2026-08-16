source ../helpers.nu

^$env.REF ls SecurityContext o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST ls SecurityContext o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-cli ls SecurityContext (rich daemon)"
