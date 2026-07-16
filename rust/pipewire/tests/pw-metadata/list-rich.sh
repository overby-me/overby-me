# Rich daemon: pw-metadata --list should find at least the "settings"
# metadata loaded by libpipewire-module-metadata.
"$REF"  --list </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" --list </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-metadata --list (rich daemon)"
