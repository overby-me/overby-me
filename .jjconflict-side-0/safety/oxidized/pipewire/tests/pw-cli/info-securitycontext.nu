source ../helpers.nu

# SecurityContext has no class registered in pw-cli, so single-id info
# prints `Error: "unsupported type ..."` to stderr.
let sc_id = (^$env.REF ls SecurityContext e> /dev/null | ^awk '/^\tid /{print $2; exit}' | ^tr -d ',' | str trim -r -c "\n")
if ($sc_id | is-empty) {
  print "FAIL: no SecurityContext global to test against"
  exit 1
}

try { ^$env.REF info $sc_id o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST info $sc_id o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli info <SecurityContext> (unsupported type)"
