source ../helpers.nu

# pw-cli's `do_list_objects` joins all positional args with spaces into
# a single filter string (`Module Foo`), then strstr's against types.
# Since no type contains "Module Foo", output is empty.
^$env.REF ls Module Foo o+e> ($env.TMPDIR | path join expected)
^$env.RUST ls Module Foo o+e> ($env.TMPDIR | path join actual)
compare "pw-cli ls Module Foo (joined-arg filter)"
