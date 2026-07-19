source ../helpers.nu

# Rich daemon: pw-link -i -I shows port ids.
^$env.REF -i -I o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST -i -I o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-link -i -I (rich daemon)"
