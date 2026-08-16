source ../helpers.nu

# SMF using running status: status byte 0x90 (NoteOn ch 0) sent once,
# subsequent events omit it.
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00000001 01e0] | save -f --raw $mid

# Track body:
#   delta=0, 0x90 60 100   = 4 bytes (NoteOn full)
#   delta=120, 64 100      = 3 bytes (running status)
#   delta=0, 60 0          = 3 bytes (running status)
#   delta=0, EndOfTrack    = 4 bytes
# Total = 14 bytes (0x0e).
0x[4d54726b 0000000e] | save -a --raw $mid
0x[00903c64] | save -a --raw $mid
0x[784064] | save -a --raw $mid
0x[003c00] | save -a --raw $mid
0x[00ff2f00] | save -a --raw $mid

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
compare "pw-mididump/running-status"
