source ../helpers.nu

try { ^$env.REF -- /etc /tmp o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -- /etc /tmp o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample -- /etc /tmp (-- terminator + dirs)"
