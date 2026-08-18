source ../helpers.nu

try { ^$env.REF connect new-name o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST connect new-name o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli connect <bad-name>"
