source ../helpers.nu

# Quoted strings with escape sequences must round-trip verbatim.
r#'{
  plain = "hello"
  esc = "line1\nline2"
  quote = "she said \"hi\""
  bslash = "C:\\path"
}
'# | save -f --raw ($env.TMPDIR | path join in.json)
^$env.REF ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join expected)
^$env.RUST ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/strings"
