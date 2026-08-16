#!/usr/bin/env nu

source ./libtest-nix.nu

setup_repo

# Verify history shows the install event
let r = (do { ^$env.FLATPAK --user history } | complete)
let output = $r.stdout + $r.stderr
print $"history after install: ($output)"

if ($output | str contains "install") {
    ok "history records install event"
} else {
    print "FAIL: history does not show install event"
    print $"Got: ($output)"
    exit 1
}

if ($output | str contains "org.test.Hello") {
    ok "history contains app ref"
} else {
    print "FAIL: history does not contain org.test.Hello"
    print $"Got: ($output)"
    exit 1
}

# Uninstall the app
^$env.FLATPAK --user uninstall org.test.Hello

# Verify history shows uninstall event
let r2 = (do { ^$env.FLATPAK --user history } | complete)
let output2 = $r2.stdout + $r2.stderr
print $"history after uninstall: ($output2)"

if ($output2 | str contains "uninstall") {
    ok "history records uninstall event"
} else {
    print "FAIL: history does not show uninstall event"
    print $"Got: ($output2)"
    exit 1
}

print "PASS: vm-history-install-uninstall"
