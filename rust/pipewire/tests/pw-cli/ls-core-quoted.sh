# `pw-cli "ls Core"` (single quoted argument) parses the same as
# `pw-cli ls Core` because the C tool joins all positional args with
# spaces, then splits on whitespace.
"$REF"  "ls Core" </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" "ls Core" </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-cli 'ls Core' (joined input)"
