source ../helpers.nu

# pw-config --color= (empty value via `=`). Same C behavior as pw-dump:
# empty optarg rejected because it isn't auto/never/always.
try { ^$env.REF "--color=" o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST "--color=" o+e> ($env.TMPDIR | path join actual) }
compare "pw-config --color= (empty value → 'Unknown color: ')"
