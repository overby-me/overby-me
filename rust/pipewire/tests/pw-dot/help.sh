# pw-dot --help: byte-identical to upstream after store-path normalization.
"$REF" --help > "$TMPDIR/expected" 2>&1
"$RUST" --help > "$TMPDIR/actual" 2>&1
compare "pw-dot/help"
