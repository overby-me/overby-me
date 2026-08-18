#!/usr/bin/env nu
source ./libtest-nix.nu

^$env.FLATPAK permission-set notifications a org.test.Reset allow
^$env.FLATPAK permission-set background b org.test.Reset allow
^$env.FLATPAK permission-set devices c org.test.Reset allow
ok "seeded permissions across 3 tables"

let br = (do { ^$env.FLATPAK permission-show org.test.Reset } | complete)
let before = (($br.stdout + $br.stderr) | lines | where {|l| $l =~ ':' } | length)
if $before < 3 {
    print $"FAIL: expected at least 3 permission rows before reset, got ($before)"
    ^$env.FLATPAK permission-show org.test.Reset
    exit 1
}

^$env.FLATPAK permission-reset org.test.Reset
ok "reset"

let ar = (do { ^$env.FLATPAK permission-show org.test.Reset } | complete)
let after = $ar.stdout + $ar.stderr
if not ($after | lines | any {|l| $l =~ 'No permissions' }) {
    print "FAIL: expected 'No permissions' after reset"
    print $"Got: ($after)"
    exit 1
}
ok "no permissions remain after reset"

# Permissions for other apps must survive.
^$env.FLATPAK permission-set notifications x org.test.Other allow
^$env.FLATPAK permission-reset org.test.Reset
let or = (do { ^$env.FLATPAK permission-show org.test.Other } | complete)
let other = $or.stdout + $or.stderr
if not ($other | lines | any {|l| $l =~ 'notifications/x' }) {
    print "FAIL: reset clobbered other app permissions"
    print $"Got: ($other)"
    exit 1
}
ok "reset is scoped to target app"

print "PASS: vm-permission-reset"
