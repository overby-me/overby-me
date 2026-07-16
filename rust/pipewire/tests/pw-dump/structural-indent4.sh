# Same structural test as `structural`, but with `-i 4` indentation.
"$RUST" -i 4 > "$TMPDIR/dump.json" 2>"$TMPDIR/dump.err"
exit_code=$?
if [ $exit_code -ne 0 ]; then
  echo "FAIL: pw-dump exited with $exit_code"
  cat "$TMPDIR/dump.err"
  exit 1
fi

assert_grep "Core type"    '"PipeWire:Interface:Core"'    "$TMPDIR/dump.json"
# At indent=4, each nesting level is 4 spaces. Array > object > property
# = 8 spaces before `"id":`.
assert_grep "8-space id key" '^        "id":' "$TMPDIR/dump.json"

first_byte=$(head -c1 "$TMPDIR/dump.json")
if [ "$first_byte" != "[" ]; then
  echo "FAIL: pw-dump output does not start with '['"
  exit 1
fi
echo "PASS: pw-dump --indent 4 structural"
