#!/usr/bin/env nu
# Snapshot registry index entries for the exact crate versions pinned in one
# or more Cargo.lock files, fetching from the sparse index
# (https://index.crates.io/) and writing the standard git-index directory
# layout. The result is a committed mini-index that lib/index.nix can read at
# eval time without IFD or a giant flake input. See PLAN.md.
#
# Usage:
#   nu platform/nix/lib/cargo/tools/snapshot-index.nu <out-dir> <Cargo.lock>...

const CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"

# Index directory layout for a crate name (lowercased).
def rel-path [name: string] {
  let n = ($name | str downcase)
  let len = ($n | str length)
  if $len == 1 {
    $"1/($n)"
  } else if $len == 2 {
    $"2/($n)"
  } else if $len == 3 {
    $"3/($n | str substring 0..<1)/($n)"
  } else {
    $"($n | str substring 0..<2)/($n | str substring 2..<4)/($n)"
  }
}

def main [out_dir: string, ...locks: path] {
  if ($locks | is-empty) {
    error make {msg: "no Cargo.lock files given"}
  }

  let pkgs = $locks
  | each {|l|
    open --raw $l | from toml | get -o package | default []
  }
  | flatten
  | where {|p| ($p | get -o source | default "") == $CRATES_IO}
  | select name version
  | uniq

  let by_name = ($pkgs | group-by name | transpose name items | sort-by name)

  for entry in $by_name {
    let name = $entry.name
    let versions = ($entry.items | get version)
    let rel = (rel-path $name)
    let url = $"https://index.crates.io/($rel)"
    let resp = (http get --raw $url)
    let raw = if ($resp | describe) == "binary" {
      $resp | decode utf-8
    } else {
      $resp
    }
    let keep = $raw
    | lines
    | where {|l| ($l | str trim) != ""}
    | where {|l| ($l | from json | get vers) in $versions}
    if ($keep | length) < ($versions | length) {
      let found = ($keep | each {|l| $l | from json | get vers})
      error make {
        msg: $"($name): wanted versions ($versions | str join ', ') but sparse index only has ($found | str join ', ')"
      }
    }
    let dest = ($out_dir | path join $rel)
    mkdir ($dest | path dirname)
    (($keep | str join "\n") + "\n") | save -f $dest
    print $"($name): ($keep | length) version\(s\)"
  }

  print $"snapshot complete: ($by_name | length) crates -> ($out_dir)"
}
