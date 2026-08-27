#!/usr/bin/env nu
# Compiles one crate, lib and/or bins, with rustc directly and no cargo,
# implementing the cargo build-script protocol: build.rs is compiled with host
# deps, run with the documented environment, its cargo: directives parsed and
# applied to the library compile, and links metadata plus native link flags
# persisted for dependents under $out/nix-support/.
#
# The config is JSON, its path in argv; see buildCrate.nix. Runs inside the
# sandbox via `nu --no-config-file`.

def snake [s: string] {
  $s | str replace -a "-" "_"
}

# `str upcase`, not its 0.114 rename `str uppercase`: this runs under
# `pkgs.nushell`, which nixos-26.05 pins to 0.112.
def upperize [s: string] {
  $s | str upcase | str replace -a "-" "_"
}

# Build-time manifest planning for registry crates (published, normalized
# manifests; workspace members get an eval-provided plan instead). Reading
# the manifest here, inside the sandbox, is what keeps evaluation IFD-free.
def plan-from-manifest [root: string] {
  let m = (open ($root | path join "Cargo.toml"))
  let pkg = $m.package
  let libt = ($m | get -o lib)
  let auto_lib = (($root | path join "src/lib.rs") | path exists)
  let lib_plan = if $libt == null and (not $auto_lib) {
    null
  } else {
    let l = ($libt | default {})
    let proc = ($l | get -o proc-macro | default ($l | get -o proc_macro) | default false)
    let ct = (
      $l | get -o crate-type
      | default ($l | get -o crate_type)
      | default (if $proc { ["proc-macro"] } else { ["lib"] })
    )
    {
      name: ($l | get -o name | default (snake $pkg.name)),
      path: ($l | get -o path | default "src/lib.rs"),
      procMacro: $proc,
      # Specified as a list, seen as a bare string in the wild.
      crateTypes: (if ($ct | describe) == "string" { [$ct] } else { $ct }),
    }
  }
  let build_field = ($pkg | get -o build)
  let build = if $build_field == null {
    if (($root | path join "build.rs") | path exists) { "build.rs" } else { null }
  } else if $build_field == false {
    null
  } else {
    $build_field
  }
  let authors0 = ($pkg | get -o authors | default [])
  {
    name: $pkg.name,
    version: ($pkg.version | into string),
    edition: ($pkg | get -o edition | default "2015" | into string),
    links: ($pkg | get -o links),
    description: ($pkg | get -o description | default ""),
    license: ($pkg | get -o license | default ""),
    repository: ($pkg | get -o repository | default ""),
    authors: (if ($authors0 | describe) == "string" { [$authors0] } else { $authors0 }),
    rustVersion: ($pkg | get -o rust-version | default "" | into string),
    lib: $lib_plan,
    build: $build,
    bins: [],
  }
}

# The cargo package env contract, shared by build scripts and rustc calls.
def pkg-env [plan: record, features: list<string>] {
  let version = $plan.version
  let core = ($version | split row "+" | first)
  let dash_parts = ($core | split row "-")
  let nums = ($dash_parts | first | split row ".")
  let pre = ($dash_parts | skip 1 | str join "-")
  mut e = {
    CARGO_MANIFEST_DIR: $env.PWD,
    CARGO_MANIFEST_PATH: ($env.PWD | path join "Cargo.toml"),
    CARGO_PKG_NAME: $plan.name,
    CARGO_PKG_VERSION: $version,
    CARGO_PKG_VERSION_MAJOR: ($nums | get 0),
    CARGO_PKG_VERSION_MINOR: ($nums | get -o 1 | default "0"),
    CARGO_PKG_VERSION_PATCH: ($nums | get -o 2 | default "0"),
    CARGO_PKG_VERSION_PRE: $pre,
    CARGO_PKG_AUTHORS: ($plan.authors | str join ":"),
    CARGO_PKG_DESCRIPTION: ($plan.description | default ""),
    CARGO_PKG_HOMEPAGE: "",
    CARGO_PKG_REPOSITORY: ($plan.repository | default ""),
    CARGO_PKG_LICENSE: ($plan.license | default ""),
    CARGO_PKG_LICENSE_FILE: "",
    CARGO_PKG_README: "",
    CARGO_PKG_RUST_VERSION: ($plan.rustVersion | default ""),
  }
  for f in $features {
    $e = ($e | upsert $"CARGO_FEATURE_(upperize $f)" "1")
  }
  if ($plan | get -o links) != null {
    $e = ($e | upsert CARGO_MANIFEST_LINKS $plan.links)
  }
  $e
}

