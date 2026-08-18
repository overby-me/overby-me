source ../helpers.nu

# C's find_global uses spa_atou32(arg, &id, 0) which auto-detects base
# from prefix (0x → hex, 0 → octal, decimal otherwise). So `info 0x10`
# looks up id 16. Our Rust now matches via parse_u32_autobase. In the
# nix sandbox neither connects to a daemon, so both fail the same way.
try { ^$env.REF "info 0x10" o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST "info 0x10" o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli info 0x10 (hex id parsing)"
