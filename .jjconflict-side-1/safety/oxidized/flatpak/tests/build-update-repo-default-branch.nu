#!/usr/bin/env nu

let app = $env.WORK | path join defbranch-app
let repo = $env.WORK | path join defbranch-repo
^$env.FLATPAK build-init $app org.test.DefBranch org.test.Sdk org.test.Platform
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
^$env.FLATPAK build-export $repo $app
^$env.FLATPAK build-update-repo --default-branch=beta $repo

if (do { ^grep -q "default-branch=beta" ($repo | path join config) } | complete).exit_code != 0 {
  print "FAIL: default-branch not set in repo config"
  exit 1
}

print "PASS: build-update-repo-default-branch"
