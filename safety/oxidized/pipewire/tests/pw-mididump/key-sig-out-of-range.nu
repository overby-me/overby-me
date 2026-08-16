source ../helpers.nu

# Key Signature meta (0xFF 0x59) with a "negative" sf byte. C reads
# meta[0] as int (implicit u8→int conversion = positive); -7 becomes
# 249, classified "sharps", abs(249)=249, table lookup is clamped to
# 18 → "Unknown major". Our impl now matches the u8-as-int quirk.
let mid = (
    0x[4d 54 68 64 00 00 00 06 00 00 00 01 01 e0]
    ++ 0x[4d 54 72 6b 00 00 00 0a]
    ++ 0x[00 ff 59 02 f9 00]     # sf=-7 (0xF9), major
    ++ 0x[00 ff 2f 00]
)
$mid | save -f --raw ($env.TMPDIR | path join in.mid)

^$env.REF ($env.TMPDIR | path join in.mid) o+e> ($env.TMPDIR | path join expected)
^$env.RUST ($env.TMPDIR | path join in.mid) o+e> ($env.TMPDIR | path join actual)
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-mididump key-sig-out-of-range (negative sf byte → '249 sharps: Unknown major')"
