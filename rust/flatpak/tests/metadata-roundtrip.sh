#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/rtapp" org.test.Roundtrip org.test.Sdk org.test.Platform stable

# Check initial metadata
grep "name=org.test.Roundtrip" "$WORK/rtapp/metadata" || { echo "FAIL: name not in metadata"; cat "$WORK/rtapp/metadata"; exit 1; }
grep "runtime=org.test.Platform" "$WORK/rtapp/metadata" || { echo "FAIL: runtime not in metadata"; cat "$WORK/rtapp/metadata"; exit 1; }
grep "sdk=org.test.Sdk" "$WORK/rtapp/metadata" || { echo "FAIL: sdk not in metadata"; cat "$WORK/rtapp/metadata"; exit 1; }

# Add permissions
$FLATPAK build-finish "$WORK/rtapp" --command testcmd --share network --share ipc --socket x11 --socket wayland --socket pulseaudio --device dri --filesystem home --filesystem /tmp

# Check all fields survive
grep "command=testcmd" "$WORK/rtapp/metadata" || { echo "FAIL: command not in metadata"; cat "$WORK/rtapp/metadata"; exit 1; }
grep "shared" "$WORK/rtapp/metadata" | grep -q "network" || { echo "FAIL: network not in shared"; cat "$WORK/rtapp/metadata"; exit 1; }
grep "shared" "$WORK/rtapp/metadata" | grep -q "ipc" || { echo "FAIL: ipc not in shared"; cat "$WORK/rtapp/metadata"; exit 1; }
grep "sockets" "$WORK/rtapp/metadata" | grep -q "x11" || { echo "FAIL: x11 not in sockets"; cat "$WORK/rtapp/metadata"; exit 1; }
grep "sockets" "$WORK/rtapp/metadata" | grep -q "wayland" || { echo "FAIL: wayland not in sockets"; cat "$WORK/rtapp/metadata"; exit 1; }
grep "sockets" "$WORK/rtapp/metadata" | grep -q "pulseaudio" || { echo "FAIL: pulseaudio not in sockets"; cat "$WORK/rtapp/metadata"; exit 1; }
grep "devices" "$WORK/rtapp/metadata" | grep -q "dri" || { echo "FAIL: dri not in devices"; cat "$WORK/rtapp/metadata"; exit 1; }
grep "filesystems" "$WORK/rtapp/metadata" | grep -q "home" || { echo "FAIL: home not in filesystems"; cat "$WORK/rtapp/metadata"; exit 1; }
grep "filesystems" "$WORK/rtapp/metadata" | grep -q "/tmp" || { echo "FAIL: /tmp not in filesystems"; cat "$WORK/rtapp/metadata"; exit 1; }

# Verify original fields still present after build-finish
grep "name=org.test.Roundtrip" "$WORK/rtapp/metadata" || { echo "FAIL: name lost after build-finish"; cat "$WORK/rtapp/metadata"; exit 1; }
grep "runtime=org.test.Platform" "$WORK/rtapp/metadata" || { echo "FAIL: runtime lost after build-finish"; cat "$WORK/rtapp/metadata"; exit 1; }
grep "sdk=org.test.Sdk" "$WORK/rtapp/metadata" || { echo "FAIL: sdk lost after build-finish"; cat "$WORK/rtapp/metadata"; exit 1; }

echo "PASS: metadata-roundtrip"