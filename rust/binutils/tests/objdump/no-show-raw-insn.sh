# Test objdump -d --no-show-raw-insn — disassembly with the bytes column suppressed
$REF -d --no-show-raw-insn "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -d --no-show-raw-insn "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -d --no-show-raw-insn"
