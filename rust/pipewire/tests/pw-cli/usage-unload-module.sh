# pw-cli unload-module with no args prints `Error: "unload-module <usage>"` to stderr.
"$REF"  unload-module </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" unload-module </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli unload-module (usage error)"
