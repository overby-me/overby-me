source ../helpers.nu

# SMF with controller, program-change, pitch-bend.
# Track body:
#   delta=0, 0xb0 0x07 0x40    = 4 bytes (CC 7 = 64)
#   delta=0, 0xc0 0x28         = 3 bytes (Program 40)
#   delta=0, 0xe0 0x00 0x40    = 4 bytes (Pitch Bend center)
#   delta=0, EndOfTrack        = 4 bytes
# Total = 15 bytes (0x0f).
let mid = (
    0x[4d 54 68 64 00 00 00 06 00 00 00 01 01 e0]
    ++ 0x[4d 54 72 6b 00 00 00 0f]
    ++ 0x[00 b0 07 40]
    ++ 0x[00 c0 28]
    ++ 0x[00 e0 00 40]
    ++ 0x[00 ff 2f 00]
)
$mid | save -f --raw ($env.TMPDIR | path join in.mid)

^$env.REF ($env.TMPDIR | path join in.mid) o+e> ($env.TMPDIR | path join expected)
^$env.RUST ($env.TMPDIR | path join in.mid) o+e> ($env.TMPDIR | path join actual)
compare "pw-mididump/controllers"
