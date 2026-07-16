# pw-mididump on a non-existent file: same error message and exit code
# (255, from C's return -1) as upstream.
set +e
"$REF" /nonexistent.mid > "$TMPDIR/expected" 2>&1
ec_ref=$?
"$RUST" /nonexistent.mid > "$TMPDIR/actual" 2>&1
ec_rust=$?
set -e
if [ "$ec_ref" != "$ec_rust" ]; then
  echo "FAIL: exit codes differ (REF=$ec_ref RUST=$ec_rust)"
  exit 1
fi
compare "pw-mididump /nonexistent.mid"
