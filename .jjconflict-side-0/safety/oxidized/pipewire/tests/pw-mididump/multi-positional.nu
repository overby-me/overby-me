source ../helpers.nu

try { ^$env.REF /tmp/nonexistent1 /tmp/nonexistent2 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST /tmp/nonexistent1 /tmp/nonexistent2 o+e> ($env.TMPDIR | path join actual) }
compare "pw-mididump foo bar (uses first positional)"
