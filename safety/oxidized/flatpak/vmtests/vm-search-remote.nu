#!/usr/bin/env nu
source ./libtest-nix.nu

setup_http_repo

# Add the HTTP repo as a remote
^$env.FLATPAK --user remote-add --no-gpg-verify test-remote $env.REPO_URL

# Search for the test app by name
let r = (do { ^$env.FLATPAK --user search Hello } | complete)
let output = $r.stdout + $r.stderr
print $"search output: ($output)"

if ($output | lines | any {|l| $l =~ 'org.test.Hello' }) {
  ok "search finds app by name"
} else {
  print "FAIL: search did not find org.test.Hello"
  print $"Got: ($output)"
  exit 1
}

# Search for something that doesn't exist
let r2 = (do { ^$env.FLATPAK --user search NonExistentApp12345 } | complete)
let rc = $r2.exit_code
let output2 = $r2.stdout + $r2.stderr
print $"search non-existent output \(rc=($rc)): ($output2)"

# Should either return non-zero or produce no matching output
if ($rc != 0) or (not ($output2 | lines | any {|l| $l =~ 'NonExistentApp12345' })) {
  ok "search for non-existent app returns no match"
} else {
  print "FAIL: search unexpectedly found NonExistentApp12345"
  exit 1
}

cleanup_http
print "PASS: vm-search-remote"
