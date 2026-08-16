source ../helpers.nu

# SMF with pitch wheel events.
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00000001 01e0] | save -f --raw $mid

# 0xe0 ll mh (pitch wheel ch 0, value)
# Center: 0x40 0x00 (0x2000 = 8192)
# Up max: 0x7f 0x7f (0x3fff = 16383)
0x[4d54726b 0000000c] | save -a --raw $mid
0x[00e04000] | save -a --raw $mid
0x[00e07f7f] | save -a --raw $mid
0x[00ff2f00] | save -a --raw $mid

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
compare "pw-mididump/pitch-wheel"
