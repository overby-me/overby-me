source ../helpers.nu

# spa-json-dump doesn't declare --version. Long-form rejected.
try { ^$env.REF --version o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --version o+e> ($env.TMPDIR | path join actual) }
compare "spa-json-dump --version (unrecognized: not declared)"
