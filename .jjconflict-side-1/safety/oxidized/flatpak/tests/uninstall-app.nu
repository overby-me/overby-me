#!/usr/bin/env nu

let uninstapp = $env.WORK | path join uninstapp
^$env.FLATPAK build-init $uninstapp org.test.Uninst org.test.Sdk org.test.Platform
mkdir ($uninstapp | path join files bin)
"#!/bin/sh\n" | save -f --raw ($uninstapp | path join files bin app)
^chmod +x ($uninstapp | path join files bin app)
^$env.FLATPAK build-finish $uninstapp --command app
^$env.FLATPAK --user install $uninstapp

# Verify it was installed
let install_dir = $env.HOME | path join .local share flatpak
if (glob ($install_dir | path join "**" org.test.Uninst) | where {|p| ($p | path type) == "dir" } | is-empty) {
  print "FAIL: app was not installed"
  exit 1
}

^$env.FLATPAK --user uninstall org.test.Uninst

# Check the deployment directory is gone
let leftover = (glob ($install_dir | path join app org.test.Uninst "**" "*") | where {|p| ($p | path type) == "dir" })
if ($leftover | is-not-empty) {
  print "FAIL: deployment directory still exists after uninstall"
  glob ($install_dir | path join "**" org.test.Uninst) | where {|p| ($p | path type) == "dir" } | to text | print
  exit 1
}

print "PASS: uninstall-app"
