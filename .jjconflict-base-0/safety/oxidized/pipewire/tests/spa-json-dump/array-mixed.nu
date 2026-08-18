source ../helpers.nu

# Mixed-type array: numbers, strings, nested object, nested array.
let inp = ($env.TMPDIR | path join in.json)
r#'arr = [ 1 "two" 3.14 { k = v } [ "nested" ] true false null ]
'# | save -f --raw $inp
^$env.REF $inp o+e> ($env.TMPDIR | path join expected)
^$env.RUST $inp o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/array-mixed"
