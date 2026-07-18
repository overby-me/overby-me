#!/usr/bin/env nu
# Differential oracle: compare this library's pure-eval resolution against
# `cargo metadata` for a project (see PLAN.md, M7).
#
# Compares the reachable package set and the per-package enabled feature
# sets. Our resolution runs with includeDev = true because cargo metadata's
# resolve graph always includes workspace dev-dependencies.
#
# Needs `cargo` on PATH (e.g. `nix shell nixpkgs#cargo`) and network access
# the first time (cargo downloads manifests of locked crates).
#
# Usage:
#   nu nix/lib/cargo/tools/diff-cargo.nu <project-dir> [--platform x86_64-linux]

const TRIPLES = {
  x86_64-linux: "x86_64-unknown-linux-gnu",
  aarch64-linux: "aarch64-unknown-linux-gnu",
  x86_64-darwin: "x86_64-apple-darwin",
  aarch64-darwin: "aarch64-apple-darwin",
}

# Our pure-eval resolution: record of "name-version" -> [features].
def our-resolution [lib_dir: string, proj: string, root: string, platform: string] {
  let expr = $"
    let l = import (
      $lib_dir | path join "lib" | to json
    ); r = l.resolve.resolve {
      lock = l.lock.parseLock \(builtins.readFile ($proj | path join "Cargo.lock" | to json)\);
      indexDir = ($lib_dir | path join "index" | to json);
      platform = l.cfg.platforms.($platform);
      workspace = l.manifest.loadWorkspace ($proj | to json);
      roots = [($root | to json)];
      includeDev = true;
    }; in builtins.mapAttrs \(_id: n: n.features\) r.nodes"
  ^nix eval --json --impure --expr $expr | from json
}

# cargo's resolution: record of "name-version" -> [features], restricted to
# packages reachable from the root over platform-filtered edges.
def cargo-resolution [proj: string, triple: string] {
  if (which cargo | is-empty) {
    error make {msg: "cargo not found in PATH; try: nix shell nixpkgs#cargo -c nu ..."}
  }
  let meta = (
    ^cargo metadata --format-version 1 --locked
      --filter-platform $triple
      --manifest-path ($proj | path join "Cargo.toml")
    | from json
  )
  let by_id = ($meta.packages | reduce --fold {} {|p, acc|
    $acc | upsert $p.id $"($p.name)-($p.version)"
  })
  let nodes = ($meta.resolve.nodes | reduce --fold {} {|n, acc|
    $acc | upsert $n.id {features: ($n.features | sort), deps: ($n.deps | get pkg)}
  })
  let root = $meta.resolve.root
  if $root == null {
    error make {msg: "virtual workspaces are not supported yet"}
  }

  # Reachability over the filtered graph.
  mut visited = [$root]
  mut queue = [$root]
  while not ($queue | is-empty) {
    let cur = ($queue | first)
    $queue = ($queue | skip 1)
    for dep in ($nodes | get $cur | get deps) {
      if not ($dep in $visited) {
        $visited = ($visited | append $dep)
        $queue = ($queue | append $dep)
      }
    }
  }

  $visited | reduce --fold {} {|id, acc|
    $acc | upsert ($by_id | get $id) ($nodes | get $id | get features)
  }
}

def main [project: path, --platform: string = "x86_64-linux"] {
  let self = (path self)
  let lib_dir = ($self | path dirname | path dirname)
  let proj = ($project | path expand)
  let triple = ($TRIPLES | get $platform)

  let manifest = (open ($proj | path join "Cargo.toml"))
  let root = $manifest.package.name

  let ours = (our-resolution $lib_dir $proj $root $platform)
  let cargos = (cargo-resolution $proj $triple)

  let our_ids = ($ours | columns | sort)
  let cargo_ids = ($cargos | columns | sort)

  let only_ours = ($our_ids | where {|i| not ($i in $cargo_ids)})
  let only_cargo = ($cargo_ids | where {|i| not ($i in $our_ids)})
  let common = ($our_ids | where {|i| $i in $cargo_ids})

  let feature_diffs = ($common | each {|id|
    let a = ($ours | get $id | sort)
    let b = ($cargos | get $id)
    if $a != $b {
      {
        package: $id,
        extra_ours: ($a | where {|f| not ($f in $b)} | str join ","),
        missing_ours: ($b | where {|f| not ($f in $a)} | str join ","),
      }
    } else {
      null
    }
  } | where {|d| $d != null})

  print $"($proj | path basename): ours ($our_ids | length) packages, cargo ($cargo_ids | length) packages, ($common | length) common"
  if not ($only_ours | is-empty) {
    print "only in our resolution:"
    print ($only_ours | each {|i| $"  ($i)"} | str join "\n")
  }
  if not ($only_cargo | is-empty) {
    print "only in cargo's resolution:"
    print ($only_cargo | each {|i| $"  ($i)"} | str join "\n")
  }
  if not ($feature_diffs | is-empty) {
    print "feature differences:"
    print ($feature_diffs | table)
  }

  if ($only_ours | is-empty) and ($only_cargo | is-empty) and ($feature_diffs | is-empty) {
    print "identical: graph and features match cargo"
  } else {
    exit 1
  }
}
