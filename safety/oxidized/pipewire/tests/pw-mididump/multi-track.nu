source ../helpers.nu

# format=1 SMF with 2 tracks. Each track has a NoteOn+NoteOff.
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00010002 01e0] | save -f --raw $mid

# Track 1: 9 bytes, delta 0 NoteOn, delta 96 NoteOff, EndOfTrack.
0x[4d54726b 0000000c] | save -a --raw $mid
0x[00903c64] | save -a --raw $mid
0x[60803c00] | save -a --raw $mid
0x[00ff2f00] | save -a --raw $mid

# Track 2: same shape but channel 1, note D4.
0x[4d54726b 0000000c] | save -a --raw $mid
0x[00913e64] | save -a --raw $mid
0x[60813e00] | save -a --raw $mid
0x[00ff2f00] | save -a --raw $mid

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
compare "pw-mididump/multi-track"
