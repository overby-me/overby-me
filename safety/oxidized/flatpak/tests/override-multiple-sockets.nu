#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

^$env.FLATPAK --user override --socket x11 org.test.Multi
^$env.FLATPAK --user override --socket wayland org.test.Multi
^$env.FLATPAK --user override --socket pulseaudio org.test.Multi

let override_file = ($env.HOME | path join .local share flatpak overrides org.test.Multi)

if not ($override_file | path exists) {
  print $"FAIL: override file not created at ($override_file)"
  exit 1
}

if (do { ^grep -q "x11" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'x11'"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "wayland" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'wayland'"
  ^cat $override_file
  exit 1
}

if (do { ^grep -q "pulseaudio" $override_file } | complete).exit_code != 0 {
  print "FAIL: override file does not contain 'pulseaudio'"
  ^cat $override_file
  exit 1
}

print "PASS: override-multiple-sockets"
