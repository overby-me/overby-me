#!/usr/bin/env nu

let app = $env.WORK | path join metaapp
^$env.FLATPAK build-init $app org.test.Meta org.test.Sdk org.test.Platform
mkdir ($app | path join files share metainfo)
"<component>test</component>\n" | save -f --raw ($app | path join files share metainfo org.test.Meta.metainfo.xml)
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin test)
^$env.FLATPAK build-finish $app --command test
if (($app | path join export share metainfo org.test.Meta.metainfo.xml) | path type) != "file" {
  exit 1
}

print "PASS: build-finish-export-metainfo"
