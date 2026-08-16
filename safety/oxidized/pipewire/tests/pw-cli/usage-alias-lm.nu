source ../helpers.nu

try { ^$env.REF lm o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST lm o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli lm (alias usage)"
