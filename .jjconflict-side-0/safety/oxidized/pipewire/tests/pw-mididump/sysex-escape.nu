source ../helpers.nu

# 0xF7 alone (without prior 0xF0) is the SMF "escape" sequence: raw
# bytes treated as a sysex chunk. The parser reads varlen length then
# that many payload bytes, and the printer emits `SysEx: f7 <bytes>`.
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00000001 01e0] | save -f --raw $mid
0x[4d54726b 00000009] | save -a --raw $mid
0x[00f702 aabb] | save -a --raw $mid     # delta 0, escape, len 2, payload AA BB
0x[00ff2f00] | save -a --raw $mid

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-mididump sysex-escape (0xF7 escape sequence)"
