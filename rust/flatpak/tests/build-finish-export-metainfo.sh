#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/metaapp" org.test.Meta org.test.Sdk org.test.Platform
mkdir -p "$WORK/metaapp/files/share/metainfo"
echo "<component>test</component>" > "$WORK/metaapp/files/share/metainfo/org.test.Meta.metainfo.xml"
mkdir -p "$WORK/metaapp/files/bin"
echo '#!/bin/sh' > "$WORK/metaapp/files/bin/test"
$FLATPAK build-finish "$WORK/metaapp" --command test
[ -f "$WORK/metaapp/export/share/metainfo/org.test.Meta.metainfo.xml" ]

echo "PASS: build-finish-export-metainfo"