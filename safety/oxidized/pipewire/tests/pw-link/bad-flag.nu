source ../helpers.nu

# Unrecognized option: getopt_long emits standard `<argv0>: unrecognized
# option '<flag>'` to stderr, then the help block to stdout.
try { ^$env.REF --bad-flag o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --bad-flag o+e> ($env.TMPDIR | path join actual) }
compare "pw-link --bad-flag"
