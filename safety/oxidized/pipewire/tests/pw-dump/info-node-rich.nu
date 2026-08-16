source ../helpers.nu

# Rich daemon: a null-audio-sink Node lives at id 8. pw-dump <id> should
# emit the full registry entry for that one global, including the per-
# class Node info block (max-input-ports, n-input-ports, state, props,
# params).
let cfull = $env.TMPDIR | path join c.full
let rfull = $env.TMPDIR | path join r.full
^$env.REF 8 o> $cfull e> ($env.TMPDIR | path join c.err)
^$env.RUST 8 o> $rfull e> ($env.TMPDIR | path join r.err)

# The Node's clock.* props (clock.id / clock.start-quantum) and any
# spa-allocated object.serial fields are stable across runs in this
# clean test daemon, so no normalization needed beyond the standard
# nix-store path stripping done by `compare`.
cp $cfull ($env.TMPDIR | path join expected)
cp $rfull ($env.TMPDIR | path join actual)
compare "pw-dump 8 (Node info block: max-input-ports/n-input-ports/state/props/params, rich daemon)"
