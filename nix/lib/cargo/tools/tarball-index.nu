#!/usr/bin/env nu
# Generate a registry mini-index for the crates.io packages in a Cargo.lock
# WITHOUT the network: read each crate's published Cargo.toml out of its
# already-fetched .crate tarball and emit the index line cargo published.
#
# Inputs arrive as a JSON manifest of [{name, version, cksum, tar}] records,
# where `tar` is a fixed-output .crate fetch (hash from the lock). Because the
# tarballs are content-verified store paths, this whole step is pure and
# cacheable; lib/index.nix reads the result at eval time (the one IFD).
#
# The published Cargo.toml is exactly what crates.io turns into an index
# entry, so the conversion is a straight field mapping: dependency tables to
# {name, req, features, optional, default_features, target, kind}, the
# [features] table verbatim (dep:/?/ syntax merges with features2 downstream),
# links and the checksum from the lock. Renamed deps put the rename in `name`
# and the real crate in `package`, matching the registry schema.
#
# Usage:
#   nu tarball-index.nu <out-dir> <manifest.json>

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

# One [dependencies]-style table (string or inline-table values) to a list of
# index dependency records tagged with the given kind and target cfg.
def mk-deps [tbl: any, kind: string, target: any] {
  if $tbl == null {
    []
  } else {
    $tbl | columns | each {|key|
      let v = ($tbl | get $key)
      if ($v | describe | str starts-with "record") {
        let base = {
          name: $key
          req: ($v.version? | default "*")
          features: ($v.features? | default [])
          optional: ($v.optional? | default false)
          default_features: ($v."default-features"? | default true)
          target: $target
          kind: $kind
        }
        let pkg = ($v.package?)
        if $pkg != null { $base | insert package $pkg } else { $base }
      } else {
        {
          name: $key
          req: $v
          features: []
          optional: false
          default_features: true
          target: $target
          kind: $kind
        }
      }
    }
  }
}

# All dependency kinds for one manifest scope (root or one target section).
def deps-from [scope: any, target: any] {
  (mk-deps ($scope.dependencies?) "normal" $target)
  | append (mk-deps ($scope."build-dependencies"?) "build" $target)
  | append (mk-deps ($scope."dev-dependencies"?) "dev" $target)
}

def main [out_dir: string, manifest: path] {
  let items = (open --raw $manifest | from json)

  let entries = $items | each {|it|
    let toml = (^tar -xzOf $it.tar $"($it.name)-($it.version)/Cargo.toml" | from toml)
    let pkg = $toml.package

    let plain = (deps-from $toml null)
    let targeted = if ($toml.target?) == null {
      []
    } else {
      $toml.target | columns | each {|t| deps-from ($toml.target | get $t) $t } | flatten
    }
    let deps = ($plain | append $targeted)

    let entry = {
      name: $it.name
      vers: $it.version
      deps: $deps
      cksum: $it.cksum
      features: ($toml.features? | default {})
      yanked: false
    }
    let entry = if ($pkg.links?) != null { $entry | insert links $pkg.links } else { $entry }

    { name: $it.name, vers: $it.version, line: ($entry | to json --raw) }
  }

  $entries | group-by name | items {|name, rows|
    let rel = (rel-path $name)
    let dest = ($out_dir | path join $rel)
    mkdir ($dest | path dirname)
    (($rows | sort-by vers | get line | str join "\n") + "\n") | save -f $dest
  }

  print $"generated ($entries | length) crate version\(s\) -> ($out_dir)"
}
