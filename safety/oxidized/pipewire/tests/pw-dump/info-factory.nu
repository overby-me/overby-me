source ../helpers.nu

# Factory info block: name, type, version (uint32 not string), change-mask, props.
# Factory id 7 is libpipewire-module-metadata factory (always present).
try { ^$env.REF 7 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST 7 o+e> ($env.TMPDIR | path join actual) }
compare "pw-dump 7 (Factory info block: name/type/version/props)"
