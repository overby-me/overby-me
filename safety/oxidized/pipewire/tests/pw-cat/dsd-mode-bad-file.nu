source ../helpers.nu

# `pw-cat --dsd -p` switches to TYPE_DSD; C uses dsdfile loader.
try { ^$env.REF --dsd -p /tmp/nonexistent-dsd-file o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --dsd -p /tmp/nonexistent-dsd-file o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat --dsd -p (uses dsdfile error format)"
