# pw-cli load-module with no args prints `Error: "load-module <usage>"` to stderr.
"$REF"  load-module </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" load-module </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli load-module (usage error)"
