# Same as disconnect but via switch-remote. Both share find_var
# lookup semantics.
"$REF"  "switch-remote 99" </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" "switch-remote 99" </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli switch-remote 99 (Error: 'Remote 99 does not exist')"
