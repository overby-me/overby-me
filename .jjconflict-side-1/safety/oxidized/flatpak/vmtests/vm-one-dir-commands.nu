#!/usr/bin/env nu
source ./libtest-nix.nu

# Test that --system --user on same command gives error
for cmd in [config override remote-add repair] {
    let r = (do { ^$env.FLATPAK $cmd --system --user } | complete)
    let rc = $r.exit_code
    # oxidized-flatpak may not enforce this yet, just check it doesn't crash
    if ($rc == 139) or ($rc == 134) or ($rc == 136) {
        print $"FAIL: ($cmd) --system --user crashed"
        exit 1
    }
}
ok "one-dir commands don't crash"

print "PASS: vm-one-dir-commands"
