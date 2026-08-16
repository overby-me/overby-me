#!/usr/bin/env nu

source ./libtest-nix.nu

# Test that config --unset removes a key

# Set a value first
^$env.FLATPAK --user config --set languages "en;de"
let r = (do { ^$env.FLATPAK --user config --get languages } | complete)
let output = ($r.stdout + $r.stderr) | str trim -r -c "\n"
assert_streq $output "en;de"
ok "config --set languages works"

# Unset the value
^$env.FLATPAK --user config --unset languages
ok "config --unset succeeded"

# Verify the key is gone (--get should fail)
let rc = (do { ^$env.FLATPAK --user config --get languages } | complete).exit_code
if $rc != 0 {
    ok "config --get fails after --unset"
} else {
    print "FAIL: config --get should fail after --unset"
    exit 1
}

# Unset of non-existent key should not fail
^$env.FLATPAK --user config --unset nonexistent-key
ok "config --unset of non-existent key does not fail"

print "PASS: vm-config-unset"
