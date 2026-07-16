# Rich daemon: a null-audio-sink Node has 2 input ports (FL, FR). ls Port
# should list them; both binaries should produce identical output.
"$REF"  ls Port </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" ls Port </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-cli ls Port (rich daemon)"
