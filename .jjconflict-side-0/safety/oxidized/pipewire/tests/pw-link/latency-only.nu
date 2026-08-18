source ../helpers.nu

# pw-link -t alone (without -i/-o/-l): C does nothing, no per-port
# iteration since neither LIST_PORTS nor LIST_LINKS is set.
^$env.REF -t o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST -t o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-link -t alone (no output)"
