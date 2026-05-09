# pw-cli create-device with no args prints `Error: "create-device <usage>"` to stderr.
"$REF"  create-device </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" create-device </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli create-device (usage error)"
