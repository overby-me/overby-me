#!/usr/bin/env nu

let app = $env.WORK | path join exportinst
^$env.FLATPAK build-init $app org.test.ExportInst org.test.Sdk org.test.Platform
mkdir ($app | path join files share applications)
"[Desktop Entry]\nName=Test\nExec=test\nType=Application\n" | save -f --raw ($app | path join files share applications org.test.ExportInst.desktop)
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin test)
^chmod +x ($app | path join files bin test)
^$env.FLATPAK build-finish $app --command test

^$env.FLATPAK --user install $app

let inst_dir = $env.HOME | path join .local share flatpak

# At minimum the metadata should be there
if (do { ^find $inst_dir -name "metadata" -exec grep -l "org.test.ExportInst" "{}" "+" } | complete).exit_code != 0 {
  print "FAIL: metadata for org.test.ExportInst not found in install dir"
  print -n (do { ^find $inst_dir -type f } | complete | get stdout)
  exit 1
}

print "PASS: install-dir-with-export"
