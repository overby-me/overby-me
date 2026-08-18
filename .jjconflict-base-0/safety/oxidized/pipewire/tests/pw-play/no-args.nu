source ../helpers.nu

# pw-play with no args prints an error then the help block.
try { ^$env.REF o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST o+e> ($env.TMPDIR | path join actual) }
compare "pw-play/no-args"
