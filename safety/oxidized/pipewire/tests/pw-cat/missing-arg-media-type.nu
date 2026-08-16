source ../helpers.nu

try { ^$env.REF --media-type o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --media-type o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat/missing-arg-media-type"
