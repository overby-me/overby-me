# Factory id 4 is the client-node factory (loaded by libpipewire-module-client-node).
"$REF"  info 4 </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" info 4 </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-cli info 4 (Factory: client-node)"