# CARGO_CFG_* for build scripts, from the compiler's own cfg set: the shared
# per-toolchain cfg file when provided, otherwise rustc itself.
def rustc-cfg-env [rustc: string, cfg_file: any] {
  (
    if $cfg_file != null and ($cfg_file | path exists) {
      open --raw $cfg_file
    } else {
      ^$rustc --print cfg
    }
  ) | lines | reduce --fold {} {|line, acc|
    if ($line | str contains "=") {
      let kv = ($line | split row -n 2 "=")
      let key = $"CARGO_CFG_(upperize $kv.0)"
      let val = ($kv | get 1 | str trim -c '"')
      let prev = ($acc | get -o $key)
      $acc | upsert $key (if $prev == null or $prev == "" { $val } else { $"($prev),($val)" })
    } else if $line != "" {
      let key = $"CARGO_CFG_(upperize $line)"
      if ($acc | get -o $key) == null { $acc | upsert $key "" } else { $acc }
    } else {
      $acc
    }
  }
}

def extern-args [externs: list] {
  $externs | each {|e|
    let marker = ($e.out | path join "nix-support/extern")
    if not ($marker | path exists) {
      error make {msg: $"dependency ($e.name) \(($e.out)\) has no lib artifact"}
    }
    # Cargo's extern name is the dependency's lib target name (e.g. crate
    # md-5 exposes lib "md5"), unless the manifest renames the dependency.
    let libname_marker = ($e.out | path join "nix-support/libname")
    let name = (
      if ($e | get -o renamed | default false) or (not ($libname_marker | path exists)) {
        snake $e.name
      } else {
        open --raw $libname_marker | str trim
      }
    )
    ["--extern", $"($name)=(open --raw $marker | str trim)"]
  } | flatten
}

def dep-dir-args [dep_outs: list<string>] {
  $dep_outs | each {|o| ["-L", $"dependency=($o)/lib"]} | flatten
}

def feature-cfg-args [features: list<string>] {
  $features | each {|f| ["--cfg", $"feature=\"($f)\""]} | flatten
}

# Parse `cargo:`/`cargo::` directives from a build script's stdout into
# { cfgs, envs, link_libs, link_search, metadata }.
def parse-directives [stdout: string] {
  mut out = {cfgs: [], envs: [], link_libs: [], link_search: [], metadata: []}
  for line in ($stdout | lines) {
    let d = if ($line | str starts-with "cargo::") {
      {content: ($line | str substring 7..), modern: true}
    } else if ($line | str starts-with "cargo:") {
      {content: ($line | str substring 6..), modern: false}
    } else {
      null
    }
    if $d == null { continue }
    let kv = ($d.content | split row -n 2 "=")
    let key = ($kv | get 0)
    let val = ($kv | get -o 1 | default "")
    match $key {
      "rustc-cfg" => { $out.cfgs = ($out.cfgs | append $val) }
      "rustc-env" => {
        let p = ($val | split row -n 2 "=")
        $out.envs = ($out.envs | append {name: ($p | get 0), value: ($p | get -o 1 | default "")})
      }
      "rustc-link-lib" => { $out.link_libs = ($out.link_libs | append $val) }
      "rustc-link-search" => { $out.link_search = ($out.link_search | append $val) }
      "rustc-flags" => {
        let toks = ($val | split row " " | where {|t| $t != ""})
        mut i = 0
        while $i < ($toks | length) {
          let t = ($toks | get $i)
          if $t in ["-l", "-L"] and ($i + 1) < ($toks | length) {
            let v = ($toks | get ($i + 1))
            if $t == "-l" {
              $out.link_libs = ($out.link_libs | append $v)
            } else {
              $out.link_search = ($out.link_search | append $v)
            }
            $i = $i + 2
          } else if ($t | str starts-with "-l") or ($t | str starts-with "-L") {
            let v = ($t | str substring 2..)
            if ($t | str starts-with "-l") {
              $out.link_libs = ($out.link_libs | append $v)
            } else {
              $out.link_search = ($out.link_search | append $v)
            }
            $i = $i + 1
          } else {
            error make {msg: $"unsupported rustc-flags token: ($t)"}
          }
        }
      }
      "metadata" => {
        let p = ($val | split row -n 2 "=")
        $out.metadata = ($out.metadata | append {name: ($p | get 0), value: ($p | get -o 1 | default "")})
      }
      "warning" => { print -e $"build script warning: ($val)" }
      "error" => { error make {msg: $"build script error: ($val)"} }
      _ => {
        if ($key | str starts-with "rerun-if") or $key in ["rustc-check-cfg", "rustc-cdylib-link-arg"] {
          # Irrelevant in a sandboxed one-shot build.
        } else if not $d.modern {
          # Legacy links metadata: cargo:KEY=VALUE
          $out.metadata = ($out.metadata | append {name: $key, value: $val})
        } else {
          error make {msg: $"unknown cargo:: directive: ($line)"}
        }
      }
    }
  }
  $out
}

