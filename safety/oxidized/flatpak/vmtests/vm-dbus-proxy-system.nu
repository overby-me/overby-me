#!/usr/bin/env nu

source ./libtest-nix.nu

setup_repo

# The test app does NOT have --socket system-bus, so the system bus
# should be filtered or not exposed inside the sandbox.

# Run a command inside the sandbox that checks for the system bus socket
let r = (do { run_sh org.test.Hello '
  if [ -S /run/dbus/system_bus_socket ]; then
    echo "SOCKET_PRESENT"
  else
    echo "SOCKET_ABSENT"
  fi
' } | complete)
let output = $r.stdout + $r.stderr
print $"system bus socket check: ($output)"

# Either outcome is acceptable depending on host setup:
# - SOCKET_PRESENT: filtered proxy is providing the socket
# - SOCKET_ABSENT: no system bus access at all
if ($output | lines | any {|l| $l =~ "SOCKET_PRESENT|SOCKET_ABSENT" }) {
    ok "system bus handling did not crash"
} else {
    print "FAIL: unexpected output from system bus check"
    print $"Got: ($output)"
    exit 1
}

# Verify that the sandbox didn't crash and the app still works
let r2 = (do { run org.test.Hello } | complete)
let run_output = $r2.stdout + $r2.stderr
if ($run_output | str contains "Hello world, from a sandbox") {
    ok "app runs normally with D-Bus system filtering"
} else {
    print "FAIL: app did not run with D-Bus filtering active"
    print $"Got: ($run_output)"
    exit 1
}

print "PASS: vm-dbus-proxy-system"
