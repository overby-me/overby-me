source ../helpers.nu

# pw-sysex -V is an unrecognized short option (the long --version has no
# short alias). C tool prints `invalid option -- 'V'` then the help.
# Both exit non-zero, so we tolerate the failure with try.
try { ^$env.REF -V o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -V o+e> ($env.TMPDIR | path join actual) }
compare "pw-sysex/invalid-V"