# DEP_<links>_<key> env vars from direct deps' persisted links metadata.
def dep-links-env [links_deps: list<string>] {
  $links_deps | reduce --fold {} {|dep, acc|
    let f = ($dep | path join "nix-support/cargo-links")
    if not ($f | path exists) {
      $acc
    } else {
      let contents = (open --raw $f | lines)
      let links = ($contents | first)
      $contents | skip 1 | reduce --fold $acc {|kv, a|
        let p = ($kv | split row -n 2 "=")
        if ($p | get 0) == "" {
          $a
        } else {
          $a | upsert $"DEP_(upperize $links)_(upperize ($p | get 0))" ($p | get -o 1 | default "")
        }
      }
    }
  }
}

# Compile and run build.rs; returns { directives, out_dir }.
def run-build-script [cfg: record, plan: record, rustc: string, base_env: record] {
  let bs_dir = ($env.PWD | path join ".build-script")
  let out_dir = ($env.PWD | path join ".build-out")
  mkdir $bs_dir $out_dir

  let compile_args = (
    [
      $plan.build,
      "--crate-name", "build_script_build",
      "--crate-type", "bin",
      "--edition", $plan.edition,
      "-C", "opt-level=0",
      "--out-dir", $bs_dir,
    ]
    | append ($cfg | get -o rustcFlags | default [])
    | append (if $cfg.capLints { ["--cap-lints", "allow"] } else { [] })
    | append (feature-cfg-args $cfg.features)
    | append (extern-args $cfg.buildExterns)
    | append (dep-dir-args $cfg.buildDepOuts)
  )
  with-env $base_env { ^$rustc ...$compile_args }

  let script_env = (
    $base_env
    | merge (rustc-cfg-env $rustc ($cfg | get -o rustcCfgFile))
    | merge (dep-links-env $cfg.linksDeps)
    | merge {
      OUT_DIR: $out_dir,
      TARGET: $cfg.target,
      HOST: ($cfg | get -o hostTriple | default $cfg.target),
      NUM_JOBS: (sys cpu | length | into string),
      OPT_LEVEL: $cfg.profile.optLevel,
      PROFILE: (if $cfg.profile.debugInfo == "0" { "release" } else { "debug" }),
      DEBUG: (if $cfg.profile.debugInfo == "0" { "false" } else { "true" }),
      RUSTC: $rustc,
      RUSTDOC: "rustdoc",
      CARGO: (which cargo | get -o 0.path | default "cargo"),
      CARGO_ENCODED_RUSTFLAGS: "",
    }
  )
  let res = (with-env $script_env { ^($bs_dir | path join "build_script_build") } | complete)
  if $res.stderr != "" {
    print -e $res.stderr
  }
  if $res.exit_code != 0 {
    print -e $res.stdout
    error make {msg: $"build script failed \(($res.exit_code)\)"}
  }
  {directives: (parse-directives $res.stdout), out_dir: $out_dir}
}

