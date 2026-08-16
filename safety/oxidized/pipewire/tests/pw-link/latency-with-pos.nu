source ../helpers.nu

try { ^$env.REF -t foo bar o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -t foo bar o+e> ($env.TMPDIR | path join actual) }
compare "pw-link -t foo bar (latency mode skips link)"
