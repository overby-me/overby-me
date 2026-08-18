#!/usr/bin/env nu

^$env.FLATPAK --user override --share network org.test.ShareMulti
^$env.FLATPAK --user override --share ipc org.test.ShareMulti
^$env.FLATPAK --user override --unshare cups org.test.ShareMulti

let override_file = ($env.HOME | path join .local share flatpak overrides org.test.ShareMulti)

if not ($override_file | path exists) {
  print $"FAIL: override file not created at ($override_file)"
  exit 1
}

if (do { ^grep -q "network" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'network'"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "ipc" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'ipc'"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "!cups" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain '!cups'"
  ^cat $override_file
  exit 1
}

print "PASS: override-share-multiple"
