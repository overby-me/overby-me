source ../helpers.nu

# Deeply nested structure: verifies indent stacking.
r#'outer = {
  middle = {
    inner = {
      list = [ { x = 1 } { x = 2 } ]
    }
  }
}
'# | save -f --raw ($env.TMPDIR | path join in.json)
^$env.REF ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join expected)
^$env.RUST ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/nested"
