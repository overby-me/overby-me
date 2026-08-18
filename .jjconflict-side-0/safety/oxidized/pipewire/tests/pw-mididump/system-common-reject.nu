source ../helpers.nu

# SMF spec: MTrk events are limited to channel messages (0x80-0xEF),
# sysex (0xF0/0xF7) and meta (0xFF). System Common (0xF1-0xF6) and
# Realtime (0xF8-0xFE) bytes hit C's `default: return -EINVAL` arm in
# midifile.c: the event loop stops without printing them.
#
# Build an SMF with a stray 0xF1 (MIDI Time Code Quarter Frame) byte
# and verify both tools only emit the "opened" header.
let mid = $env.TMPDIR | path join in.mid
0x[4d546864 00000006 00000001 01e0] | save -f --raw $mid
0x[4d54726b 00000008] | save -a --raw $mid
0x[00f142] | save -a --raw $mid          # delta 0, MTC Quarter Frame
0x[00ff2f00] | save -a --raw $mid        # delta 0, EndOfTrack

^$env.REF $mid o+e> ($env.TMPDIR | path join expected)
^$env.RUST $mid o+e> ($env.TMPDIR | path join actual)
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-mididump system-common 0xF1 (rejected silently like C midifile.c)"
