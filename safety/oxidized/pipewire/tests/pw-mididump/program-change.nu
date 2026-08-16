source ../helpers.nu

# Program Change (0xC0): channel event with 1 data byte. The C tool
# (midievent.c) maps program numbers to GM instrument names.
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00000001 01e0] | save -f --raw $mid
0x[4d54726b 00000009] | save -a --raw $mid
0x[00c001] | save -a --raw $mid          # delta 0, Program Change ch 1, program 1 (Bright Acoustic)
0x[00ff2f00] | save -a --raw $mid        # delta 0, EndOfTrack

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-mididump program-change (0xC0 with GM instrument name)"
