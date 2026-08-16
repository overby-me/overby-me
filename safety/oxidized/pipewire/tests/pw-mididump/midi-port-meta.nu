source ../helpers.nu

# Midi Port meta event (0xFF 0x21): single-byte payload; printer
# formats with zero-padded 3-digit number ("%03d").
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00000001 01e0] | save -f --raw $mid
0x[4d54726b 00000008] | save -a --raw $mid
0x[00ff2101 05] | save -a --raw $mid
0x[00ff2f00] | save -a --raw $mid

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-mididump midi-port-meta (0xFF 0x21 → 'Midi Port: 005')"
