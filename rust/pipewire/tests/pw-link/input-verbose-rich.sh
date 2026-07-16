# Rich daemon: pw-link -i -v adds object.path and port.alias indented
# under each port name.
"$REF"  -i -v </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" -i -v </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-link -i -v (rich daemon)"
