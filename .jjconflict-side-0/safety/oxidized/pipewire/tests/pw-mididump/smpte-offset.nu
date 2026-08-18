source ../helpers.nu

# SMF with SMPTE offset meta event.
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00000001 01e0] | save -f --raw $mid

# 0xff 0x54 0x05 hr mn se fr ff (SMPTE offset)
0x[4d54726b 0000000d] | save -a --raw $mid
0x[00ff5405 0102030405] | save -a --raw $mid
0x[00ff2f00] | save -a --raw $mid

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
compare "pw-mididump/smpte-offset"
