source ../helpers.nu

# Number formats: int, negative, float, scientific notation, dotted-quad
# (which is NOT a number and must be requoted as a string).
r#'{
  i = 42
  ineg = -7
  f = 1.5
  fexp = 1.5e10
  ip = 192.168.1.1
}
'# | save -f --raw ($env.TMPDIR | path join in.json)
^$env.REF ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join expected)
^$env.RUST ($env.TMPDIR | path join in.json) o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/numbers"
