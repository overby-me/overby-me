# `pw-cli list-vars` (lv) prints the auto-added remote variable. The C
# tool's output includes a heap pointer which varies per run, so we
# normalize both outputs to a canonical sentinel before diffing.
"$REF"  list-vars </dev/null > "$TMPDIR/c.full" 2>&1
"$RUST" list-vars </dev/null > "$TMPDIR/r.full" 2>&1

normalize_ptrs() {
  sed -E 's|@remote:0x[0-9a-fA-F]+|@remote:PTR|g' "$1" > "$2"
}
normalize_ptrs "$TMPDIR/c.full" "$TMPDIR/expected"
normalize_ptrs "$TMPDIR/r.full" "$TMPDIR/actual"
compare "pw-cli list-vars (pointer-normalized)"
