#!/usr/bin/env nu

let app = $env.WORK | path join dbusapp
^$env.FLATPAK build-init $app org.test.DBus org.test.Sdk org.test.Platform
mkdir ($app | path join files share dbus-1 services)
"[D-BUS Service]\nName=org.test.DBus\n" | save -f --raw ($app | path join files share dbus-1 services org.test.DBus.service)
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin test)
^$env.FLATPAK build-finish $app --command test
if (($app | path join export share dbus-1 services org.test.DBus.service) | path type) != "file" {
  print "FAIL: dbus service not exported"
  exit 1
}

print "PASS: build-finish-export-dbus"
