#!/usr/bin/env nu
source ./libtest-nix.nu

^$env.FLATPAK permission-set notifications gone org.test.App allow
ok "set"
let sr0 = (do { ^$env.FLATPAK permission-show org.test.App } | complete)
if not (($sr0.stdout + $sr0.stderr) | lines | any {|l| $l =~ 'notifications/gone' }) {
    print "FAIL: setup"
    exit 1
}

^$env.FLATPAK permission-remove notifications gone
ok "remove"

let sr = (do { ^$env.FLATPAK permission-show org.test.App } | complete)
let show = $sr.stdout + $sr.stderr
if ($show | lines | any {|l| $l =~ 'notifications/gone' }) {
    print "FAIL: permission still present after remove"
    print $"Got: ($show)"
    exit 1
}
ok "permission no longer present"

print "PASS: vm-permission-remove"
