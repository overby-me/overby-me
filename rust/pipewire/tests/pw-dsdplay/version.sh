# pw-dsdplay --version: byte-identical.
"$REF" --version > "$TMPDIR/expected" 2>&1
"$RUST" --version > "$TMPDIR/actual" 2>&1
compare "pw-dsdplay/version"
