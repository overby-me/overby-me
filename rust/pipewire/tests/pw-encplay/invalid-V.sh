# pw-encplay -V is an unrecognized short option (the long --version has no
# short alias). C tool prints `invalid option -- 'V'` then the help.
# Both exit non-zero, so we need || true.
"$REF" -V > "$TMPDIR/expected" 2>&1 || true
"$RUST" -V > "$TMPDIR/actual" 2>&1 || true
compare "pw-encplay/invalid-V"
