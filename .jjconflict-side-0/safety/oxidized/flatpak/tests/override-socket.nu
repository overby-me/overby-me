#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

^$env.FLATPAK --user override --socket wayland org.test.Hello
^$env.FLATPAK --user override --nosocket ssh-auth org.test.Hello

let override_file = ($env.HOME | path join .local share flatpak overrides org.test.Hello)

if not ($override_file | path exists) {
  print $"FAIL: override file not created at ($override_file)"
  exit 1
}

if (do { ^grep -qi "sockets" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'sockets' section"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "wayland" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'wayland'"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "!ssh-auth" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain '!ssh-auth'"
  ^cat $override_file
  exit 1
}

print "PASS: override-socket"
