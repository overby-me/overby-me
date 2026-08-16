#!/usr/bin/env nu
source ./libtest-nix.nu

setup_http_repo

# Add the HTTP repo as a remote
^$env.FLATPAK --user remote-add --no-gpg-verify test-remote $env.REPO_URL

# Remove locally installed app (setup_http_repo only installs runtime locally)
try { ^$env.FLATPAK --user uninstall org.test.Hello }

# Verify app is not installed
let l0 = (do { ^$env.FLATPAK --user list } | complete)
if (($l0.stdout + $l0.stderr) | lines | any {|l| $l =~ 'org.test.Hello' }) {
    print "FAIL: org.test.Hello should not be installed yet"
    exit 1
}

# Install app from the HTTP remote
let ri = (do { ^$env.FLATPAK --user install test-remote org.test.Hello } | complete)
let output = $ri.stdout + $ri.stderr
print $"install output: ($output)"

# Verify app is now installed
let rl = (do { ^$env.FLATPAK --user list } | complete)
let list_output = $rl.stdout + $rl.stderr
print $"list output: ($list_output)"

if ($list_output | lines | any {|l| $l =~ 'org.test.Hello' }) {
    ok "app installed from remote"
} else {
    print "FAIL: org.test.Hello not found after install"
    print $"Got: ($list_output)"
    exit 1
}

# Verify the installed app can actually run
let rr = (do { run org.test.Hello } | complete)
let run_output = $rr.stdout + $rr.stderr
print $"run output: ($run_output)"

if ($run_output | lines | any {|l| $l =~ 'Hello world, from a sandbox' }) {
    ok "app installed from remote runs correctly"
} else {
    print "FAIL: expected 'Hello world, from a sandbox' in output"
    print $"Got: ($run_output)"
    exit 1
}

cleanup_http
print "PASS: vm-install-from-remote"
