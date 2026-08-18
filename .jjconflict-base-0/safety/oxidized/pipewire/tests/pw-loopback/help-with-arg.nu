source ../helpers.nu

try { ^$env.REF --help=foo o+e> ($env.TMPDIR | path join c.full) }
try { ^$env.RUST --help=foo o+e> ($env.TMPDIR | path join r.full) }
^sed -E 's/pw-loopback-[0-9]+/pw-loopback-PID/g' ($env.TMPDIR | path join c.full) o> ($env.TMPDIR | path join expected)
^sed -E 's/pw-loopback-[0-9]+/pw-loopback-PID/g' ($env.TMPDIR | path join r.full) o> ($env.TMPDIR | path join actual)
compare "pw-loopback/help-with-arg"
