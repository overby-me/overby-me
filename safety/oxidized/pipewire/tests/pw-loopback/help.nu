source ../helpers.nu

# pw-loopback's help text bakes in the current PID for the default node
# name. Normalize PIDs to a sentinel before diffing.
^$env.REF --help o+e> ($env.TMPDIR | path join c.full)
^$env.RUST --help o+e> ($env.TMPDIR | path join r.full)
^sed -E 's/pw-loopback-[0-9]+/pw-loopback-PID/g' ($env.TMPDIR | path join c.full) o> ($env.TMPDIR | path join expected)
^sed -E 's/pw-loopback-[0-9]+/pw-loopback-PID/g' ($env.TMPDIR | path join r.full) o> ($env.TMPDIR | path join actual)
compare "pw-loopback/help (PID-normalized)"
