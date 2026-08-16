#!/usr/bin/env nu
source ./libtest-nix.nu

# Empty show returns "No permissions" message.
let r0 = (do { ^$env.FLATPAK permission-show org.test.NoApp } | complete)
let out = $r0.stdout + $r0.stderr
if not ($out | lines | any {|l| $l =~ 'No permissions' }) {
    print "FAIL: expected 'No permissions' for empty app"
    print $"Got: ($out)"
    exit 1
}
ok "empty show works"

^$env.FLATPAK permission-set background mybg org.test.AppX allow
^$env.FLATPAK permission-set notifications mynotif org.test.AppX deny

let sr = (do { ^$env.FLATPAK permission-show org.test.AppX } | complete)
let show = $sr.stdout + $sr.stderr
print $show
if (not ($show | lines | any {|l| $l =~ 'background/mybg' })) or (not ($show | lines | any {|l| $l =~ 'notifications/mynotif' })) {
    print "FAIL: not all permissions visible"
    print $"Got: ($show)"
    exit 1
}
ok "show lists all permissions"

# Other apps should not show these.
let orr = (do { ^$env.FLATPAK permission-show org.test.Other } | complete)
let other = $orr.stdout + $orr.stderr
if ($other | lines | any {|l| $l =~ 'mybg|mynotif' }) {
    print "FAIL: leaked permissions to other app"
    exit 1
}
ok "permissions are scoped per app-id"

print "PASS: vm-permission-show"
