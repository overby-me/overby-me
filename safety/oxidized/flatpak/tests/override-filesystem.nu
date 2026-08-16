#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

^$env.FLATPAK --user override --filesystem home org.test.Hello

let override_file = ($env.HOME | path join .local share flatpak overrides org.test.Hello)

if not ($override_file | path exists) {
  print $"FAIL: override file not created at ($override_file)"
  exit 1
}

if (do { ^grep -qi "filesystems" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file missing 'filesystems' key"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "home" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file missing 'home' filesystem"
  ^cat $override_file
  exit 1
}

print "PASS: override-filesystem"
