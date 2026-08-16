source ../helpers.nu

# C: i32 atoi overflow → 'bad rate -727379969'.
try { ^$env.REF --rate=999999999999 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --rate=999999999999 o+e> ($env.TMPDIR | path join actual) }
# bash recorded `$?` after `|| true`, which is always 0.
"exit=0\n" | save -a --raw ($env.TMPDIR | path join expected)
"exit=0\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-cat --rate=999999999999 (i32 atoi overflow → 'bad rate -727379969')"
