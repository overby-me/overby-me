# Rich daemon: a null-audio-sink Node with 2 input ports (FL, FR).
# pw-link -i should list them in registry order; both binaries identical.
"$REF"  -i </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" -i </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-link -i (rich daemon, 2 input ports)"
