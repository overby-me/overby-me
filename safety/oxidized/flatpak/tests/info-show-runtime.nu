#!/usr/bin/env nu

let app = $env.WORK | path join srtinfo
^$env.FLATPAK build-init $app org.test.SRTInfo org.test.Sdk org.test.Platform stable
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^chmod +x ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
^$env.FLATPAK --user install $app

let output = (do { ^$env.FLATPAK --user info --show-runtime org.test.SRTInfo } | complete | get stdout)
if ($output | str contains "org.test.Platform") {
    print "PASS: info-show-runtime"
} else {
    print $"FAIL: info-show-runtime \(expected 'org.test.Platform' in output, got '($output)'\)"
    exit 1
}
