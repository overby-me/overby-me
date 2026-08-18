source ../helpers.nu

r#'quote = "with \"quote\""
backslash = "a\\b"
newline = "line1\nline2"
tab = "a\tb"
'# | save -f --raw ($env.TMPDIR | path join in.json)
^$env.REF ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join expected)
^$env.RUST ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/escaped-strings"
