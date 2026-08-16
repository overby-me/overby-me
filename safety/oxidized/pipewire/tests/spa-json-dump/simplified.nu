source ../helpers.nu

# -s: output simplified SPA JSON (no commas, '=' separator, unquoted barewords).
r#'{ "first": "value", "second": 42, "list": [ "a", "b", "c" ] }
'# | save -f --raw ($env.TMPDIR | path join in.json)
^$env.REF -s ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join expected)
^$env.RUST -s ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/simplified"
