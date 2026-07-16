# Filter by name: pw-metadata --list -n settings should print only
# the matching metadata. Both binaries identical.
"$REF"  --list -n settings </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" --list -n settings </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-metadata --list -n settings (rich daemon)"
