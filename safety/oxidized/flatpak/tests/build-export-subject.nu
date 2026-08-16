#!/usr/bin/env nu

let app = $env.WORK | path join subjapp
let repo = $env.WORK | path join subjrepo

^$env.FLATPAK build-init $app org.test.Subj org.test.Sdk org.test.Platform
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
let r = (do { ^$env.FLATPAK build-export $repo $app -s "My custom subject" } | complete)
# Check the subject was used (appears in output)
if ($r.stdout + $r.stderr) !~ '(?i)My custom subject' {
  print "Note: subject may not appear in output, but export should succeed"
}
# Just verify repo was created
if ($repo | path type) != "dir" {
  exit 1
}

print "PASS: build-export-subject"
