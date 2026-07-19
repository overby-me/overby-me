#!/usr/bin/env nu

$env.DBUS_SESSION_BUS_ADDRESS = "unix:path=/nonexistent"

mkdir ($env.WORK | path join docs)
"data\n" | save -f --raw ($env.WORK | path join docs a.txt)
let abs = $env.WORK | path join docs a.txt

let r = (do { ^$env.FLATPAK document-export $abs } | complete)
let id = (($r.stdout + $r.stderr) | ^sed -n 's/^Exported as: //p' | lines | get 0? | default "")
if ($id | is-empty) {
  print "FAIL: empty doc id"
  exit 1
}

let r = (do { ^$env.FLATPAK document-info $id } | complete)
let info = ($r.stdout + $r.stderr)
if not ($info | str contains $abs) {
  print "FAIL: info missing path"
  print $info
  exit 1
}

^$env.FLATPAK document-unexport $id
let r = (do { ^$env.FLATPAK document-info $id } | complete)
if $r.exit_code == 0 {
  print "FAIL: info should fail"
  exit 1
}

print "PASS: document-local-store"
