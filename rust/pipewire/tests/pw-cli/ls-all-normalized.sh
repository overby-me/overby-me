# Compare full `pw-cli ls` (no filter) with normalization.
#
# The Client global at the tail of the registry is whichever client is
# currently connected — its id, application.name, pipewire.sec.pid all
# differ between the two binaries. We strip the entire Client block from
# both outputs before comparing.

"$REF"  ls </dev/null > "$TMPDIR/c.full"  2>"$TMPDIR/c.err"
"$RUST" ls </dev/null > "$TMPDIR/r.full"  2>"$TMPDIR/r.err"

# Drop everything from the first `id N, type PipeWire:Interface:Client/` line
# to end of file. Both outputs are sorted by id, so the connecting client
# is always the last block.
strip_client() {
  awk '/type PipeWire:Interface:Client\//{exit} {print}' "$1" > "$2"
}
strip_client "$TMPDIR/c.full"  "$TMPDIR/expected"
strip_client "$TMPDIR/r.full"  "$TMPDIR/actual"
compare "pw-cli ls (sans Client)"
