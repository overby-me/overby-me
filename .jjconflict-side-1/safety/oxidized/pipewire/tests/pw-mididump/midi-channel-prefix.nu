source ../helpers.nu

# SMF with MIDI channel prefix meta event.
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00000001 01e0] | save -f --raw $mid

# 0xff 0x20 0x01 cc (MIDI channel prefix)
# This binds subsequent meta events to channel cc.
0x[4d54726b 00000009] | save -a --raw $mid
0x[00ff2001 05] | save -a --raw $mid
0x[00ff2f00] | save -a --raw $mid

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
compare "pw-mididump/midi-channel-prefix"
