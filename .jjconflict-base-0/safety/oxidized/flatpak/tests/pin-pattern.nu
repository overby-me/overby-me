#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

^$env.FLATPAK --user pin org.test.Pinned

# Check that a pin file was created somewhere under the installation
let pins_dir = $env.HOME | path join .local share flatpak
if (do { ^grep -rl "org.test.Pinned" $pins_dir } | complete).exit_code != 0 {
  print $"FAIL: pin for org.test.Pinned not found under ($pins_dir)"
  glob ($pins_dir | path join "**" "*") --no-dir | first 20 | to text | print
  exit 1
}

print "PASS: pin-pattern"
