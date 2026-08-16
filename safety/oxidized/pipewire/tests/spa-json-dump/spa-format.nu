source ../helpers.nu

# SPA-relaxed JSON: unquoted keys, '=' instead of ':', whitespace separators.
r#'key1 = value1
key2 = "quoted value"
list = [ 1 2 3 ]
nested = { a = 1 b = 2 }
'# | save -f --raw ($env.TMPDIR | path join in.json)
^$env.REF ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join expected)
^$env.RUST ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/spa-format"
