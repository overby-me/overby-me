#!/usr/bin/env nu
source ./libtest-nix.nu

setup_http_repo

# Add the HTTP repo as a remote
^$env.FLATPAK --user remote-add --no-gpg-verify test-remote $env.REPO_URL

# remote-info should show details for a specific ref
let r = (do { ^$env.FLATPAK --user remote-info test-remote org.test.Hello } | complete)
let output = $r.stdout + $r.stderr
print $"remote-info output: ($output)"

if ($output | lines | any {|l| $l =~ 'org.test.Hello' }) {
  ok "remote-info shows app id"
} else {
  print "FAIL: remote-info did not show org.test.Hello"
  print $"Got: ($output)"
  exit 1
}

# Also test with runtime ref
let r2 = (do { ^$env.FLATPAK --user remote-info test-remote org.test.Platform } | complete)
let output2 = $r2.stdout + $r2.stderr
print $"remote-info runtime output: ($output2)"

if ($output2 | lines | any {|l| $l =~ 'org.test.Platform' }) {
  ok "remote-info shows runtime id"
} else {
  print "FAIL: remote-info did not show org.test.Platform"
  print $"Got: ($output2)"
  exit 1
}

cleanup_http
print "PASS: vm-remote-info"
