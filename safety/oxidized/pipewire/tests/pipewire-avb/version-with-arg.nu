source ../helpers.nu

try { ^$env.REF --version=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --version=foo o+e> ($env.TMPDIR | path join actual) }
compare "pipewire-avb/version-with-arg"
