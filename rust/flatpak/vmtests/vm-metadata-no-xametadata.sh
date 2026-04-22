#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Build an app then strip the metadata file. Equivalent to an OSTree commit
# missing xa.metadata: the install must reject it instead of installing a
# silently broken app.
build_dir=$(make_test_app org.test.NoMeta stable)
rm -f "$build_dir/metadata"
ok "metadata stripped from build dir"

set +e
output=$($FLATPAK --user install "$build_dir" 2>&1)
rc=$?
set -e

echo "install output: $output"
echo "install rc: $rc"

if [ "$rc" -eq 0 ]; then
  echo "FAIL: install should have failed without metadata"
  exit 1
fi
ok "install rejected (no metadata)"

if echo "$output" | grep -q "no metadata"; then
  ok "error message mentions missing metadata"
else
  echo "FAIL: error message did not mention missing metadata"
  echo "Got: $output"
  exit 1
fi

# Verify nothing was deployed.
if find "$FL_DIR" -path "*org.test.NoMeta*" -type d | grep -q .; then
  echo "FAIL: org.test.NoMeta should not have been deployed"
  exit 1
fi
ok "no deployment created"

echo "PASS: vm-metadata-no-xametadata"
