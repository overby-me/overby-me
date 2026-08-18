#!/usr/bin/env nu

let app = $env.WORK | path join sum-app
let repo = $env.WORK | path join sum-repo
^$env.FLATPAK build-init $app org.test.Summary org.test.Sdk org.test.Platform
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
^$env.FLATPAK build-export $repo $app
try { ^$env.FLATPAK build-update-repo $repo }
# Check that repo directory still exists and has content
if ($repo | path type) != "dir" { exit 1 }
# Check that GVariant binary summary was created in the OSTree repo dir
let summary = $repo | path join repo summary
if ($summary | path type) != "file" {
  print "FAIL: repo/summary not created"
  exit 1
}
# Binary summary should not start with '#' (text format): GVariant starts with binary data
let first_byte = (^od -An -tx1 -N1 $summary | ^tr -d ' ' | str trim)
if $first_byte == "23" {
  print "FAIL: repo/summary appears to be text, not GVariant"
  exit 1
}

print "PASS: build-update-repo-creates-summary"
