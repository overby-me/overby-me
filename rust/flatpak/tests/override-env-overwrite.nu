#!/usr/bin/env nu

^$env.FLATPAK --user override --env FOO=first org.test.EnvOW
^$env.FLATPAK --user override --env FOO=second org.test.EnvOW

let override_file = ($env.HOME | path join .local share flatpak overrides org.test.EnvOW)

if not ($override_file | path exists) {
  print $"FAIL: override file not created at ($override_file)"
  exit 1
}

if (do { ^grep -q "FOO=first" $override_file } | complete).exit_code == 0 {
  print "FAIL: override file still contains FOO=first (should have been overwritten)"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "FOO=second" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain FOO=second"
  ^cat $override_file
  exit 1
}

print "PASS: override-env-overwrite"
