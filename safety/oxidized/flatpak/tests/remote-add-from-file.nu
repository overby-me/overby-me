#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

r#'[Flatpak Repo]
Title=Test Remote
Url=https://example.com/test-repo
'# | save -f --raw ($env.WORK | path join test.flatpakrepo)

^$env.FLATPAK --user remote-add --from ($env.WORK | path join test.flatpakrepo)

let remotes_dir = $env.HOME | path join .local share flatpak

if (do { ^grep -rl "https://example.com/test-repo" $remotes_dir } | complete).exit_code != 0 {
  print $"FAIL: remote URL 'https://example.com/test-repo' not found in flatpak config under ($remotes_dir)"
  exit 1
}

print "PASS: remote-add-from-file"
