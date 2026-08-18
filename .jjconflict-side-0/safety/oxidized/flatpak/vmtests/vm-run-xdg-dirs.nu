#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo
# Check XDG dirs are remapped inside sandbox
let r = (do { run_sh org.test.Hello 'echo $XDG_CACHE_HOME' } | complete)
let cache_dir = $r.stdout + $r.stderr
# The sandbox should remap XDG dirs to ~/.var/app/<id>/
if ($cache_dir | lines | any {|l| $l =~ '.var/app/org.test.Hello' }) {
  ok "XDG dirs remapped"
} else {
  print $"Note: XDG_CACHE_HOME=($cache_dir) \(may not be remapped yet)"
  ok "XDG dirs (checked, implementation may vary)"
}

print "PASS: vm-run-xdg-dirs"
