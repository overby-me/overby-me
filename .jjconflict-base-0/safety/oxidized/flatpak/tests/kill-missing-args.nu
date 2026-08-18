#!/usr/bin/env nu

let r = (try { do { ^$env.FLATPAK kill } | complete } catch { {crashed: true} })
if ($r | get crashed? | default false) {
  print "FAIL: kill crashed with signal"
  exit 1
}
let rc = $r.exit_code

if $rc == 139 or $rc == 134 or $rc == 136 {
  print $"FAIL: kill crashed with signal \(rc=($rc)\)"
  exit 1
}

print "PASS: kill-missing-args"
