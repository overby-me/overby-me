source ../helpers.nu

try { ^$env.REF --raw=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --raw=foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-play/raw-with-arg"
