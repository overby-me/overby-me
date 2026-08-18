#!/usr/bin/env nu

r#'[Flatpak Repo]
Title=Alt Remote
Url=https://alt.example.com/repo
'# | save -f --raw ($env.WORK | path join test2.flatpakrepo)

^$env.FLATPAK --user remote-add --from ($env.WORK | path join test2.flatpakrepo)

let r = (do { ^$env.FLATPAK --user remotes } | complete)
let output = ($r.stdout + $r.stderr) | str trim -r -c "\n"

if not ($output =~ "test2") {
  print "FAIL: remote 'test2' not found in remotes output"
  print $output
  exit 1
}

print "PASS: remote-add-flatpakrepo-url"
