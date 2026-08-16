#!/usr/bin/env nu

source ./libtest-nix.nu

# Test config --set / --get / --unset

# Set a config value
^$env.FLATPAK --user config --set languages "en;de"
ok "config --set succeeded"

# Get the config value back
let r = (do { ^$env.FLATPAK --user config --get languages } | complete)
let output = ($r.stdout + $r.stderr) | str trim -r -c "\n"
assert_streq $output "en;de"
ok "config --get returns correct value"

# Overwrite the value
^$env.FLATPAK --user config --set languages "en;fr"
let r2 = (do { ^$env.FLATPAK --user config --get languages } | complete)
let output2 = ($r2.stdout + $r2.stderr) | str trim -r -c "\n"
assert_streq $output2 "en;fr"
ok "config --set overwrites existing value"

# Set a second key
^$env.FLATPAK --user config --set extra-languages "de"
let r3 = (do { ^$env.FLATPAK --user config --get extra-languages } | complete)
let output3 = ($r3.stdout + $r3.stderr) | str trim -r -c "\n"
assert_streq $output3 "de"
ok "config --set second key works"

# First key still intact
let r4 = (do { ^$env.FLATPAK --user config --get languages } | complete)
let output4 = ($r4.stdout + $r4.stderr) | str trim -r -c "\n"
assert_streq $output4 "en;fr"
ok "first key still readable after setting second"

# Unset the first key
^$env.FLATPAK --user config --unset languages
let rc = (do { ^$env.FLATPAK --user config --get languages } | complete).exit_code
if $rc != 0 {
    ok "config --unset removes key (--get fails)"
} else {
    print "FAIL: config --get should fail after --unset"
    exit 1
}

# Second key still intact after unset of first
let r5 = (do { ^$env.FLATPAK --user config --get extra-languages } | complete)
let output5 = ($r5.stdout + $r5.stderr) | str trim -r -c "\n"
assert_streq $output5 "de"
ok "other keys survive --unset"

print "PASS: vm-config-set-get"
