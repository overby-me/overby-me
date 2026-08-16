source ../helpers.nu

try { ^$env.REF --raw -p missing-foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --raw -p missing-foo o+e> ($env.TMPDIR | path join actual) }
"exit=0\n" | save -a --raw ($env.TMPDIR | path join expected)
"exit=0\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-cat --raw -p missing-foo (raw mode → 'raw: can't open file')"
