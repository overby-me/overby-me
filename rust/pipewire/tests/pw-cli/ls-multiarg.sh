# pw-cli's `do_list_objects` joins all positional args with spaces into
# a single filter string (`Module Foo`), then strstr's against types.
# Since no type contains "Module Foo", output is empty.
"$REF"  ls Module Foo </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" ls Module Foo </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-cli ls Module Foo (joined-arg filter)"
