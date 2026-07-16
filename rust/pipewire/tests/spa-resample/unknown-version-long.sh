# spa-resample doesn't declare --version (only --help/-h/-v[erbose]).
# Long-form `--version` falls through to the "unrecognized option" path.
"$REF"  --version > "$TMPDIR/expected" 2>&1 || true
"$RUST" --version > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --version (unrecognized: not declared in long_options)"
