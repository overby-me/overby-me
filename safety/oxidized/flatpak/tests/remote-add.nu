#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

^$env.FLATPAK --user remote-add testremote https://example.com/repo

# Check that the remote was added somewhere in the flatpak config
let remotes_dir = $env.HOME | path join .local share flatpak
if (do { ^grep -rl "testremote" $remotes_dir } | complete).exit_code != 0 {
    print $"FAIL: 'testremote' not found in flatpak config under ($remotes_dir)"
    exit 1
}

if (do { ^grep -rl "https://example.com/repo" $remotes_dir } | complete).exit_code != 0 {
    print $"FAIL: 'https://example.com/repo' not found in flatpak config under ($remotes_dir)"
    exit 1
}

print "PASS: remote-add created testremote with correct URL"
