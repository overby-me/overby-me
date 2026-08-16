source ../helpers.nu

# Channel Pressure / Aftertouch (0xD0): channel event with 1 data
# byte (the pressure value). Distinct from polyphonic key pressure
# (0xA0) which has 2 data bytes (note + pressure).
let mid = (
    0x[4d 54 68 64 00 00 00 06 00 00 00 01 01 e0]
    ++ 0x[4d 54 72 6b 00 00 00 08]
    ++ 0x[00 d0 64]          # delta 0, Channel Pressure ch 1, pressure 100
    ++ 0x[00 ff 2f 00]       # delta 0, EndOfTrack
)
$mid | save -f --raw ($env.TMPDIR | path join in.mid)

^$env.REF ($env.TMPDIR | path join in.mid) o+e> ($env.TMPDIR | path join expected)
^$env.RUST ($env.TMPDIR | path join in.mid) o+e> ($env.TMPDIR | path join actual)
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-mididump channel-pressure (0xD0 with 1 data byte)"
