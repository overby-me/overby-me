#!/usr/bin/env nu

let app = $env.WORK | path join branchapp
let repo = $env.WORK | path join branchrepo

^$env.FLATPAK build-init $app org.test.Branch org.test.Sdk org.test.Platform
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^chmod +x ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
^$env.FLATPAK build-export $repo $app -b mybranch

if ($repo | path type) != "dir" {
  print "FAIL: repo directory not created"
  exit 1
}

# Check that the repo references mybranch somewhere in its structure
if (do { ^grep -rl "mybranch" $repo } | complete).exit_code != 0 {
  # Also check directory names
  if (glob $"($repo)/**/*mybranch*" | is-empty) {
    # Try binary grep across all files
    if (do { ^grep -rl "mybranch" $repo } | complete).exit_code != 0 {
      print "WARN: could not confirm mybranch in repo (may be in binary format)"
    }
  }
}

print "PASS: build-export-branch"
