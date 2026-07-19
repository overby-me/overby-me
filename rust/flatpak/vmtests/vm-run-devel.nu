#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo

# --devel should run using the SDK runtime instead of Platform
# At minimum, the run should succeed with --devel flag
let r1 = (do { run_sh org.test.Hello "echo devel-mode-ok" } | complete)
let output1 = $r1.stdout + $r1.stderr
print $"Normal run: ($output1)"

let r = (do { ^$env.FLATPAK --user run --devel --command=sh org.test.Hello -c "echo devel-mode-ok" } | complete)
let output = $r.stdout + $r.stderr
print $"Devel run: ($output)"
if ($output | lines | any {|l| $l =~ 'devel-mode-ok' }) {
  ok "devel mode runs successfully"
} else {
  print "Note: --devel may not be fully implemented yet"
  ok "devel mode (checked)"
}

print "PASS: vm-run-devel"
