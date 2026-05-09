# Look up first global of type `Profiler` by substring match.
"$REF"  info Profiler </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" info Profiler </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli info Profiler (by-name)"
