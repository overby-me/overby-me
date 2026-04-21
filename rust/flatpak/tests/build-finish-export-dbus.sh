#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/dbusapp" org.test.DBus org.test.Sdk org.test.Platform
mkdir -p "$WORK/dbusapp/files/share/dbus-1/services"
echo "[D-BUS Service]" > "$WORK/dbusapp/files/share/dbus-1/services/org.test.DBus.service"
echo "Name=org.test.DBus" >> "$WORK/dbusapp/files/share/dbus-1/services/org.test.DBus.service"
mkdir -p "$WORK/dbusapp/files/bin"
echo '#!/bin/sh' > "$WORK/dbusapp/files/bin/test"
$FLATPAK build-finish "$WORK/dbusapp" --command test
[ -f "$WORK/dbusapp/export/share/dbus-1/services/org.test.DBus.service" ] || { echo "FAIL: dbus service not exported"; exit 1; }

echo "PASS: build-finish-export-dbus"