#!/usr/bin/env nu

source ./libtest-nix.nu

# Test that the `complete` subcommand prints expected candidates.

let r = (do { ^$env.FLATPAK complete } | complete)
let out = $r.stdout + $r.stderr
print "complete (no args) output:"
print $out
for cmd in [install run list] {
    if not ($out | lines | any {|l| $l == $cmd }) {
        print $"FAIL: 'flatpak complete' missing '($cmd)'"
        exit 1
    }
}
ok "flatpak complete includes install, run, list"

let r2 = (do { ^$env.FLATPAK complete remote- } | complete)
let out2 = $r2.stdout + $r2.stderr
print "complete remote- output:"
print $out2
for cmd in [remote-add remote-delete remote-ls remote-info] {
    if not ($out2 | lines | any {|l| $l == $cmd }) {
        print $"FAIL: 'flatpak complete remote-' missing '($cmd)'"
        exit 1
    }
}
if ($out2 | lines | any {|l| $l == "install" }) {
    print "FAIL: 'flatpak complete remote-' should not list 'install'"
    exit 1
}
ok "flatpak complete remote- filters to remote-* only"

print "PASS: vm-completion"
