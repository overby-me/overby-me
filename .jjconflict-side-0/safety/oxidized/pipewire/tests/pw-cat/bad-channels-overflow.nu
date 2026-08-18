source ../helpers.nu

# C: ret = atoi("999999999999"); i32 wrap-around. Whatever the wrapped
# value, the error format is 'error: bad channels <wrapped>'.
try { ^$env.REF --channels=999999999999 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --channels=999999999999 o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat --channels=999999999999 (atoi wraps to negative i32)"
