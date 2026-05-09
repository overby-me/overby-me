# Rich daemon: the null-audio-sink Node has no output ports.
"$REF"  -o </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" -o </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-link -o (rich daemon, no output ports)"
