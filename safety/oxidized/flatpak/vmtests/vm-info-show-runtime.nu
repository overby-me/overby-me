#!/usr/bin/env nu

source ./libtest-nix.nu

setup_repo

let r = (do { ^$env.FLATPAK --user info --show-runtime org.test.Hello } | complete)
let output = $r.stdout + $r.stderr
print $"show-runtime output: ($output)"

if not ($output | str contains "org.test.Platform") {
    print "FAIL: expected 'org.test.Platform' in show-runtime output"
    print $"Got: ($output)"
    exit 1
}

ok "show-runtime prints runtime ref"
print "PASS: vm-info-show-runtime"
