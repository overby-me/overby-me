source ../helpers.nu

try { ^$env.REF --bogus o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --bogus o+e> ($env.TMPDIR | path join actual) }
# bash read $? after `|| true`, so the recorded exit code is always 0.
"exit=0\n" | save -a --raw ($env.TMPDIR | path join expected)
"exit=0\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-cli --bogus exits 255 (matches C)"
