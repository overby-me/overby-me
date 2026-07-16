# With --indent 0 the output collapses to a single line of JSON.
"$RUST" -i 0 > "$TMPDIR/dump.json" 2>"$TMPDIR/dump.err"
ec=$?
if [ $ec -ne 0 ]; then
  echo "FAIL: pw-dump exited $ec"
  cat "$TMPDIR/dump.err"
  exit 1
fi

# Should be exactly one line.
lines=$(wc -l < "$TMPDIR/dump.json")
if [ "$lines" -ne 1 ]; then
  echo "FAIL: expected 1 line, got $lines"
  head -3 "$TMPDIR/dump.json"
  exit 1
fi

assert_grep "Core type" '"PipeWire:Interface:Core"' "$TMPDIR/dump.json"
echo "PASS: pw-dump --indent 0 structural"
