source ../helpers.nu

# SMF with a SysEx (System Exclusive) event.
# Avoid null bytes in shell variables: write body directly to file.
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00000001 01e0] | save -f --raw $mid

# Track body, length 13 bytes:
#   8 bytes: SysEx event with 5-byte payload (length-byte + 4 data bytes)
#   4 bytes: EndOfTrack meta
#   delta-time bytes: 1+1 = 2
# Total = 13. We hard-code MTrk length.
0x[4d54726b 0000000d] | save -a --raw $mid
# delta=0, 0xf0, length=4, data 0x43 0x12 0x01 0xf7 (no internal NULs).
0x[00f004431201f7] | save -a --raw $mid
# delta=0, EndOfTrack
0x[00ff2f00] | save -a --raw $mid

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
compare "pw-mididump/sysex"
