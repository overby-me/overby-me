#!/usr/bin/env nu

let app = $env.WORK | path join brapp
^$env.FLATPAK build-init $app org.test.BrApp org.test.Sdk org.test.Platform mybranch
^grep -q "mybranch" ($app | path join metadata)

print "PASS: build-init-custom-branch"
