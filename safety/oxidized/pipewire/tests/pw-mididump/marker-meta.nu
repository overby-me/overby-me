source ../helpers.nu

# Marker meta event (0x06): same printer path as Text/Copyright/Track/
# Instrument/Lyric/Cue (case 0x01..0x09 in midievent.c). Single event
# only so we don't trigger C's `%s` buffer-reuse bug that reads stale
# bytes past the payload boundary on multi-event meta sequences.
let mid = (
    0x[4d 54 68 64 00 00 00 06 00 00 00 01 01 e0]
    ++ 0x[4d 54 72 6b 00 00 00 0c]
    ++ 0x[00 ff 06 05 56 65 72 73 65]     # Marker "Verse"
    ++ 0x[00 ff 2f 00]
)
$mid | save -f --raw ($env.TMPDIR | path join in.mid)

^$env.REF ($env.TMPDIR | path join in.mid) o+e> ($env.TMPDIR | path join expected)
^$env.RUST ($env.TMPDIR | path join in.mid) o+e> ($env.TMPDIR | path join actual)
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-mididump marker-meta (0x06)"
