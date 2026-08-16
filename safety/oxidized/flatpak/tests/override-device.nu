#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

^$env.FLATPAK --user override --device dri org.test.Hello
^$env.FLATPAK --user override --nodevice kvm org.test.Hello

let override_file = ($env.HOME | path join .local share flatpak overrides org.test.Hello)

if not ($override_file | path exists) {
  print $"FAIL: override file not created at ($override_file)"
  exit 1
}

if (do { ^grep -q "devices" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'devices'"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "dri" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'dri'"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "!kvm" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain '!kvm'"
  ^cat $override_file
  exit 1
}

print "PASS: override-device"
