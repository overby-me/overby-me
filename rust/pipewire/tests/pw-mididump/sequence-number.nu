source ../helpers.nu

# SMF with sequence number meta event (FF 00 02 ssss).
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00000001 01e0] | save -f --raw $mid

# 0xff 0x00 0x02 ss ss (sequence number = 0x0042 = 66)
0x[4d54726b 0000000a] | save -a --raw $mid
0x[00ff0002 0042] | save -a --raw $mid
0x[00ff2f00] | save -a --raw $mid

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
compare "pw-mididump/sequence-number"
