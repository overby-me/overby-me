source ../helpers.nu

# -i 4: change indent width.
r#'{ a = 1, b = [ 2 3 ] }
'# | save -f --raw ($env.TMPDIR | path join in.json)
^$env.REF -i 4 ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join expected)
^$env.RUST -i 4 ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/indent-4"
