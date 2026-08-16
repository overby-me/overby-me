#!/usr/bin/env nu

let app = $env.WORK | path join exportapp
let repo = $env.WORK | path join repo

^$env.FLATPAK build-init $app org.test.Export org.test.Sdk org.test.Platform
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin myapp)
^chmod +x ($app | path join files bin myapp)
^$env.FLATPAK build-finish $app --command myapp
^$env.FLATPAK build-export $repo $app

if ($repo | path type) != "dir" {
  print "FAIL: repo directory was not created"
  exit 1
}

# Check that the repo has some content (ostree repo structure)
if (ls -a $repo | is-empty) {
  print "FAIL: repo directory is empty"
  exit 1
}

print "PASS: build-export-creates-repo"
