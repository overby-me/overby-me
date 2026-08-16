source ../helpers.nu

# pw-dot with an empty JSON file: emits the bare digraph header/footer
# and the standard "set output file -" prefix when -o is `-`.
"[]\n" | save -f --raw ($env.TMPDIR | path join test.json)
^$env.REF -j ($env.TMPDIR | path join test.json) -o - o+e> ($env.TMPDIR | path join expected)
^$env.RUST -j ($env.TMPDIR | path join test.json) -o - o+e> ($env.TMPDIR | path join actual)
compare "pw-dot/empty-json"
