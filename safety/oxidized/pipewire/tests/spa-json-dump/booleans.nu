source ../helpers.nu

let inp = ($env.TMPDIR | path join in.json)
r#'a = true
b = false
c = null
arr = [ true false null ]
'# | save -f --raw $inp
^$env.REF $inp o+e> ($env.TMPDIR | path join expected)
^$env.RUST $inp o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/booleans"
