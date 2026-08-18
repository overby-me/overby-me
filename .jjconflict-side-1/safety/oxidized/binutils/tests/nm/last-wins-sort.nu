source ../helpers.nu

# Test nm bundled sort flags: last-wins between -n (numeric) and -p (no-sort)
try { ^$env.REF -np $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -np $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm bundled flags (last-wins direction 1)"

let args = "-p" + "n"
try { ^$env.REF $args $env.TESTOBJ o+e>> ($env.TMPDIR | path join expected) }
try { ^$env.RUST $args $env.TESTOBJ o+e>> ($env.TMPDIR | path join actual) }
compare "nm bundled flags (last-wins both)"
