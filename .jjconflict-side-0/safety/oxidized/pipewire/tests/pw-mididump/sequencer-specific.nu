source ../helpers.nu

# Sequencer Specific meta event (0xFF 0x7F): printer uses dump_mem
# with "Sequencer" label, emitting the raw bytes as hex.
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00000001 01e0] | save -f --raw $mid
0x[4d54726b 00000009] | save -a --raw $mid
0x[00ff7f02 aabb] | save -a --raw $mid
0x[00ff2f00] | save -a --raw $mid

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-mididump sequencer-specific (0xFF 0x7F → 'Sequencer: <hex>')"
