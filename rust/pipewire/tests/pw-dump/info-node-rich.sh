# Rich daemon: a null-audio-sink Node lives at id 8. pw-dump <id> should
# emit the full registry entry for that one global, including the per-
# class Node info block (max-input-ports, n-input-ports, state, props,
# params).
"$REF"  8 </dev/null > "$TMPDIR/c.full"  2>"$TMPDIR/c.err"
"$RUST" 8 </dev/null > "$TMPDIR/r.full"  2>"$TMPDIR/r.err"

# The Node's clock.* props (clock.id / clock.start-quantum) and any
# spa-allocated object.serial fields are stable across runs in this
# clean test daemon, so no normalization needed beyond the standard
# nix-store path stripping done by `compare`.
cp "$TMPDIR/c.full" "$TMPDIR/expected"
cp "$TMPDIR/r.full" "$TMPDIR/actual"
compare "pw-dump 8 (Node info block: max-input-ports/n-input-ports/state/props/params, rich daemon)"
