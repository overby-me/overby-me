#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo

let r = (do { ^$env.FLATPAK --user info --show-sdk org.test.Hello } | complete)
let output = $r.stdout + $r.stderr
print $"info --show-sdk output: ($output)"

if ($output | lines | any {|l| $l =~ 'org.test.Sdk' }) {
    ok "show-sdk contains org.test.Sdk"
} else {
    print "FAIL: expected 'org.test.Sdk' in output (derived from Platform->Sdk)"
    print $"Got: ($output)"
    exit 1
}

print "PASS: vm-info-show-sdk"
