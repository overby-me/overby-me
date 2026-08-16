source ../helpers.nu

# `info all` walks every registry global. Apply the same Client-block strip
# as ls-all-normalized so the connecting client (which differs per binary)
# doesn't pollute the diff.
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
compare "pw-cli info all (sans Client)"
