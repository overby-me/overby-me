#!/usr/bin/env nu

let app = $env.WORK | path join redir-app
let repo = $env.WORK | path join redir-repo
^$env.FLATPAK build-init $app org.test.Redir org.test.Sdk org.test.Platform
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
^$env.FLATPAK build-export $repo $app
^$env.FLATPAK build-update-repo --redirect-url=http://example.com/redir $repo

if (do { ^grep -q "redirect-url=http://example.com/redir" ($repo | path join config) } | complete).exit_code != 0 {
  print "FAIL: redirect-url not found in repo config"
  exit 1
}

print "PASS: build-update-repo-redirect"