# Rewrite build-sandbox OUT_DIR paths to their persisted $out/out location.
# Link-search values may carry a kind prefix ("native=/path", "all=/path",
# ...) which must be preserved while the path part is rewritten.
def rewrite-out-dir [p: string, out_dir: any, out: string] {
  if $out_dir == null {
    $p
  } else {
    let parts = ($p | split row -n 2 "=")
    let has_kind = (($parts | length) == 2 and not ($parts | get 0 | str contains "/"))
    let kind = (if $has_kind { ($parts | get 0) + "=" } else { "" })
    let path = (if $has_kind { $parts | get 1 } else { $p })
    if ($path | str starts-with $out_dir) {
      $"($kind)($out)/out($path | str substring ($out_dir | str length)..)"
    } else {
      $p
    }
  }
}

# Native -L/-l flags from this crate's own build script.
def native-link-args [bs: any] {
  if $bs == null {
    []
  } else {
    (
      ($bs.link_search | each {|s| ["-L", $s]})
      | append ($bs.link_libs | each {|l| ["-l", $l]})
      | flatten
    )
  }
}

# Native link flags recorded by transitive deps' build scripts (search paths
# are needed at the final bin link; rustc does not embed them in rlibs).
def collect-transitive-native [dep_outs: list<string>] {
  $dep_outs | each {|o|
    let f = ($o | path join "nix-support/rustc-link")
    if ($f | path exists) {
      open --raw $f | lines | where {|l| $l != ""} | each {|l|
        let p = ($l | split row -n 2 " ")
        [($p | get 0), ($p | get -o 1 | default "")]
      } | flatten
    } else {
      []
    }
  } | flatten
}

# Flags shared by lib and bin compiles. panic=abort is skipped for
# proc-macro crates and build scripts, matching cargo.
def profile-args [profile: record, is_proc_macro: bool] {
  ["-C", $"opt-level=($profile.optLevel)"]
  | append (if $profile.debugInfo != "0" { ["-C", $"debuginfo=($profile.debugInfo)"] } else { [] })
  | append (
    if ($profile | get -o codegenUnits) != null {
      ["-C", $"codegen-units=($profile.codegenUnits)"]
    } else { [] }
  )
  | append (
    if $profile.panic == "abort" and (not $is_proc_macro) {
      ["-C", "panic=abort"]
    } else { [] }
  )
}

# Extra flags for library compiles when LTO is enabled downstream.
def lto-lib-args [profile: record, is_proc_macro: bool] {
  if $profile.lto != "off" and (not $is_proc_macro) {
    ["-C", "embed-bitcode=yes"]
  } else { [] }
}

# Extra flags for the final bin links.
def lto-bin-args [profile: record] {
  (
    if $profile.lto != "off" { ["-C", $"lto=($profile.lto)"] } else { [] }
  )
  | append (
    if $profile.strip != "none" { ["-C", $"strip=($profile.strip)"] } else { [] }
  )
}

# Compile the lib target; returns the artifact path recorded for dependents.
def compile-lib [plan: record, rustc: string, base_env: record, common_args: list, extra_args: list, bs: any, crate_hash: string, out: string, metadata_only: bool, cross: string] {
  let lib = $plan.lib
  let types = ($lib.crateTypes | where {|t| $t in ["lib", "rlib", "proc-macro"]})
  let crate_types = (if ($types | is-empty) { ["lib"] } else { $types })
  mkdir ($out | path join "lib")
  # rustc does not link the compiler's proc_macro crate on its own; cargo
  # passes --extern proc_macro (sysroot, no path) for proc-macro compiles.
  let proc_macro_extern = (
    if "proc-macro" in $crate_types { ["--extern", "proc_macro"] } else { [] }
  )
  let args = (
    [
      $lib.path,
      "--crate-name", $lib.name,
      "--crate-type", ($crate_types | str join ","),
      "--edition", $plan.edition,
      "-C", $"metadata=($crate_hash)",
      "-C", $"extra-filename=-($crate_hash)",
      "--out-dir", ($out | path join "lib"),
    ]
    | append (
      # Cross-compilation: proc-macros must stay host binaries.
      if $cross != "" and (not ($lib | get -o procMacro | default false)) {
        ["--target", $cross]
      } else { [] }
    )
    | append (if $metadata_only { ["--emit=metadata"] } else { [] })
    | append $proc_macro_extern
    | append $common_args
    | append $extra_args
    | append (native-link-args $bs)
  )
  with-env ($base_env | merge {CARGO_CRATE_NAME: $lib.name}) { ^$rustc ...$args }
  let produced = (ls ($out | path join "lib") | get name | sort)
  let rmetas = ($produced | where {|f| $f | str ends-with ".rmeta"})
  let rlibs = ($produced | where {|f| $f | str ends-with ".rlib"})
  let dylibs = ($produced | where {|f| ($f | str ends-with ".so") or ($f | str ends-with ".dylib")})
  let artifact = (
    if $metadata_only { $rmetas | first } else { $rlibs | append $dylibs | first }
  )
  $artifact | save -f ($out | path join "nix-support/extern")
  $lib.name | save -f ($out | path join "nix-support/libname")
  $artifact
}

