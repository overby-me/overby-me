source ../helpers.nu

# Plain JSON object: exercise the round-trip through the parser/printer.
let inp = ($env.TMPDIR | path join in.json)
r#'{ "a": 1, "b": "hello", "c": true, "d": null }
'# | save -f --raw $inp
^$env.REF $inp o+e> ($env.TMPDIR | path join expected)
^$env.RUST $inp o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/basic"
