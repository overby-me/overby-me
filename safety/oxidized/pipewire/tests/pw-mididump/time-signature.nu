source ../helpers.nu

# Time Signature meta (0x58): 4-byte payload: numerator, denominator
# (2^N), MIDI clocks per click, 32nd notes per quarter note. C prints
# `Time Signature: <num>/<2^denom>, <clocks> clocks per click, <32nds>
# notated 32nd notes per quarter note`.
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00000001 01e0] | save -f --raw $mid
0x[4d54726b 0000000b] | save -a --raw $mid
# 6/8 time, 24 clocks/click, 8 notated 32nds/quarter
0x[00ff5804 06031808] | save -a --raw $mid
0x[00ff2f00] | save -a --raw $mid

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-mididump time-signature (0xFF 0x58 → 6/8 with click+32nds)"
