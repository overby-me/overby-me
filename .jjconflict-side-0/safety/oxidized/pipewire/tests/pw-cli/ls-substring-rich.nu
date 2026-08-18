source ../helpers.nu

# Rich daemon: `ls Pi` matches every global. Tests substring filter
# behavior with the larger registry that includes a Node.
^$env.REF ls Pi o> ($env.TMPDIR | path join c.full) e> ($env.TMPDIR | path join c.err)
^$env.RUST ls Pi o> ($env.TMPDIR | path join r.full) e> ($env.TMPDIR | path join r.err)

def strip_client [src: string, dst: string] {
    ^awk '/type PipeWire:Interface:Client\//{exit} {print}' $src o> $dst
}
strip_client ($env.TMPDIR | path join c.full) ($env.TMPDIR | path join expected)
strip_client ($env.TMPDIR | path join r.full) ($env.TMPDIR | path join actual)
compare "pw-cli ls Pi (rich daemon)"
