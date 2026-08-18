#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo

# Setting --persist with path traversal must be rejected (CVE-2024-42472).
# The override is accepted (stored in overrides file), but at sandbox setup
# time the dangerous path is rejected with a warning and not bind-mounted.
^$env.FLATPAK --user override --persist=../../../etc org.test.Hello

let r = (do { run_sh org.test.Hello "true" } | complete)
let output = $r.stdout + $r.stderr

assert_match $output "ignoring dangerous --persist path|persist.*reject" "path traversal via --persist should be blocked"

ok "persist path traversal blocked"
print "PASS: vm-persist-path-traversal"
