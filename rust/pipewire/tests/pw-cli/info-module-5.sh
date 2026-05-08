# Module id 5 is libpipewire-module-access in our test daemon.
"$REF"  info 5 </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" info 5 </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-cli info 5 (Module: access)"
