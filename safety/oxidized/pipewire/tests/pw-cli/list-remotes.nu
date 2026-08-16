source ../helpers.nu

# `pw-cli list-remotes` (lr) prints the connected remotes. As with
# list-vars, the heap pointer varies, so we normalize.
^$env.REF list-remotes o+e> ($env.TMPDIR | path join c.full)
^$env.RUST list-remotes o+e> ($env.TMPDIR | path join r.full)

def normalize_ptrs [inp: string, out: string] {
    ^sed -E 's|@remote:0x[0-9a-fA-F]+|@remote:PTR|g' $inp o> $out
}
normalize_ptrs ($env.TMPDIR | path join c.full) ($env.TMPDIR | path join expected)
normalize_ptrs ($env.TMPDIR | path join r.full) ($env.TMPDIR | path join actual)
compare "pw-cli list-remotes (pointer-normalized)"
