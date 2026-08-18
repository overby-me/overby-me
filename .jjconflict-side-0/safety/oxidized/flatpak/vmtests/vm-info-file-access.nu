#!/usr/bin/env nu

source ./libtest-nix.nu

# Test that info --file-access reports correct access levels based on app metadata.
# The test app has filesystems=home; in its [Context], so home should be read-write
# and paths not granted (like /usr) should be hidden.

setup_repo

# Check that home access is reported as read-write
let r = (do { ^$env.FLATPAK --user info --file-access=home org.test.Hello } | complete)
let output = $r.stdout + $r.stderr
print $"file-access=home output: ($output)"
if ($output | str contains --ignore-case "read-write") {
    ok "home reported as read-write"
} else {
    print $"FAIL: expected 'read-write' for home access, got: ($output)"
    exit 1
}

# Check that a path not granted is reported as hidden
let r2 = (do { ^$env.FLATPAK --user info --file-access=/usr org.test.Hello } | complete)
let output2 = $r2.stdout + $r2.stderr
print $"file-access=/usr output: ($output2)"
if ($output2 | str contains --ignore-case "hidden") {
    ok "/usr reported as hidden"
} else {
    print $"FAIL: expected 'hidden' for /usr access, got: ($output2)"
    exit 1
}

print "PASS: vm-info-file-access"
