#!/usr/bin/env nu

let app = $env.WORK | path join extapp
let meta = $app | path join metadata
^$env.FLATPAK build-init $app org.test.Ext org.test.Sdk org.test.Platform master --extension-tag org.test.Base

if ($meta | path type) != "file" {
  print "FAIL: metadata file not created"
  exit 1
}

if (do { ^grep -q '\[Runtime\]\|\[Application\]' $meta } | complete).exit_code != 0 {
  print "FAIL: metadata missing [Runtime] or [Application] section"
  ^cat $meta
  exit 1
}

if (do { ^grep -qi "ExtensionOf\|extension" $meta } | complete).exit_code != 0 {
  print "FAIL: metadata missing extension information"
  ^cat $meta
  exit 1
}

print "PASS: build-init-extension"
