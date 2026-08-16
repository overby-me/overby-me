source ../helpers.nu

# spa-resample doesn't declare --version (only --help/-h/-v[erbose]).
# Long-form `--version` falls through to the "unrecognized option" path.
try { ^$env.REF --version o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --version o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --version (unrecognized: not declared in long_options)"
