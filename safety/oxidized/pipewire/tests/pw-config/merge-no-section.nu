source ../helpers.nu

try { ^$env.REF merge o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST merge o+e> ($env.TMPDIR | path join actual) }
compare "pw-config/merge-no-section"
