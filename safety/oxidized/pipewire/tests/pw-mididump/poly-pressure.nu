source ../helpers.nu

# SMF with polyphonic key pressure (aftertouch) and channel pressure.
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00000001 01e0] | save -f --raw $mid

# Track body, length 11:
#   4 bytes: delta=0, 0xa0 0x3c 0x40 = poly key pressure ch 0 note 60 = 64
#   3 bytes: delta=0, 0xd0 0x40    = channel pressure ch 0 = 64
#   4 bytes: delta=0, EndOfTrack
0x[4d54726b 0000000b] | save -a --raw $mid
0x[00a03c40] | save -a --raw $mid
0x[00d040] | save -a --raw $mid
0x[00ff2f00] | save -a --raw $mid

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
compare "pw-mididump/poly-pressure"
