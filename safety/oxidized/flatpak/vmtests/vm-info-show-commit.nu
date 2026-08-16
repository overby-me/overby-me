#!/usr/bin/env nu

source ./libtest-nix.nu

# Test that info --show-commit prints a commit hash for an app installed from remote.
# Remote installs store the commit checksum in deploy_path/commit.

setup_http_repo

# Add the HTTP repo as a remote
^$env.FLATPAK --user remote-add --no-gpg-verify test-remote $env.REPO_URL

# Install app from remote (this stores the commit checksum)
^$env.FLATPAK --user install test-remote org.test.Hello
ok "app installed from remote"

# Get the commit via info --show-commit
let r = (do { ^$env.FLATPAK --user info --show-commit org.test.Hello } | complete)
let output = ($r.stdout + $r.stderr) | str trim -r -c "\n"
print $"show-commit output: ($output)"

# The commit should be a hex string (at least 32 chars of hex)
if ($output | lines | any {|l| $l =~ '^[0-9a-f]{32,}$' }) {
    ok "info --show-commit prints valid commit hash"
} else {
    print $"FAIL: expected hex commit hash, got: ($output)"
    exit 1
}

# Verify it's non-empty and not just a path basename fallback
let olen = ($output | str length)
if $olen >= 32 {
    ok $"commit hash has reasonable length \(($olen) chars\)"
} else {
    print $"FAIL: commit hash too short: ($output)"
    exit 1
}

cleanup_http
print "PASS: vm-info-show-commit"
