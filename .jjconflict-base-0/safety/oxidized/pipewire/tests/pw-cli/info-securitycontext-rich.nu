source ../helpers.nu

let sc_id = (^$env.REF ls SecurityContext e> /dev/null | ^awk '/^\tid /{print $2; exit}' | ^tr -d ',' | str trim -r -c "\n")
if ($sc_id | is-empty) {
  print "FAIL: no SecurityContext global"
  exit 1
}
try { ^$env.REF info $sc_id o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST info $sc_id o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli info <SecurityContext> (rich daemon)"
