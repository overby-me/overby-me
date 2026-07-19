#!/usr/bin/env nu

let app = $env.WORK | path join ssdkinfo
^$env.FLATPAK build-init $app org.test.SSdkInfo org.test.Sdk org.test.Platform stable
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^chmod +x ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app --sdk=org.test.Sdk/x86_64/stable
^$env.FLATPAK --user install $app

let output = (do { ^$env.FLATPAK --user info --show-sdk org.test.SSdkInfo } | complete | get stdout)
if ($output | str contains "org.test.Sdk") {
    print "PASS: info-show-sdk"
} else {
    print $"FAIL: info-show-sdk \(expected 'org.test.Sdk' in output, got: ($output)\)"
    exit 1
}
