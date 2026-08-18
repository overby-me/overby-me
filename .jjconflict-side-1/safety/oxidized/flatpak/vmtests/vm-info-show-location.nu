#!/usr/bin/env nu

source ./libtest-nix.nu

setup_repo

let r = (do { ^$env.FLATPAK --user info --show-location org.test.Hello } | complete)
let output = ($r.stdout + $r.stderr) | str trim -r -c "\n"
print $"show-location output: ($output)"

if not ($output | str contains "org.test.Hello") {
    print "FAIL: expected 'org.test.Hello' in show-location output"
    print $"Got: ($output)"
    exit 1
}

if not (test-flag "-d" $output) {
    print "FAIL: show-location path does not exist as a directory"
    print $"Got: ($output)"
    exit 1
}

ok "show-location prints deploy path"
print "PASS: vm-info-show-location"
