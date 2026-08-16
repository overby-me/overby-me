source ../helpers.nu

try { ^$env.REF i Audio o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST i Audio o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli i Audio (alias error format)"
