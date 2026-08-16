#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

# Add a remote first
^$env.FLATPAK --user remote-add testremote https://example.com/repo

# Verify it was added
let r = (do { ^$env.FLATPAK --user remotes } | complete)
if not (($r.stdout + $r.stderr) =~ "testremote") {
  print "FAIL: testremote was not added"
  exit 1
}

# Delete the remote
^$env.FLATPAK --user remote-delete testremote

# Verify it was removed
let r = (do { ^$env.FLATPAK --user remotes } | complete)
if ($r.stdout + $r.stderr) =~ "testremote" {
  print "FAIL: testremote still present after remote-delete"
  exit 1
}

print "PASS: remote-delete works correctly"
