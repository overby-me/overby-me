source ../helpers.nu

# `info all` against the rich daemon: every interface gets exercised,
# including the null-audio-sink Node. Strip the connecting client section
# (its props differ per session).
^$env.REF info all o> ($env.TMPDIR | path join c.full) e> ($env.TMPDIR | path join c.err)
^$env.RUST info all o> ($env.TMPDIR | path join r.full) e> ($env.TMPDIR | path join r.err)

def strip_client [inp: string, out: string] {
    ^awk '
    /type: PipeWire:Interface:Client\//{ skip=1 }
    skip { next }
    { print }
  ' $inp o> $out
}
strip_client ($env.TMPDIR | path join c.full) ($env.TMPDIR | path join expected)
strip_client ($env.TMPDIR | path join r.full) ($env.TMPDIR | path join actual)
compare "pw-cli info all (rich daemon, sans Client)"
