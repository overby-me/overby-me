source ../helpers.nu

# Keys with dots, dashes, slashes, and quoted special chars.
r#'"node.name" = "alice"
"node-with-dash" = 1
"foo/bar" = "slash"
"key with spaces" = ok
'# | save -f --raw ($env.TMPDIR | path join in.json)
^$env.REF ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join expected)
^$env.RUST ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/special-keys"
