#!/usr/bin/env nu

let app = $env.WORK | path join deskapp
^$env.FLATPAK build-init $app org.test.Desktop org.test.Sdk org.test.Platform
mkdir ($app | path join files share applications)
"[Desktop Entry]\nName=Test\nExec=test\nType=Application\n" | save -f --raw ($app | path join files share applications org.test.Desktop.desktop)
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin test)
^$env.FLATPAK build-finish $app --command test
if (($app | path join export share applications org.test.Desktop.desktop) | path type) != "file" {
  print "FAIL: desktop file not exported"
  exit 1
}

print "PASS: build-finish-export-desktop"
