source ../helpers.nu

# C: ret = atoi("-3") = -3; ret <= 0 → 'error: bad channels -3'.
try { ^$env.REF --channels=-3 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --channels=-3 o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat --channels=-3 (negative → 'bad channels -3')"
