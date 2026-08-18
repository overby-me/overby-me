source ../helpers.nu

# pw-cli ls with no filter on rich daemon. Strip the connecting Client.
^$env.REF ls o+e> ($env.TMPDIR | path join c.full)
^$env.RUST ls o+e> ($env.TMPDIR | path join r.full)
def strip_client [src: string, dst: string] {
    ^awk '/type PipeWire:Interface:Client\//{exit} {print}' $src o> $dst
}
strip_client ($env.TMPDIR | path join c.full) ($env.TMPDIR | path join expected)
strip_client ($env.TMPDIR | path join r.full) ($env.TMPDIR | path join actual)
compare "pw-cli ls (rich daemon, sans Client)"
