# Module id 3 is libpipewire-module-client-node in our test daemon.
"$REF"  info 3 </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" info 3 </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-cli info 3 (Module: client-node)"
