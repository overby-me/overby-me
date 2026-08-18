#!/usr/bin/env nu

let app = $env.WORK | path join dirapp
^$env.FLATPAK build-init $app org.test.Dirs org.test.Sdk org.test.Platform

for d in [files "files/bin" "files/lib" "files/share" var "var/tmp" "var/lib" "var/run"] {
  if (($app | path join $d) | path type) != "dir" {
    print $"FAIL: expected directory ($d) not created"
    exit 1
  }
}

print "PASS: build-init-dirs"
