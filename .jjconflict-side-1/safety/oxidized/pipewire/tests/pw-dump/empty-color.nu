source ../helpers.nu

# pw-dump --color= (empty value via `=`). C's getopt sets optarg to ""
# for `--color=`; the empty value doesn't match auto/never/always so we
# hit the "Unknown color: " error path. Distinct from bare `--color`
# which uses the auto default.
try { ^$env.REF "--color=" o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST "--color=" o+e> ($env.TMPDIR | path join actual) }
compare "pw-dump --color= (empty value: 'Unknown color: ')"
