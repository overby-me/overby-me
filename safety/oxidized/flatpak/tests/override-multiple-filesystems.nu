#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

^$env.FLATPAK --user override --filesystem home org.test.Multi
^$env.FLATPAK --user override --filesystem /tmp org.test.Multi
^$env.FLATPAK --user override --filesystem xdg-desktop org.test.Multi

let override_file = ($env.HOME | path join .local share flatpak overrides org.test.Multi)

if not ($override_file | path exists) {
  print $"FAIL: override file not created at ($override_file)"
  exit 1
}

if (do { ^grep -q "home" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'home'"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "/tmp" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain '/tmp'"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "xdg-desktop" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'xdg-desktop'"
  ^cat $override_file
  exit 1
}

print "PASS: override-multiple-filesystems"
