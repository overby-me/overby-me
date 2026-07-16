# Looking up an id that doesn't exist should print
# `Error: "info: unknown global 'X'"` to stderr (matching the C tool).
# Both binaries exit non-zero; we don't care about exit code, just output.
"$REF"  info 9999 </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" info 9999 </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli info 9999 (unknown global)"
