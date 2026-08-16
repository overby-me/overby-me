source ../helpers.nu

# Input with #-comments and inline #-comments after values.
let inp = ($env.TMPDIR | path join in.json)
r##'# leading comment
key1 = 1   # trailing comment
key2 = 2
# blank-line comment
key3 = 3
'## | save -f --raw $inp
^$env.REF $inp o+e> ($env.TMPDIR | path join expected)
^$env.RUST $inp o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/comments"
