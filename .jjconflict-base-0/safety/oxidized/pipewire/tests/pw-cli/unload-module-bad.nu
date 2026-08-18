source ../helpers.nu

try { ^$env.REF unload-module foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST unload-module foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli unload-module foo (unknown module)"
