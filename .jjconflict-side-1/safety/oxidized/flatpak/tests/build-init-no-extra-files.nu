#!/usr/bin/env nu

let app = $env.WORK | path join cleanapp
^$env.FLATPAK build-init $app org.test.Clean org.test.Sdk org.test.Platform
# Expected: metadata, files/, var/
if (($app | path join metadata) | path type) != "file" { exit 1 }
if (($app | path join files) | path type) != "dir" { exit 1 }
if (($app | path join var) | path type) != "dir" { exit 1 }
# No other top-level items besides metadata, files, var
let count = (ls $app | length)
if $count > 3 {
  let listing = (^ls $app | str trim -r -c "\n")
  print $"FAIL: unexpected files in build dir: ($listing)"
  exit 1
}

print "PASS: build-init-no-extra-files"
