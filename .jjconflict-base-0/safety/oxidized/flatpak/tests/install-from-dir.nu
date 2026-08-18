#!/usr/bin/env nu

let app = $env.WORK | path join installapp
^$env.FLATPAK build-init $app org.test.Install org.test.Sdk org.test.Platform
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin hello)
^chmod +x ($app | path join files bin hello)
^$env.FLATPAK build-finish $app --command hello
^$env.FLATPAK --user install $app

let install_dir = $env.HOME | path join .local share flatpak
let found = (do { ^find $install_dir -type d -name "org.test.Install" } | complete | get stdout | str trim)
if ($found | is-empty) {
  print "FAIL: org.test.Install not found in user installation directory"
  print -n (do { ^find $install_dir -type d } | complete | get stdout)
  exit 1
}

print "PASS: install-from-dir"
