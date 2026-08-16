#!/usr/bin/env nu

^$env.FLATPAK --user remote-add urlcheck https://url.example.com/flatpak

# Check the config file directly for the URL
let remotes_dir = $env.HOME | path join .local share flatpak
if (do { ^grep -rl "url.example.com" $remotes_dir } | complete).exit_code != 0 {
  print "FAIL: could not find url.example.com in any flatpak config file"
  try { glob ($remotes_dir | path join "**" "*") --no-dir | to text | print }
  exit 1
}

print "PASS: remote-add-then-list-url"
