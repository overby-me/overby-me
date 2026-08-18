#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

^$env.FLATPAK --user override --share network org.test.Hello
^$env.FLATPAK --user override --unshare ipc org.test.Hello

let override_file = ($env.HOME | path join .local share flatpak overrides org.test.Hello)

if not ($override_file | path exists) {
  print $"FAIL: override file not found at ($override_file)"
  exit 1
}

print "Override file contents:"
^cat $override_file

if (do { ^grep -qi "shared" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'shared' section"
  exit 1
}

if (do { ^grep -q "network" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'network'"
  exit 1
}

if (do { ^grep -q "!ipc" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain '!ipc'"
  exit 1
}

print "PASS: override share/unshare works correctly"
