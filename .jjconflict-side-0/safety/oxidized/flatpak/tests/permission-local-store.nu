#!/usr/bin/env nu

# Local fallback portal: works without a real session bus.
$env.DBUS_SESSION_BUS_ADDRESS = "unix:path=/nonexistent"

^$env.FLATPAK permission-set notifications myid org.test.App allow
^$env.FLATPAK permission-set background bgid org.test.App deny

let r = (do { ^$env.FLATPAK permission-show org.test.App } | complete)
let show = ($r.stdout + $r.stderr) | str trim -r -c "\n"
if not ($show =~ "notifications/myid") {
  print "FAIL: notifications/myid missing"; print $show; exit 1
}
if not ($show =~ "background/bgid") {
  print "FAIL: background/bgid missing"; print $show; exit 1
}

# Other apps don't see these.
let r = (do { ^$env.FLATPAK permission-show org.test.NeverSeenBefore.X9XQ } | complete)
let other = ($r.stdout + $r.stderr) | str trim -r -c "\n"
if not ($other =~ "No permissions") {
  print "FAIL: scoped show broken"; print $other; exit 1
}

# Remove + reset.
^$env.FLATPAK permission-remove notifications myid
let r = (do { ^$env.FLATPAK permission-show org.test.App } | complete)
let remaining = ($r.stdout + $r.stderr) | str trim -r -c "\n"
if $remaining =~ "notifications/myid" { print "FAIL: not removed"; exit 1 }

^$env.FLATPAK permission-reset org.test.App
let r = (do { ^$env.FLATPAK permission-show org.test.App } | complete)
let final = ($r.stdout + $r.stderr) | str trim -r -c "\n"
if not ($final =~ "No permissions") {
  print "FAIL: not reset"; print $final; exit 1
}

print "PASS: permission-local-store"
