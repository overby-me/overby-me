# Verify that `pw-cli info NonExistent` exits 0 (the error message goes
# to stderr but doesn't fail the command).
"$REF"  info NonExistent </dev/null > /dev/null 2>&1
ref_exit=$?
"$RUST" info NonExistent </dev/null > /dev/null 2>&1
rust_exit=$?
echo "exit=$ref_exit"  > "$TMPDIR/expected"
echo "exit=$rust_exit" > "$TMPDIR/actual"
compare "pw-cli info NonExistent (exit code parity)"
