source ../helpers.nu

# SMF spec rejects realtime bytes (0xF8-0xFE): C midifile.c returns
# -EINVAL on unknown status, so the loop stops without printing them.
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00000001 01e0] | save -f --raw $mid
0x[4d54726b 00000007] | save -a --raw $mid
0x[00f8] | save -a --raw $mid            # delta 0, Timing Clock
0x[00ff2f00] | save -a --raw $mid        # delta 0, EndOfTrack

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-mididump realtime 0xF8 (rejected silently like C midifile.c)"
