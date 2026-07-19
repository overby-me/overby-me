source ../helpers.nu

let inp = ($env.TMPDIR | path join in.json)
r#'matrix = [ [ 1 2 3 ] [ 4 5 6 ] [ 7 8 9 ] ]
'# | save -f --raw $inp
^$env.REF $inp o+e> ($env.TMPDIR | path join expected)
^$env.RUST $inp o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/array-of-arrays"
