source ../helpers.nu

try { ^$env.REF --rate=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --rate=foo o+e> ($env.TMPDIR | path join actual) }
^sed -i 's/pw-loopback-[0-9]*/pw-loopback-PID/g' ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-loopback --rate=foo (unknown long option)"
