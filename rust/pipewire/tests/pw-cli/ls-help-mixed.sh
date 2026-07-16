# `pw-cli ls -h` — the C tool joins all args and parses; -h becomes
# part of the filter string.
"$REF"  ls -h </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" ls -h </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-cli ls -h (mixed args)"
