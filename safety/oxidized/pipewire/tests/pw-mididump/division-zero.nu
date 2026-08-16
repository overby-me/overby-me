source ../helpers.nu

# SMF with division=0 (invalid per spec, but produced by some tools).
# C's midifile divides by info.division giving NaN/inf; printf "%f"
# emits "nan"/"-nan" (signed). Our impl now matches C's NaN
# propagation rather than clamping to 0.
let mid = (
    0x[4d 54 68 64 00 00 00 06 00 00 00 01 00 00]
    ++ 0x[4d 54 72 6b 00 00 00 04]
    ++ 0x[00 ff 2f 00]
)
$mid | save -f --raw ($env.TMPDIR | path join in.mid)

^$env.REF ($env.TMPDIR | path join in.mid) o+e> ($env.TMPDIR | path join expected)
^$env.RUST ($env.TMPDIR | path join in.mid) o+e> ($env.TMPDIR | path join actual)
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-mididump division-zero (sec divides by 0 → nan)"
