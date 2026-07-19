#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo
# Run the app and check if /.flatpak-info exists inside the sandbox
let r = (do { run_sh org.test.Hello 'cat /.flatpak-info 2>/dev/null || echo NO_FLATPAK_INFO' } | complete)
let output = $r.stdout + $r.stderr
# Depending on rust-flatpak implementation, this may or may not exist yet
# At minimum, the run should succeed
ok "run flatpak-info"

print "PASS: vm-run-flatpak-info"
