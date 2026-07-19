#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

^$env.FLATPAK --user remote-add modremote https://example.com/v1
^$env.FLATPAK --user remote-delete modremote
^$env.FLATPAK --user remote-add modremote https://example.com/v2

let remotes_dir = $env.HOME | path join .local share flatpak
if (do { ^grep -rl "https://example.com/v2" $remotes_dir } | complete).exit_code != 0 {
  print "FAIL: v2 URL not found in flatpak config"
  exit 1
}

if (do { ^grep -rl "https://example.com/v1" $remotes_dir } | complete).exit_code == 0 {
  print "FAIL: old v1 URL still present in flatpak config"
  exit 1
}

print "PASS: remote-modify-implicit"
