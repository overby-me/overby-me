"$REF"  --dsd </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --dsd </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat --dsd alone (sub-mode requires --playback or --record)"