def compile-bins [cfg: record, plan: record, rustc: string, base_env: record, common_args: list, extra_args: list, bs: any, lib_artifact: any, out: string] {
  let bins = ($cfg.bins | default ($plan | get -o bins) | default [])
  mkdir ($out | path join "bin")
  let link_dep_outs = ($cfg | get -o linkDepOuts | default $cfg.depOuts)
  let bin_common = (
    (profile-args $cfg.profile false)
    | append ($cfg | get -o linkArgs | default [])
    | append ($cfg | get -o rustcFlags | default [])
    | append (if $cfg.capLints { ["--cap-lints", "allow"] } else { [] })
    | append (feature-cfg-args $cfg.features)
    | append (if $bs == null { [] } else { $bs.cfgs | each {|c| ["--cfg", $c]} | flatten })
    | append (extern-args ($cfg | get -o linkExterns | default $cfg.externs))
    | append (dep-dir-args $link_dep_outs)
    | append $extra_args
    | append (
      if $lib_artifact != null {
        ["--extern", $"($plan.lib.name)=($lib_artifact)"]
      } else {
        []
      }
    )
    | append (native-link-args $bs)
    | append (collect-transitive-native $link_dep_outs)
  )
  let cross = ($cfg | get -o crossTarget | default "")
  for b in $bins {
    let crate_name = (snake $b.name)
    let args = (
      [
        $b.path,
        "--crate-name", $crate_name,
        "--crate-type", "bin",
        "--edition", $plan.edition,
        "-o", ($out | path join "bin" $b.name),
      ]
      | append (if $cross != "" { ["--target", $cross] } else { [] })
      | append $bin_common
    )
    with-env ($base_env | merge {CARGO_CRATE_NAME: $crate_name}) { ^$rustc ...$args }
  }
}

# Persist build script results for dependents: OUT_DIR contents (when
# referenced), links metadata, and native link flags.
def persist-build-script [plan: record, bs: any, bs_out_dir: any, out: string] {
  let search = ($bs.link_search | each {|s| rewrite-out-dir $s $bs_out_dir $out})
  let out_prefix = $"($out)/out"
  let needs_out = (
    ($search | any {|s| $s | str starts-with $out_prefix})
    or ($bs.metadata | any {|m| $m.value | str starts-with $bs_out_dir})
  )
  if $needs_out or ((ls $bs_out_dir | length) > 0) {
    cp -r $bs_out_dir ($out | path join "out")
  }
  if ($plan | get -o links) != null {
    let lines = (
      [$plan.links]
      | append ($bs.metadata | each {|m| $"($m.name)=(rewrite-out-dir $m.value $bs_out_dir $out)"})
    )
    ($lines | str join "\n") + "\n" | save -f ($out | path join "nix-support/cargo-links")
  }
  let link_lines = (
    ($search | each {|s| $"-L ($s)"})
    | append ($bs.link_libs | each {|l| $"-l ($l)"})
  )
  if not ($link_lines | is-empty) {
    ($link_lines | str join "\n") + "\n" | save -f ($out | path join "nix-support/rustc-link")
  }
}

