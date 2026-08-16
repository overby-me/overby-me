#!/usr/bin/env nu

source ./libtest-nix.nu

# Test that info --show-extensions reports no extensions for an app without extension points.

setup_repo

let r = (do { ^$env.FLATPAK --user info --show-extensions org.test.Hello } | complete)
let output = $r.stdout + $r.stderr
print $"show-extensions output: ($output)"

if ($output | str contains --ignore-case "No extensions") {
    ok "no extensions reported for app without extension points"
} else {
    print "FAIL: expected 'No extensions' in output"
    print $"Got: ($output)"
    exit 1
}

print "PASS: vm-info-show-extensions"
