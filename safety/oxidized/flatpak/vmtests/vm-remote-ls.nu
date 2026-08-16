#!/usr/bin/env nu
source ./libtest-nix.nu

setup_http_repo

# Add the HTTP repo as a remote
^$env.FLATPAK --user remote-add --no-gpg-verify test-remote $env.REPO_URL

# remote-ls should list app refs by default
let r = (do { ^$env.FLATPAK --user remote-ls test-remote } | complete)
let output = $r.stdout + $r.stderr
print $"remote-ls output: ($output)"

if ($output | lines | any {|l| $l =~ 'org.test.Hello' }) {
  ok "remote-ls lists app ref"
} else {
  print "FAIL: remote-ls did not list org.test.Hello"
  print $"Got: ($output)"
  exit 1
}

# remote-ls -a should also list runtime refs
let r2 = (do { ^$env.FLATPAK --user remote-ls -a test-remote } | complete)
let output_all = $r2.stdout + $r2.stderr
print $"remote-ls -a output: ($output_all)"

if ($output_all | lines | any {|l| $l =~ 'org.test.Platform' }) {
  ok "remote-ls -a lists runtime ref"
} else {
  print "FAIL: remote-ls -a did not list org.test.Platform"
  print $"Got: ($output_all)"
  exit 1
}

cleanup_http
print "PASS: vm-remote-ls"
