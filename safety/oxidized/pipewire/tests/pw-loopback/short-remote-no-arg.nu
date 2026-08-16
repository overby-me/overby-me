source ../helpers.nu

try { ^$env.REF -r o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -r o+e> ($env.TMPDIR | path join actual) }
^sed -i 's/pw-loopback-[0-9]*/pw-loopback-PID/g' ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-loopback -r (short form requires arg)"
