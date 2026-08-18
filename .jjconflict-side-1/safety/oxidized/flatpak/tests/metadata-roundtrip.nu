#!/usr/bin/env nu

^$env.FLATPAK build-init ($env.WORK | path join rtapp) org.test.Roundtrip org.test.Sdk org.test.Platform stable

let meta = ($env.WORK | path join rtapp metadata)

# Check initial metadata
if (do { ^grep "name=org.test.Roundtrip" $meta } | complete).exit_code != 0 { print "FAIL: name not in metadata"; ^cat $meta; exit 1 }
if (do { ^grep "runtime=org.test.Platform" $meta } | complete).exit_code != 0 { print "FAIL: runtime not in metadata"; ^cat $meta; exit 1 }
if (do { ^grep "sdk=org.test.Sdk" $meta } | complete).exit_code != 0 { print "FAIL: sdk not in metadata"; ^cat $meta; exit 1 }

# Add permissions
^$env.FLATPAK build-finish ($env.WORK | path join rtapp) --command testcmd --share network --share ipc --socket x11 --socket wayland --socket pulseaudio --device dri --filesystem home --filesystem /tmp

# Check all fields survive
if (do { ^grep "command=testcmd" $meta } | complete).exit_code != 0 { print "FAIL: command not in metadata"; ^cat $meta; exit 1 }
if (do { ^grep "shared" $meta | ^grep -q "network" } | complete).exit_code != 0 { print "FAIL: network not in shared"; ^cat $meta; exit 1 }
if (do { ^grep "shared" $meta | ^grep -q "ipc" } | complete).exit_code != 0 { print "FAIL: ipc not in shared"; ^cat $meta; exit 1 }
if (do { ^grep "sockets" $meta | ^grep -q "x11" } | complete).exit_code != 0 { print "FAIL: x11 not in sockets"; ^cat $meta; exit 1 }
if (do { ^grep "sockets" $meta | ^grep -q "wayland" } | complete).exit_code != 0 { print "FAIL: wayland not in sockets"; ^cat $meta; exit 1 }
if (do { ^grep "sockets" $meta | ^grep -q "pulseaudio" } | complete).exit_code != 0 { print "FAIL: pulseaudio not in sockets"; ^cat $meta; exit 1 }
if (do { ^grep "devices" $meta | ^grep -q "dri" } | complete).exit_code != 0 { print "FAIL: dri not in devices"; ^cat $meta; exit 1 }
if (do { ^grep "filesystems" $meta | ^grep -q "home" } | complete).exit_code != 0 { print "FAIL: home not in filesystems"; ^cat $meta; exit 1 }
if (do { ^grep "filesystems" $meta | ^grep -q "/tmp" } | complete).exit_code != 0 { print "FAIL: /tmp not in filesystems"; ^cat $meta; exit 1 }

# Verify original fields still present after build-finish
if (do { ^grep "name=org.test.Roundtrip" $meta } | complete).exit_code != 0 { print "FAIL: name lost after build-finish"; ^cat $meta; exit 1 }
if (do { ^grep "runtime=org.test.Platform" $meta } | complete).exit_code != 0 { print "FAIL: runtime lost after build-finish"; ^cat $meta; exit 1 }
if (do { ^grep "sdk=org.test.Sdk" $meta } | complete).exit_code != 0 { print "FAIL: sdk lost after build-finish"; ^cat $meta; exit 1 }

print "PASS: metadata-roundtrip"
