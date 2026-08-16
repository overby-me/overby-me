#!/usr/bin/env nu

let app = $env.WORK | path join slocinfo
^$env.FLATPAK build-init $app org.test.SLocInfo org.test.Sdk org.test.Platform stable
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^chmod +x ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
^$env.FLATPAK --user install $app
let output = (do { ^$env.FLATPAK --user info --show-location org.test.SLocInfo } | complete | get stdout)
if ($output | str contains "org.test.SLocInfo") {
    print "PASS: info-show-location"
} else {
    print $"FAIL: info-show-location \(expected path containing org.test.SLocInfo, got: ($output)\)"
    exit 1
}
