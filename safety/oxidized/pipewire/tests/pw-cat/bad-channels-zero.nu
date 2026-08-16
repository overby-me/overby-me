source ../helpers.nu

# C: ret = atoi("0") = 0; ret <= 0 → 'error: bad channels 0'.
try { ^$env.REF --channels=0 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --channels=0 o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat --channels=0 (ret <= 0 → 'bad channels 0')"
