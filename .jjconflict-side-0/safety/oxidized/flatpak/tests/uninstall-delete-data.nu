#!/usr/bin/env nu

let deldata = $env.WORK | path join deldata
^$env.FLATPAK build-init $deldata org.test.DelData org.test.Sdk org.test.Platform
mkdir ($deldata | path join files bin)
"#!/bin/sh\n" | save -f --raw ($deldata | path join files bin app)
^$env.FLATPAK build-finish $deldata --command app
^$env.FLATPAK --user install $deldata

# Create app data
let data_dir = $env.HOME | path join .var app org.test.DelData data
mkdir $data_dir
"userdata\n" | save -f --raw ($data_dir | path join file.txt)

# Uninstall without --delete-data: data should remain
^$env.FLATPAK --user uninstall org.test.DelData
if not (($data_dir | path join file.txt) | path exists) {
  print "FAIL: data deleted without --delete-data"; exit 1
}

# Re-install and uninstall with --delete-data
^$env.FLATPAK --user install $deldata
^$env.FLATPAK --user uninstall --delete-data org.test.DelData
if (($env.HOME | path join .var app org.test.DelData) | path exists) {
  print "FAIL: data dir still exists after --delete-data"
  exit 1
}

print "PASS: uninstall-delete-data"
