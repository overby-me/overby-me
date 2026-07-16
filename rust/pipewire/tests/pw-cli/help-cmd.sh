# `pw-cli help` (the interactive command, not the --help flag) prints just
# the "Available commands:" list — no leading option summary.
"$REF"  help </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" help </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-cli help (interactive command)"
