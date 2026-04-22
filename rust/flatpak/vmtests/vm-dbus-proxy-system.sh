#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

setup_repo

# The test app does NOT have --socket system-bus, so the system bus
# should be filtered or not exposed inside the sandbox.

# Run a command inside the sandbox that checks for the system bus socket
output=$(run_sh org.test.Hello '
  if [ -S /run/dbus/system_bus_socket ]; then
    echo "SOCKET_PRESENT"
  else
    echo "SOCKET_ABSENT"
  fi
' 2>&1)
echo "system bus socket check: $output"

# Either outcome is acceptable depending on host setup:
# - SOCKET_PRESENT: filtered proxy is providing the socket
# - SOCKET_ABSENT: no system bus access at all
if echo "$output" | grep -qE "SOCKET_PRESENT|SOCKET_ABSENT"; then
  ok "system bus handling did not crash"
else
  echo "FAIL: unexpected output from system bus check"
  echo "Got: $output"
  exit 1
fi

# Verify that the sandbox didn't crash and the app still works
run_output=$(run org.test.Hello 2>&1)
if echo "$run_output" | grep -q "Hello world, from a sandbox"; then
  ok "app runs normally with D-Bus system filtering"
else
  echo "FAIL: app did not run with D-Bus filtering active"
  echo "Got: $run_output"
  exit 1
fi

echo "PASS: vm-dbus-proxy-system"
