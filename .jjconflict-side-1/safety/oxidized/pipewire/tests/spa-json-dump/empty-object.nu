source ../helpers.nu

# Empty object and empty array.
r#'empty_obj = {}
empty_arr = []
'# | save -f --raw ($env.TMPDIR | path join in.json)
^$env.REF ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join expected)
^$env.RUST ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/empty-object"
