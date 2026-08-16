source ../helpers.nu

try { ^$env.REF --remote o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --remote o+e> ($env.TMPDIR | path join actual) }
^sed -i 's/pw-loopback-[0-9]*/pw-loopback-PID/g' ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-loopback --remote (long form requires arg)"
