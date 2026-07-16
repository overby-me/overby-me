# Module id 1 is libpipewire-module-protocol-native — always present.
"$REF"  info 1 </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" info 1 </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-cli info 1 (Module: protocol-native)"
