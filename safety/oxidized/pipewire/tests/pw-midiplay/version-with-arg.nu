source ../helpers.nu

try { ^$env.REF --version=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --version=foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-midiplay/version-with-arg"
