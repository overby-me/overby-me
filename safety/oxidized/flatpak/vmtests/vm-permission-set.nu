#!/usr/bin/env nu
source ./libtest-nix.nu

let r = (do { ^$env.FLATPAK permission-set notifications myid org.test.App allow } | complete)
let out = $r.stdout + $r.stderr
print $out
if not ($out | lines | any {|l| $l =~ '(?i)permission set' }) {
    print "FAIL: expected 'Permission set' message"
    exit 1
}
ok "permission-set succeeded"

# show should now include it
let sr = (do { ^$env.FLATPAK permission-show org.test.App } | complete)
let show = $sr.stdout + $sr.stderr
print $show
if not ($show | lines | any {|l| $l =~ 'notifications/myid' }) {
    print "FAIL: permission not visible via show"
    exit 1
}
ok "permission visible via show"

print "PASS: vm-permission-set"
