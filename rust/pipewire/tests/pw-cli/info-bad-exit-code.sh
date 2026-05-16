# Verify that `pw-cli info NonExistent` exits 0 (the error message goes
# to stderr but doesn't fail the command). In the nix sandbox the daemon
# isn't reachable so both binaries exit non-zero from the connect-fail
# path; `|| true` keeps set -e from aborting before we capture $?.
"$REF"  info NonExistent </dev/null > /dev/null 2>&1 || true
ref_exit=$?
"$RUST" info NonExistent </dev/null > /dev/null 2>&1 || true
rust_exit=$?
echo "exit=$ref_exit"  > "$TMPDIR/expected"
echo "exit=$rust_exit" > "$TMPDIR/actual"
compare "pw-cli info NonExistent (exit code parity)"
