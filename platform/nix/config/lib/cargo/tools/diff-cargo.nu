#!/usr/bin/env nu
# Differential oracle: compare this library's pure-eval resolution against
# cargo for a project (see PLAN.md, M7).
#
# Compares the built package set and the per-package enabled feature sets
# against `cargo tree`, which reflects the feature-pruned build graph.
# (cargo metadata is unsuitable as the oracle: its resolve graph includes
# optional deps that no feature activates, and its feature sets
# over-approximate the resolver-v2 result.) Our resolution runs with
# includeDev = true because workspace dev-dependencies are part of the
# tree. Packages appearing multiple times with different feature sets
# (resolver v2 host/target splits) are unioned, matching our stage-1
# unified model; the per-unit comparison is the M8 follow-up.
#
# Note: strong "R/F" entries enable the feature R itself when R is an
# implicit-or-explicit feature (old-style optional deps); dep:-referenced
# deps have no such feature. The resolver implements this, so feature sets
# compare directly against cargo tree with no normalization.
#
# Needs `cargo` on PATH (e.g. `nix shell nixpkgs#cargo`) and network access
# the first time (cargo downloads manifests of locked crates).
#
# Usage:
#   nu platform/nix/config/lib/cargo/tools/diff-cargo.nu <project-dir> [--platform x86_64-linux]
#   nu platform/nix/config/lib/cargo/tools/diff-cargo.nu sweep rust/*/

const TRIPLES = {
  x86_64-linux: "x86_64-unknown-linux-gnu",
  aarch64-linux: "aarch64-unknown-linux-gnu",
  x86_64-darwin: "x86_64-apple-darwin",
  aarch64-darwin: "aarch64-apple-darwin",
}

const SELF = (path self)

# Our pure-eval resolution: record of "name-version" -> [features]. The
# workspace loads with src = the project's parent so that path dependencies
# on sibling projects stay inside the source root; all members are roots.
def our-resolution [lib_dir: string, proj: string, platform: string] {
  let parent = ($proj | path dirname)
  let mdir = ($proj | path basename)
  let expr = $"
    let l = import (
      $lib_dir | path join "lib" | to json
    ); ws = l.manifest.loadWorkspace {
      src = ($parent | to json);
      manifestDir = ($mdir | to json);
    }; r = l.resolve.resolve {
      lock = l.lock.parseLock \(builtins.readFile ($proj | path join "Cargo.lock" | to json)\);
      indexDir = ($lib_dir | path join "index" | to json);
      platform = l.cfg.platforms.($platform);
      workspace = ws;
      roots = map \(m: m.name\) ws.members;
      includeDev = true;
    }; in builtins.mapAttrs \(_id: n: n.features\) r.nodes"
  ^nix eval --json --impure --expr $expr | from json
}

# cargo's resolution via cargo tree: record of "name-version" -> [features].
# Lines look like "name v1.2.3 (/path)|feat1,feat2"; duplicate packages
# (host/target units) get their feature sets unioned.
def cargo-resolution [proj: string, triple: string] {
  if (which cargo | is-empty) {
    error make {msg: "cargo not found in PATH; try: nix shell nixpkgs#cargo -c nu ..."}
  }
  let lines = (
    ^cargo tree --locked --quiet
      --manifest-path ($proj | path join "Cargo.toml")
      --target $triple
      --edges normal,build,dev
      --prefix none
      --format "{p}|{f}"
    | lines
    | each {|l| $l | str replace --regex ' \(\*\)$' ""}
    | where {|l| ($l | str trim) != ""}
    | uniq
  )
  $lines | reduce --fold {} {|line, acc|
    let parts = ($line | split row -n 2 "|")
    let m = ($parts | get 0 | parse --regex '^(?P<name>\S+) v(?P<version>\S+)')
    if ($m | is-empty) {
      error make {msg: $"cargo tree: unparsable line: ($line)"}
    }
    let id = $"($m.0.name)-($m.0.version)"
    let feats = (
      $parts | get -o 1 | default ""
      | split row "," | where {|f| $f != ""}
    )
    let merged = ($acc | get -o $id | default [] | append $feats | uniq | sort)
    $acc | upsert $id $merged
  }
}

# One project: prints the comparison, returns true when identical.
def diff-project [project: path, platform: string] {
  let lib_dir = ($SELF | path dirname | path dirname)
  let proj = ($project | path expand)
  let triple = ($TRIPLES | get $platform)

  let ours = (our-resolution $lib_dir $proj $platform)
  let cargos = (cargo-resolution $proj $triple)

  let our_ids = ($ours | columns | sort)
  let cargo_ids = ($cargos | columns | sort)

  let only_ours = ($our_ids | where {|i| not ($i in $cargo_ids)})
  let only_cargo = ($cargo_ids | where {|i| not ($i in $our_ids)})
  let common = ($our_ids | where {|i| $i in $cargo_ids})

  let feature_diffs = ($common | each {|id|
    let a = ($ours | get $id)
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

  let ok = (($only_ours | is-empty) and ($only_cargo | is-empty) and ($feature_diffs | is-empty))
  if $ok {
    print "identical: graph and features match cargo"
  }
  $ok
}

# Sweep several projects (dirs without a Cargo.toml are skipped); report a
# summary table and fail if any diverge.
# Usage: nu tools/diff-cargo.nu sweep rust/*/
def "main sweep" [...projects: path, --platform: string = "x86_64-linux"] {
  let cargo_projects = ($projects | where {|p| ($p | path join "Cargo.toml") | path exists})
  let results = ($cargo_projects | each {|p|
    let r = (try {
      if (diff-project $p $platform) { "identical" } else { "DIVERGES" }
    } catch {|e|
      $"ERROR: ($e.msg)"
    })
    {project: ($p | path basename), result: $r}
  })
  print ($results | table)
  let bad = ($results | where {|r| $r.result != "identical"})
  if not ($bad | is-empty) {
    print $"($bad | length) of ($results | length) projects diverge or fail"
    exit 1
  }
  print $"all ($results | length) projects identical to cargo"
}

def main [project: path, --platform: string = "x86_64-linux"] {
  if not (diff-project $project $platform) {
    exit 1
  }
}
