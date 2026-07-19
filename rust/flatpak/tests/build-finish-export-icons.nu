#!/usr/bin/env nu

let app = $env.WORK | path join iconapp
^$env.FLATPAK build-init $app org.test.Icons org.test.Sdk org.test.Platform
mkdir ($app | path join files share icons hicolor 64x64 apps)
"PNG_DATA\n" | save -f --raw ($app | path join files share icons hicolor 64x64 apps org.test.Icons.png)
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin test)
^$env.FLATPAK build-finish $app --command test
if (($app | path join export share icons hicolor 64x64 apps org.test.Icons.png) | path type) != "file" {
  exit 1
}

print "PASS: build-finish-export-icons"
