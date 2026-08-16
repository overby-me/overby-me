#!/usr/bin/env nu

^$env.FLATPAK --user override --socket x11 --device dri --share network org.test.IniFormat

let override_file = ($env.HOME | path join .local share flatpak overrides org.test.IniFormat)

if not ($override_file | path exists) {
  print $"FAIL: override file not created at ($override_file)"
  exit 1
}

if (do { ^grep -q '^\[Context\]' $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain [Context] group header"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "sockets=.*x11" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'sockets=.*x11'"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "devices=.*dri" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'devices=.*dri'"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "shared=.*network" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'shared=.*network'"
  ^cat $override_file
  exit 1
}

print "PASS: override-file-format"
