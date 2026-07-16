# pw-dump -V (short version flag) — same output as --version.
"$REF" -V > "$TMPDIR/expected" 2>&1
"$RUST" -V > "$TMPDIR/actual" 2>&1
compare "pw-dump/version-short"
