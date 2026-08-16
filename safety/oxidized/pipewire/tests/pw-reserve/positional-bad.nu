source ../helpers.nu

# pw-reserve doesn't take positional args; without `-n NAME` it errors.
try { ^$env.REF foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-reserve foo (positional ignored, valid-name check fails)"
