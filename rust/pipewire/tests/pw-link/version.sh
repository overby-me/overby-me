# pw-link --version: byte-identical to upstream after store-path normalization.
"$REF" --version > "$TMPDIR/expected" 2>&1
"$RUST" --version > "$TMPDIR/actual" 2>&1
compare "pw-link/version"
