source ../helpers.nu

# SMF with key signature meta event.
# 0xff 0x59 0x02 sf mi
# sf = sharps/flats (-7..7), mi = 0=major 1=minor.
# 2 sharps, major (D major).
let mid = (
    0x[4d 54 68 64 00 00 00 06 00 00 00 01 01 e0]
    ++ 0x[4d 54 72 6b 00 00 00 0a]
    ++ 0x[00 ff 59 02 02 00]
    ++ 0x[00 ff 2f 00]
)
$mid | save -f --raw ($env.TMPDIR | path join in.mid)

^$env.REF ($env.TMPDIR | path join in.mid) o+e> ($env.TMPDIR | path join expected)
^$env.RUST ($env.TMPDIR | path join in.mid) o+e> ($env.TMPDIR | path join actual)
compare "pw-mididump/key-signature"
