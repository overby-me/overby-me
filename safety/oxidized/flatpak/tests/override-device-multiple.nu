#!/usr/bin/env nu

^$env.FLATPAK --user override --device dri org.test.DevMulti
^$env.FLATPAK --user override --device kvm org.test.DevMulti
^$env.FLATPAK --user override --nodevice shm org.test.DevMulti

let override_file = ($env.HOME | path join .local share flatpak overrides org.test.DevMulti)

if not ($override_file | path exists) {
  print $"FAIL: override file not created at ($override_file)"
  exit 1
}

if (do { ^grep -q "dri" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'dri'"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "kvm" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'kvm'"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "!shm" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain '!shm'"
  ^cat $override_file
  exit 1
}

print "PASS: override-device-multiple"
