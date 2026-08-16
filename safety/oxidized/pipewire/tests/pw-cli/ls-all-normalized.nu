source ../helpers.nu

# Compare full `pw-cli ls` (no filter) with normalization.
#
# The Client global at the tail of the registry is whichever client is
# currently connected: its id, application.name, pipewire.sec.pid all
# differ between the two binaries. We strip the entire Client block from
# both outputs before comparing.

^$env.REF ls o> ($env.TMPDIR | path join c.full) e> ($env.TMPDIR | path join c.err)
^$env.RUST ls o> ($env.TMPDIR | path join r.full) e> ($env.TMPDIR | path join r.err)

# Drop everything from the first `id N, type PipeWire:Interface:Client/` line
# to end of file. Both outputs are sorted by id, so the connecting client
# is always the last block.
def strip_client [src: string, dst: string] {
    ^awk '/type PipeWire:Interface:Client\//{exit} {print}' $src o> $dst
}
strip_client ($env.TMPDIR | path join c.full) ($env.TMPDIR | path join expected)
strip_client ($env.TMPDIR | path join r.full) ($env.TMPDIR | path join actual)
compare "pw-cli ls (sans Client)"
