source ../helpers.nu

try { ^$env.REF /tmp o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST /tmp o+e> ($env.TMPDIR | path join actual) }
compare "pw-mididump /tmp (directory → Invalid argument)"