# Compile and run the crate's test targets. Unit tests come from the lib and
# each bin source compiled with --test; integration tests from tests/*.rs
# compiled with --test and linked against the crate's own lib. Each harness
# binary is executed and a nonzero exit fails the build. cfg.externs already
# carries normal + dev dependencies (the eval side resolves with dev deps).
def run-tests [cfg: record, plan: record, rustc: string, base_env: record, bs: any, lib_artifact: any, out: string] {
  let link_dep_outs = ($cfg | get -o linkDepOuts | default $cfg.depOuts)
  let cross = ($cfg | get -o crossTarget | default "")
  let link_common = (
    (profile-args $cfg.profile false)
    | append ($cfg | get -o linkArgs | default [])
    | append ($cfg | get -o rustcFlags | default [])
    | append (if $cfg.capLints { ["--cap-lints", "allow"] } else { [] })
    | append (feature-cfg-args $cfg.features)
    | append (if $bs == null { [] } else { $bs.cfgs | each {|c| ["--cfg", $c]} | flatten })
    | append (extern-args $cfg.externs)
    | append (dep-dir-args $link_dep_outs)
    | append (native-link-args $bs)
    | append (collect-transitive-native $link_dep_outs)
    | append (if $cross != "" { ["--target", $cross] } else { [] })
  )
  let test_dir = ($env.PWD | path join ".test-bins")
  mkdir $test_dir

  # The lib target IS the crate, so it takes no self-extern (own = false). Bins
  # in a lib+bin crate reference the crate's lib, so they link it the way an
  # integration test does (own = true, a no-op when the crate has no lib), as do
  # the tests/*.rs integration targets.
  let unit_targets = (
    (if ($plan | get -o lib) != null { [{name: $plan.lib.name, path: $plan.lib.path, own: false}] } else { [] })
    | append (($plan | get -o bins | default []) | each {|b| {name: (snake $b.name), path: $b.path, own: true}})
  )
  let tests_dir = ($env.PWD | path join "tests")
  let integration_targets = (
    if ($tests_dir | path exists) {
      ls $tests_dir
      | where {|f| ($f.name | str ends-with ".rs")}
      | get name
      | each {|p| {name: $"it_(($p | path basename) | str replace '.rs' '')", path: $p, own: true}}
    } else {
      []
    }
  )
  let targets = ($unit_targets | append $integration_targets)

  mut ran = 0
  for t in $targets {
    let cname = (snake $t.name)
    let bin = ($test_dir | path join $cname)
    let own_lib = (
      if $t.own and $lib_artifact != null {
        ["--extern", $"($plan.lib.name)=($lib_artifact)"]
      } else {
        []
      }
    )
    let args = (
      [$t.path, "--crate-name", $cname, "--edition", $plan.edition, "--test", "-o", $bin]
      | append $own_lib
      | append $link_common
    )
    with-env ($base_env | merge {CARGO_CRATE_NAME: $cname}) { ^$rustc ...$args }
    let res = (with-env $base_env { ^$bin } | complete)
    print $res.stdout
    if $res.stderr != "" { print -e $res.stderr }
    if $res.exit_code != 0 {
      error make {msg: $"test target ($t.name) failed \(exit ($res.exit_code)\)"}
    }
    $ran = $ran + 1
  }
  ($"ran ($ran) test target\(s\)\n") | save -f ($out | path join "test-result")
}

