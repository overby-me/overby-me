#!/usr/bin/env nu

let out = (do { ^$env.FLATPAK complete } | complete)
let out_text = ($out.stdout + $out.stderr)
for cmd in [install uninstall list run] {
  if (do { $out_text | ^grep -qx $cmd } | complete).exit_code != 0 {
    print $"FAIL: 'flatpak complete' missing '($cmd)'"
    print $out_text
    exit 1
  }
}

let out2 = (do { ^$env.FLATPAK complete inst } | complete)
let out2_text = ($out2.stdout + $out2.stderr)
if (do { $out2_text | ^grep -qx "install" } | complete).exit_code != 0 {
  print "FAIL: 'flatpak complete inst' missing 'install'"
  print $out2_text
  exit 1
}
if (do { $out2_text | ^grep -qx "run" } | complete).exit_code == 0 {
  print "FAIL: 'flatpak complete inst' should not list 'run'"
  print $out2_text
  exit 1
}

print "PASS: complete-lists-commands"
