# `pw-cli ""` joins positionals into "" and parse() returns true
# (no-op success) on empty/whitespace input. Exits silently with code 0,
# unlike `pw-cli` (no args) which connects and starts the REPL.
"$REF"  "" </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" "" </dev/null > "$TMPDIR/actual"   2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-cli '' (empty command is silent no-op, not help dump)"
