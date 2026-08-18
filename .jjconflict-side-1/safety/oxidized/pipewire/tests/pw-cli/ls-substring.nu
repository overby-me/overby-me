source ../helpers.nu

# `pw-cli ls Pi` matches every global because the substring "Pi" appears
# in every "PipeWire:Interface:..." type. This exercises the C tool's
# `strstr(g->type, pattern)` substring filter, which we mirror.
^$env.REF ls Pi o> ($env.TMPDIR | path join c.full) e> ($env.TMPDIR | path join c.err)
^$env.RUST ls Pi o> ($env.TMPDIR | path join r.full) e> ($env.TMPDIR | path join r.err)

# Strip the connecting client (varies per binary).
def strip_client [src: string, dst: string] {
    ^awk '/type PipeWire:Interface:Client\//{exit} {print}' $src o> $dst
}
strip_client ($env.TMPDIR | path join c.full) ($env.TMPDIR | path join expected)
strip_client ($env.TMPDIR | path join r.full) ($env.TMPDIR | path join actual)
compare "pw-cli ls Pi (substring matches everything)"
