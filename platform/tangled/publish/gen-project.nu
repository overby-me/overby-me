#!/usr/bin/env nu

# Generate a flake and CI workflow for every published project.
#
# A published repo is a filtered copy of one directory, so it cannot use the
# monorepo's default.nix: that is a flakelight module the root flake imports,
# and it reaches for ../../../platform/nix/lib/lib. Without something of its own, a clone is
# cargo-only and has no CI.
#
# One template covers every project because the flake reads Cargo.toml itself
# with builtins.fromTOML, so nothing needs to be restated per project.
#
# These files are written under generated/ rather than into the project's own
# directory, and josh maps them into place when it publishes (see
# derive-filter in publish.nu). They were inert where they used to sit -
# nothing imports the flake, and Tangled reads workflows from a repo root
# rather than from safety/oxidized/<name>/ - so all they did there was look
# editable to anyone who found them. Keeping them together also collapses the
# workflow to one file: all 39 copies were byte-identical.
#
# The README is not one of them. It is a real file that this only edits, so
# it stays where it is written.
#
#   nu gen-project.nu            # write them
#   nu gen-project.nu --check    # fail if any is missing or stale

# subdir is "" for the usual one-directory project, whose crate sits at the
# repo root. A project with a sibling path dependency is published as several
# directories instead, so that `path = "../pcre2"` still resolves, and then
# the crate to build is one level down.
# Everything shared between published repos lives in the nix-workspace
# module, so this writes only what differs: the name, the description, and
# whichever build settings the project declares in projects.nuon.
#
# The description is written once, as the flake's own. The module reads it
# back out of the file, because a flake cannot ask itself for it: forcing any
# attribute of `self` needs the output shape the module helps decide.
#
# project_url is where that module comes from, and is the whole difference
# between the two files this generates: a Tangled URL for the published repo,
# a relative path for the copy the monorepo evaluates.
def flake-text [
    name: string
    description: string
    project_url: string
    subdir: string = ""
    native: list<string> = []
    build: list<string> = []
    check: bool = true
    toolchain: bool = false
    build_env: record = {}
    test_flags: list<string> = []
    aliases: record = {}
    setup_hook: string = ""
    fine: bool = false
    nixos_modules: record = {}
    home_modules: record = {}
]: nothing -> string {
    let quoted = {|xs| $xs | each {|x| $"\"($x)\"" } | str join " " }

    mut opts = []
    if not ($subdir | is-empty) { $opts = ($opts | append $"      subdir = \"($subdir)\";") }
    if not ($native | is-empty) {
        $opts = ($opts | append $"      nativeBuildInputs = [(do $quoted $native)];")
    }
    if not ($build | is-empty) {
        $opts = ($opts | append $"      buildInputs = [(do $quoted $build)];")
    }
    if not $check { $opts = ($opts | append "      doCheck = false;") }
    if not ($test_flags | is-empty) {
        $opts = ($opts | append $"      cargoTestFlags = [(do $quoted $test_flags)];")
    }
    if $toolchain { $opts = ($opts | append "      toolchain = true;") }
    if $fine { $opts = ($opts | append "      inherit inputs;") }
    # Keys are quoted without exception: c++filt, pkg-config and opt-rs are
    # not nix identifiers, and quoting only the ones that need it means the
    # generator has to know which those are.
    if not ($aliases | is-empty) {
        let pairs = ($aliases | items {|k, v| $"        \"($k)\" = \"($v)\";" } | str join "\n")
        $opts = ($opts | append $"      aliases = {\n($pairs)\n      };")
    }
    # A tool that replaces part of stdenv needs its hook installed, or a
    # build that lists it gets a binary that finds nothing.
    if not ($setup_hook | is-empty) {
        $opts = ($opts | append $"      setupHook = \"($setup_hook)\";")
    }
    if not ($build_env | is-empty) {
        let pairs = ($build_env | items {|k, v| $"        ($k) = \"($v)\";" } | str join "\n")
        $opts = ($opts | append $"      env = pkgs: {\n($pairs)\n      };")
    }

    # A pinned toolchain needs the overlay that provides rust-bin, and only
    # the project that asks for one pays for the extra input: an overlay is
    # its own flake, so carrying it in nix-workspace would make every published
    # repo fetch it.
    if $toolchain {
        $opts = ($opts | append "      withOverlays = [inputs.rust-overlay.overlays.default];")
    }
    let extra = if ($opts | is-empty) { "" } else { "\n" + ($opts | str join "\n") }

    # One input reads better on one line; a project with a second gets the
    # block, so the extras land inside `inputs` rather than beside it.
    let extra_inputs = ([
        (if $toolchain { "\n    # This project pins rustc through its own rust-toolchain.toml.\n    rust-overlay.url = \"github:oxalica/rust-overlay\";" } else { "" })
        (if $fine { "\n    # Declaring nix-lib is the whole of opting into the fine-grained\n    # per-crate build; the input carries the builder and its index.\n    nix-lib = {\n      url = \"git+https://tangled.org/overby.me/nix-lib\";\n      inputs.workspace.follows = \"workspace\";\n    };" } else { "" })
    ] | where {|s| $s != "" })
    let inputs_block = if ($extra_inputs | is-empty) {
        $"  inputs.workspace.url = \"($project_url)\";"
    } else {
        $"  inputs = {\n    workspace.url = \"($project_url)\";\n($extra_inputs | str join (char nl))\n  };"
    }

    # The two variants differ in that one line and in what they say about
    # themselves. Both are generated from here so they cannot drift: josh maps
    # the published one over the in-tree one when it publishes.
    let header = if ($project_url | str starts-with "path:") {
        "# Standalone build for this project, as the monorepo sees it. Generated by
# platform/tangled/publish/gen-project.nu; edit that template, not this file.
#
# nix-workspace is a path here, so the monorepo can evaluate its own change to
# it without publishing first, and so the root flake can take this as an
# input and check that it still builds. The published repo gets the same file
# with a Tangled URL, mapped over this one by josh.
#
# The relative path only resolves when this flake is reached through the repo
# root: as a root input, or `nix eval '.?dir=<this directory>#...'`."
    } else {
        "# Standalone build for the published repo. Generated by
# platform/tangled/publish/gen-project.nu; edit that template, not this file.
#
# The build, the devshell and its hooks, the formatter and the nixpkgs this
# resolves against are shared with every other repo published from the
# monorepo, and live in the nix-workspace flake. It is callable, so what is
# particular to this project is all that is left to say."
    }

    # A project's own NixOS module rides beside the build. It is an output of
    # this flake and part of the module it is, so a tree taking this repo as an
    # input gets it folded in with nothing further to say.
    let module_exports = if ($nixos_modules | is-empty) and ($home_modules | is-empty) { "" } else {
        (
            ($nixos_modules | items {|k, v| $"\n      nixosModules.\"($k)\" = ./($v);" })
            | append ($home_modules | items {|k, v| $"\n      homeModules.\"($k)\" = ./($v);" })
            | str join ""
        )
    }

    $"($header)
{
  description = \"($description)\";

($inputs_block)

  outputs = inputs:
    inputs.workspace {
      name = \"($name)\";($extra)($module_exports)
    };
}
"
}

