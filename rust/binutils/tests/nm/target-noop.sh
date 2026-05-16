# Test nm --target=NAME — should be silently accepted (no BFD targets)
$REF --target=elf64-x86-64 "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST --target=elf64-x86-64 "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm --target=elf64-x86-64"
