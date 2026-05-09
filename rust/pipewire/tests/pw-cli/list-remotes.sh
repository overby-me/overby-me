# `pw-cli list-remotes` (lr) prints the connected remotes. As with
# list-vars, the heap pointer varies, so we normalize.
"$REF"  list-remotes </dev/null > "$TMPDIR/c.full" 2>&1
"$RUST" list-remotes </dev/null > "$TMPDIR/r.full" 2>&1

normalize_ptrs() {
  sed -E 's|@remote:0x[0-9a-fA-F]+|@remote:PTR|g' "$1" > "$2"
}
normalize_ptrs "$TMPDIR/c.full" "$TMPDIR/expected"
normalize_ptrs "$TMPDIR/r.full" "$TMPDIR/actual"
compare "pw-cli list-remotes (pointer-normalized)"
