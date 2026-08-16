source ../helpers.nu

# Rich daemon: pw-link -i -v adds object.path and port.alias indented
# under each port name.
^$env.REF -i -v o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST -i -v o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-link -i -v (rich daemon)"
