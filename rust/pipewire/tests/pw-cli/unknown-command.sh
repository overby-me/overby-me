# Unknown subcommand: print the C tool's exact `Error: "Command ..."`
# message to stderr and exit 1.
"$REF"  unknowncmd </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" unknowncmd </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli unknowncmd"
