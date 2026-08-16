source ../helpers.nu

# SMF with copyright + marker + cue point meta events.
# Track body, length:
#   delta=0, 0xff 0x02 0x05 "(c)26"  = 9 bytes
#   delta=0, 0xff 0x06 0x05 "start"  = 9 bytes
#   delta=0, 0xff 0x07 0x05 "cuept"  = 9 bytes
#   delta=0, EndOfTrack              = 4 bytes
# Total = 31 bytes.
# (We use equal-length payloads to avoid C's read-past-length UB when
# its malloc'd buffer for a shorter event still holds the previous
# longer event's bytes; this would print "cuert" for a 3-byte "cue"
# after a 5-byte "start" because the unterminated payload reads into
# the leftover heap data.)
let mid = (
    0x[4d 54 68 64 00 00 00 06 00 00 00 01 01 e0]
    ++ 0x[4d 54 72 6b 00 00 00 1f]
    ++ 0x[00 ff 02 05 28 63 29 32 36]     # "(c)26"
    ++ 0x[00 ff 06 05 73 74 61 72 74]     # "start"
    ++ 0x[00 ff 07 05 63 75 65 70 74]     # "cuept"
    ++ 0x[00 ff 2f 00]
)
$mid | save -f --raw ($env.TMPDIR | path join in.mid)

^$env.REF ($env.TMPDIR | path join in.mid) o+e> ($env.TMPDIR | path join expected)
^$env.RUST ($env.TMPDIR | path join in.mid) o+e> ($env.TMPDIR | path join actual)
compare "pw-mididump/copyright-meta"
