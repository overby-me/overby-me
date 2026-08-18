#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

^$env.FLATPAK --user override --env FOO=bar org.test.Multi
^$env.FLATPAK --user override --env BAZ=qux org.test.Multi
^$env.FLATPAK --user override --env EMPTY= org.test.Multi

let override_file = ($env.HOME | path join .local share flatpak overrides org.test.Multi)

if not ($override_file | path exists) {
  print $"FAIL: override file not created at ($override_file)"
  exit 1
}

if (do { ^grep -q "FOO=bar" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'FOO=bar'"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "BAZ=qux" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'BAZ=qux'"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "EMPTY=" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'EMPTY='"
  ^cat $override_file
  exit 1
}

print "PASS: override-multiple-env"
