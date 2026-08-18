source ../helpers.nu

try { ^$env.REF a o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST a o+e> ($env.TMPDIR | path join actual) }
compare "pw-link a (1 positional -> error)"
