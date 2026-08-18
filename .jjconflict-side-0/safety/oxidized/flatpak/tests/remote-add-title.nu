#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

^$env.FLATPAK --user remote-add titled-remote https://example.com/repo --title="My Remote"

let remotes_dir = $env.HOME | path join .local share flatpak

if (do { ^grep -rl "titled-remote" $remotes_dir } | complete).exit_code != 0 {
    print $"FAIL: 'titled-remote' not found in flatpak config under ($remotes_dir)"
    exit 1
}

if (do { ^grep -rl "My Remote" $remotes_dir } | complete).exit_code != 0 {
    print $"FAIL: 'My Remote' title not found in flatpak config under ($remotes_dir)"
    for f in (glob ($remotes_dir | path join "**" "*") --no-dir) { print (open --raw $f) }
    exit 1
}

print "PASS: remote-add-title"
