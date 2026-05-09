"$REF"  unload-module foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" unload-module foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli unload-module foo (unknown module)"
