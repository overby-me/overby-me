source ../helpers.nu

# SMF with tempo + time signature meta events. Avoid bash $() command
# substitution which strips NULs: write bytes directly to the file.
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00000001 01e0] | save -f --raw $mid

# Track body, length 25:
#   delta=0, 0xff 0x51 0x03 tempo (3 bytes)  = 7 bytes
#   delta=0, 0xff 0x58 0x04 time-sig (4)     = 8 bytes
#   delta=0, 0xff 0x59 0x02 key-sig (2)      = 6 bytes
#   delta=0, EndOfTrack                      = 4 bytes
# Total = 25 bytes (0x19).
0x[4d54726b 00000019] | save -a --raw $mid
# tempo: 500000 us/qn = 120 BPM
0x[00ff5103 07a120] | save -a --raw $mid
# time sig: 4/4, 24 clocks/click, 8 32nds/qtr
0x[00ff5804 04021808] | save -a --raw $mid
# key sig: 0 sharps, major
0x[00ff5902 0000] | save -a --raw $mid
# end
0x[00ff2f00] | save -a --raw $mid

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
compare "pw-mididump/tempo-meta"
