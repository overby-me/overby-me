source ../helpers.nu

# pw-mididump on a tiny format-0 SMF: NoteOn → NoteOff at 120 ticks
# (= 0.125s at 480 division / default 120 BPM tempo).

# SMF bytes: MThd + format=0 + 1 track + division=480, MTrk with NoteOn
# (0x90 60 100), NoteOff (0x80 60 0), EndOfTrack.
let mid = (
    0x[4d 54 68 64 00 00 00 06 00 00 00 01 01 e0]
    ++ 0x[4d 54 72 6b 00 00 00 0d]
    ++ 0x[00 90 3c 64]     # delta 0, NoteOn C4 vel 100
    ++ 0x[78 80 3c 00]     # delta 120, NoteOff C4
    ++ 0x[00 ff 2f 00]     # delta 0, EndOfTrack
)
$mid | save -f --raw ($env.TMPDIR | path join in.mid)

^$env.REF ($env.TMPDIR | path join in.mid) o+e> ($env.TMPDIR | path join expected)
^$env.RUST ($env.TMPDIR | path join in.mid) o+e> ($env.TMPDIR | path join actual)
compare "pw-mididump/basic"