const CI_TEXT = "# CI for the published repo. Generated by platform/tangled/publish/gen-project.nu.
#
# Inert inside the monorepo: Tangled reads .tangled/workflows from a repo's
# root, and here this sits under rust/<name>/. It becomes the published
# repo's CI once the directory is filtered out on its own.
clone:
  depth: 1
  skip: false
  submodules: false
dependencies:
  - git
  - nix
engine: microvm
image: nixos
steps:
  - command: |-
      nix --extra-experimental-features \"nix-command flakes\" \\
        flake check --print-build-logs
    name: nix flake check
when:
  - branch: '**'
    event:
      - push
"

# A pointer back to the monorepo, for a reader who arrives at a published
# repo and would otherwise open a pull request that the next sync destroys.
#
# josh ships README.md verbatim, so the published README is this one. The
# wording therefore has to be true read from either side, which is why it
# says where development happens rather than "you are looking at a mirror".
# Deliberately free of the script name: the marker is how an existing
# banner is found, so embedding a filename means renaming the script
# silently appends a second banner to every README.
const BANNER_BEGIN = "<!-- publish:begin -->"
const BANNER_END = "<!-- publish:end -->"

def banner-text [name: string, path: string, github: string]: nothing -> string {
    let mono = "https://tangled.org/overby.me/overby.me"
    $"($BANNER_BEGIN)
> Part of the [overby.me monorepo]\(($mono)), where this lives in
> [`($path)`]\(($mono)/tree/main/($path)) and where all development happens.
>
> It is also published on its own, as
> [tangled.org/overby.me/($name)]\(https://tangled.org/overby.me/($name)) and
> [github.com/($github)/($name)]\(https://github.com/($github)/($name)). Both
> are read-only mirrors, rebuilt from the monorepo with
> [josh]\(https://github.com/josh-project/josh): a commit made to either is
> overwritten by the next sync, so please open issues and pull requests on the
> monorepo.
($BANNER_END)"
}

# Insert or refresh the banner, after the title so the README still opens with
# its own heading.
def with-banner [readme: string, banner: string]: nothing -> string {
    let stripped = if ($readme | str contains $BANNER_BEGIN) {
        let head = ($readme | split row $BANNER_BEGIN | first)
        let tail = ($readme | split row $BANNER_END | skip 1 | str join $BANNER_END)
        $"($head)($tail | str trim -l -c "\n")"
    } else { $readme }

    let lines = ($stripped | lines)
    let title_at = ($lines | enumerate | where {|l| $l.item | str starts-with "# " } | get -o 0.index)
    if $title_at == null {
        $"($banner)\n\n($stripped)"
    } else {
        let before = ($lines | first ($title_at + 1) | str join "\n")
        let after = ($lines | skip ($title_at + 1) | str join "\n" | str trim -l -c "\n")
        $"($before)\n\n($banner)\n\n($after)"
    }
}

def main [--check, --github: string = "overby-me"]: nothing -> nothing {
    let here = ($env.FILE_PWD | default ".")
    # Walk up to the monorepo root rather than counting directories, which
    # went wrong the moment this script moved an area deeper.
    let root = (
        1..6 | reduce --fold $here {|_, acc|
            if ($acc | path join "flake.lock" | path exists) { $acc } else { $acc | path dirname }
        }
    )
    let projects = (open ($here | path join "projects.nuon"))
    let gen = ($here | path join "generated")

    let verb = if $check { "checking" } else { "writing" }
    print $"($verb) flakes for ($projects | length) projects"

    # One workflow for every project. It was 39 identical copies when each
    # lived in the repo it was for; the filter maps this one into each.
    if not $check {
        mkdir $gen
        $CI_TEXT | save -f ($gen | path join "ci.yml")
    }

    mut missing = []
    for p in $projects {
        let dir = ($root | path join $p.path)

        # The description is the one thing worth taking from the monorepo's
        # own module, so the two do not drift apart.
        # A repo's description is a fact about the project, and for a
        # workspace no single package's meta states it: fe-c's first entry
        # describes its runtime crate. State it in projects.nuon where it
        # differs, and scrape the project's own module otherwise.
        let stated = ($p | get -o description)
        # workspace.nix for a project the workspace names, default.nix for one
        # that is a module directory of its own.
        let module_nix = (
            [($dir | path join "workspace.nix") ($dir | path join "default.nix")]
            | where {|f| $f | path exists } | first
        )
        let description = if ($module_nix != null) {
            let hits = (open $module_nix | lines | where {|l| $l =~ 'description = "' })
            if ($hits | is-empty) { $"A Rust rewrite, published from a monorepo" } else {
                # Unescape what the module escaped, then re-escape for the
                # generated string: one description contains quotes.
                $hits | first
                | str replace -r '.*description = "' ''
                | str replace -r '";.*' ''
                | str replace -a '\"' '"'
                | str replace -a '"' '\"'
            }
        } else { "A Rust rewrite, published from a monorepo" }
        let description = ($stated | default $description)

        # A project with sibling path dependencies publishes as several
        # directories, so its crate lands one level down under its own name.
        let subdir = if (($p | get -o deps | default []) | is-empty) { "" } else {
            $p.path | path basename
        }
        # The generated flake builds a Rust crate, so a project without a
        # Cargo.toml gets none. nix-workspace is the module the others
        # import, and writes its own.
        let is_crate = ($dir | path join "Cargo.toml" | path exists)

        # One flake per project, written into the project's own directory and
        # published from there. It names nix-workspace by URL, which is the
        # same thing in tree and in a clone now that the framework is its own
        # repo: while it lived here the two had to differ in that one line,
        # and josh mapped a second copy over the first.
        #
        # Parenthesised: without it nushell ends the command at the first
        # newline and the call returns its last argument rather than the
        # flake, which writes an empty file to every project at once.
        let want_flake = if not $is_crate { "" } else { (
            flake-text $p.name $description "git+https://tangled.org/overby.me/nix-workspace" $subdir
                ($p | get -o nativeBuildInputs | default [])
                ($p | get -o buildInputs | default [])
                ($p | get -o doCheck | default true)
                ($p | get -o toolchain | default false)
                ($p | get -o env | default {})
                ($p | get -o cargoTestFlags | default [])
                ($p | get -o aliases | default {})
                ($p | get -o setupHook | default "")
                ($p | get -o fineBuild | default false)
                ($p | get -o nixosModules | default {})
                ($p | get -o homeModules | default {})
        ) }
        let flake = ($dir | path join "flake.nix")
        # Only a crate has a generated flake. Checking every project for one
        # passed by accident while these lived in the project's directory: a
        # nix project's own flake sat exactly where the generated one would
        # have, so its absence looked like presence.
        if $check {
            if $is_crate and not ($flake | path exists) {
                $missing = ($missing | append $p.name)
            }
            continue
        }
        if $is_crate {
            $want_flake | save -f $flake
            # nix-workspace gives every repo a checks.formatting that runs
            # alejandra over the tree, so a flake this generator wrote by hand
            # would fail the CI of the repo it was written for.
            ^alejandra --quiet $flake | ignore
        }

        # A published repo's README should be titled after the project, not
        # after the directory it used to live in: nineteen still said
        # oxidized-awk after the rename, which is the first line a visitor reads.
        # The heading is prose (Oxidized Systemd), the repo name is the
        # identifier (oxidized-systemd); both are derived from the same name
        # so they cannot disagree.
        let readme = ($dir | path join "README.md")
        if ($readme | path exists) {
            let words = ($p.name | split row "-")
            let special = {
                cli: "CLI", llvm: "LLVM", gcc: "GCC", pcre2: "PCRE2",
                nixos: "NixOS", pipewire: "PipeWire", xz: "XZ", wasm: "WASM"
            }
            let want = if ($p.name in ["fe-c" "h26xtoav1" "ast-grep-rules"]) {
                if $p.name == "fe-c" { "Fe-C" } else if $p.name == "h26xtoav1" { "h26xtoav1" } else { "ast-grep rules" }
            } else if ($p.name | str ends-with "pkg-config") {
                "Oxidized pkg-config"
            } else {
                $words | each {|w| ($special | get -o $w | default ($w | str capitalize)) } | str join " "
            }
            let txt = (open --raw $readme)
            let h1 = ($txt | lines | where {|l| $l | str starts-with "# " } | get -o 0)
            if $h1 != null and $h1 != $"# ($want)" {
                $txt | str replace $h1 $"# ($want)" | save -f $readme
                print $"  ($p.name): retitled to ($want)"
            }
        }
        let banner = if ($readme | path exists) {
            let now = (open --raw $readme)
            let want = (with-banner $now (banner-text $p.name $p.path $github))
            if $want != $now { $want | save -f $readme; " + README banner" } else { "" }
        } else { "" }
        let what = if $is_crate { $"generated/($p.name)/flake.nix" } else { "no flake (not a crate)" }
        print $"  ($p.name): ($what)($banner)"
    }

    if $check {
        # The workflow is one file now, so its absence is one failure rather
        # than 39, and every project's filter maps it.
        if not ($gen | path join "ci.yml" | path exists) {
            error make {msg: $"missing ($gen)/ci.yml, which every project's filter maps in"}
        }
        if not ($missing | is-empty) {
            error make {msg: $"missing flake for: ($missing | str join ', ')"}
        }
    }
}
