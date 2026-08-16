source ../helpers.nu

# Look up first global of type `Profiler` by substring match.
try { ^$env.REF info Profiler o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST info Profiler o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli info Profiler (by-name)"
