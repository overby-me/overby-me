source ../helpers.nu

# pw-link with -o pattern -i pattern: with non-matching patterns, both
# directions are filtered to empty.
^$env.REF -o nonexistent -i nonexistent o+e> ($env.TMPDIR | path join expected)
^$env.RUST -o nonexistent -i nonexistent o+e> ($env.TMPDIR | path join actual)
compare "pw-link -o nonexistent -i nonexistent"