def main [config_path: string] {
  let cfg = (open $config_path)
  let out = $env.out
  let rustc_row = (which rustc | get -o 0)
  if $rustc_row == null {
    error make {msg: "rustc not found in PATH"}
  }
  let rustc = $rustc_row.path
  let plan = (if $cfg.plan == null { plan-from-manifest "." } else { $cfg.plan })

  # Pipelining eligibility is decided here, in the sandbox, because evaluation
  # cannot know a registry crate's targets. An ineligible crate's rmeta
  # derivation falls back to a full build, and the full derivation then only
  # copies its artifacts.
  let want_meta = ($cfg | get -o emitMetadataOnly | default false)
  let eligible = (
    ($plan | get -o lib) != null
    and (not ($plan.lib | get -o procMacro | default false))
    and ($plan | get -o build) == null
  )
  let fallback = ($cfg | get -o fallbackFrom)
  if (not $want_meta) and $fallback != null {
    let mode_file = ($fallback | path join "nix-support/pipeline-mode")
    if ($mode_file | path exists) and (open --raw $mode_file | str trim) == "full" {
      # The rmeta derivation already did the full build.
      mkdir $out
      cp -r ($fallback | path join "lib") ($out | path join "lib")
      cp -r ($fallback | path join "nix-support") ($out | path join "nix-support")
      if (($fallback | path join "out") | path exists) {
        cp -r ($fallback | path join "out") ($out | path join "out")
      }
      # Store copies arrive read-only; the marker rewrites below need +w.
      ^chmod -R u+w $out
      # Rewrite the extern marker to this derivation's copy.
      let old_extern = (open --raw ($out | path join "nix-support/extern") | str trim)
      ($old_extern | str replace $fallback $out) | save -f ($out | path join "nix-support/extern")
      let links = ($out | path join "nix-support/cargo-links")
      if ($links | path exists) {
        (open --raw $links | str replace -a $fallback $out) | save -f $links
      }
      let rl = ($out | path join "nix-support/rustc-link")
      if ($rl | path exists) {
        (open --raw $rl | str replace -a $fallback $out) | save -f $rl
      }
      return
    }
  }
  let metadata_only = ($want_meta and $eligible)
  let pkg_env = (pkg-env $plan $cfg.features)

  mkdir ($out | path join "nix-support")

  # One expression rather than null-initialised muts: nushell 0.115 types a
  # mut from its initialiser, so assigning a record over null is an error.
  let bsr = if ($plan | get -o build) != null {
    let r = (run-build-script $cfg $plan $rustc $pkg_env)
    let merged = (
      $r.directives.envs
      | reduce -f ($pkg_env | upsert OUT_DIR $r.out_dir) {|e, acc| $acc | upsert $e.name $e.value }
    )
    {bs: $r.directives, out_dir: $r.out_dir, env: $merged}
  } else {
    {bs: null, out_dir: null, env: $pkg_env}
  }
  let bs = $bsr.bs
  let bs_out_dir = $bsr.out_dir
  let base_env = $bsr.env

  let is_proc_macro = (
    ($plan | get -o lib) != null and ($plan.lib | get -o procMacro | default false)
  )
  let common_args = (
    (profile-args $cfg.profile $is_proc_macro)
    | append ($cfg | get -o linkArgs | default [])
    | append ($cfg | get -o rustcFlags | default [])
    | append (if $cfg.capLints { ["--cap-lints", "allow"] } else { [] })
    | append (feature-cfg-args $cfg.features)
    | append (if $bs == null { [] } else { $bs.cfgs | each {|c| ["--cfg", $c]} | flatten })
    | append (extern-args $cfg.externs)
    | append (dep-dir-args $cfg.depOuts)
  )

  let lib_artifact = (
    if ($plan | get -o lib) != null {
      # The metadata pass must be flag-identical to the full pass (same
      # SVH), differing only in --emit.
      compile-lib $plan $rustc $base_env $common_args (lto-lib-args $cfg.profile $is_proc_macro) $bs $cfg.crateHash $out $metadata_only ($cfg | get -o crossTarget | default "")
    } else {
      null
    }
  )
  if ($cfg | get -o testMode | default false) {
    if $bs != null and $bs_out_dir != null {
      persist-build-script $plan $bs $bs_out_dir $out
    }
    run-tests $cfg $plan $rustc $base_env $bs $lib_artifact $out
    return
  }

  if $want_meta {
    (if $metadata_only { "metadata" } else { "full" })
    | save -f ($out | path join "nix-support/pipeline-mode")
  }

  if $cfg.buildBins {
    compile-bins $cfg $plan $rustc $base_env $common_args (lto-bin-args $cfg.profile) $bs $lib_artifact $out
  }

  if $bs != null and $bs_out_dir != null {
    persist-build-script $plan $bs $bs_out_dir $out
  }
}
