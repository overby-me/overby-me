source ../helpers.nu

# `pw-cli list-vars` (lv) prints the auto-added remote variable. The C
# tool's output includes a heap pointer which varies per run, so we
# normalize both outputs to a canonical sentinel before diffing.
^$env.REF list-vars o+e> ($env.TMPDIR | path join c.full)
^$env.RUST list-vars o+e> ($env.TMPDIR | path join r.full)

def normalize_ptrs [inp: string, out: string] {
    ^sed -E 's|@remote:0x[0-9a-fA-F]+|@remote:PTR|g' $inp o> $out
}
normalize_ptrs ($env.TMPDIR | path join c.full) ($env.TMPDIR | path join expected)
normalize_ptrs ($env.TMPDIR | path join r.full) ($env.TMPDIR | path join actual)
compare "pw-cli list-vars (pointer-normalized)"
