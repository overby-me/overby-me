# pw-top -h (short help flag) — same output as --help.
"$REF" -h > "$TMPDIR/expected" 2>&1
"$RUST" -h > "$TMPDIR/actual" 2>&1
compare "pw-top/help-short"
