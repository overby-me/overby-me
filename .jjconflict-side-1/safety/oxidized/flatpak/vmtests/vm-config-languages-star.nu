#!/usr/bin/env nu

source ./libtest-nix.nu

# Test that setting languages to "*" works correctly

^$env.FLATPAK --user config --set languages "*"
ok "config --set languages '*' succeeded"

let r = (do { ^$env.FLATPAK --user config --get languages } | complete)
let output = ($r.stdout + $r.stderr) | str trim -r -c "\n"
assert_streq $output "*"
ok "config --get languages returns '*'"

print "PASS: vm-config-languages-star"
