#!/usr/bin/env nu

let app = $env.WORK | path join myapp
^$env.FLATPAK build-init $app org.test.App org.test.Sdk org.test.Platform
^$env.FLATPAK build-finish $app --command mycommand

let meta = $app | path join metadata
if ($meta | path type) != "file" {
  print "FAIL: metadata file not found"
  exit 1
}

if (do { ^grep -q "command=mycommand" $meta } | complete).exit_code != 0 {
  print "FAIL: metadata does not contain 'command=mycommand'"
  ^cat $meta
  exit 1
}

print "PASS: build-finish-command"
