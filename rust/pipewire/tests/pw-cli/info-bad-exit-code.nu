source ../helpers.nu

# Verify that `pw-cli info NonExistent` exits 0 (the error message goes
# to stderr but doesn't fail the command). In the nix sandbox the daemon
# isn't reachable so both binaries exit non-zero from the connect-fail
# path; `| complete` keeps the failure from aborting before we capture the code.
let ref_exit = (do { ^$env.REF info NonExistent } | complete).exit_code
let rust_exit = (do { ^$env.RUST info NonExistent } | complete).exit_code
$"exit=($ref_exit)\n" | save -f --raw ($env.TMPDIR | path join expected)
$"exit=($rust_exit)\n" | save -f --raw ($env.TMPDIR | path join actual)
compare "pw-cli info NonExistent (exit code parity)"
