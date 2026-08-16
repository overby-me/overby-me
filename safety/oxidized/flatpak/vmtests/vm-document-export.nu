#!/usr/bin/env nu

source ./libtest-nix.nu

# Create a file we can export.
mkdir ($env.WORK | path join docs)
"secret data\n" | save -f --raw ($env.WORK | path join docs file1.txt)

let r = (do { ^$env.FLATPAK document-export ($env.WORK | path join docs file1.txt) } | complete)
let out = $r.stdout + $r.stderr
print $"export output: ($out)"
let doc_id = (
    $out | lines
    | where {|l| $l | str starts-with "Exported as: " }
    | each {|l| $l | str replace "Exported as: " "" }
    | get 0? | default ""
)
if ($doc_id | is-empty) {
    print "FAIL: no doc id printed"
    exit 1
}
ok $"document exported with id: ($doc_id)"

# documents listing should now include this doc id.
let rl = (do { ^$env.FLATPAK documents } | complete)
let list = $rl.stdout + $rl.stderr
print $"list: ($list)"
if not ($list | str contains $doc_id) {
    print "FAIL: doc id missing from documents listing"
    print $"Got: ($list)"
    exit 1
}
ok "doc id present in documents listing"

print "PASS: vm-document-export"
