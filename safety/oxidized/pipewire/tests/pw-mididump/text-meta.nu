source ../helpers.nu

# Text meta events: track-name, instrument, lyric.
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00000001 01e0] | save -f --raw $mid

# Track body:
#   delta=0, 0xff 0x03 0x04 "test"     = 8 bytes
#   delta=0, 0xff 0x04 0x05 "piano"    = 9 bytes
#   delta=0, 0xff 0x05 0x02 "la"       = 6 bytes
#   delta=0, EndOfTrack                = 4 bytes
# Total = 27 bytes (0x1b).
0x[4d54726b 0000001b] | save -a --raw $mid
0x[00ff0304 74657374] | save -a --raw $mid         # "test"
0x[00ff0405 7069616e6f] | save -a --raw $mid       # "piano"
0x[00ff0502 6c61] | save -a --raw $mid             # "la"
0x[00ff2f00] | save -a --raw $mid

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
compare "pw-mididump/text-meta"
