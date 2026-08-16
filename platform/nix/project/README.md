# Nix Project

<!-- publish:begin -->
> Part of the [overby.me monorepo](https://tangled.org/overby.me/overby.me), where this lives in
> [`platform/nix/project`](https://tangled.org/overby.me/overby.me/tree/main/platform/nix/project) and where all development happens.
>
> It is also published on its own, as
> [tangled.org/overby.me/nix-project](https://tangled.org/overby.me/nix-project) and
> [github.com/overby-me/nix-project](https://github.com/overby-me/nix-project). Both
> are read-only mirrors, rebuilt from the monorepo with
> [josh](https://github.com/josh-project/josh): a commit made to either is
> overwritten by the next sync, so please open issues and pull requests on the
> monorepo.
<!-- publish:end -->

The flakelight module shared by every repo published out of the
[overby.me monorepo](https://tangled.org/overby.me/overby.me).

Each of those repos is one directory of that monorepo, filtered out with
[josh](https://github.com/josh-project/josh) and given a flake of its own so it
builds on its own. They all need the same three things, so those live here
rather than in twenty-odd copies:

- a plain-nixpkgs build of the crate, read from its own `Cargo.toml`
- a devshell carrying the pre-commit hooks the monorepo holds it to
- a formatter

This flake is callable, through flakelight's `functor` option, so a consuming
flake is the call and whatever is different about that project:

```nix
{
  description = "A GNU sed-compatible stream editor written in Rust";

  inputs.project.url = "github:overby-me/nix-project";

  outputs = inputs:
    inputs.project ./. {
      name = "oxidized-sed";
      description = "A GNU sed-compatible stream editor written in Rust";
    };
}
```

That is the whole file. The inputs the call closes over are this flake's own,
so nixpkgs, flakelight and the hooks are the same in every published repo
without any of them naming a revision, and each of their locks has one direct
input.

The module is also exported as `flakelightModules.default`, for a flake that
needs to compose it with modules of its own.

## Workspaces

`project` is one published unit. `workspace` is the tree they live in, and
finds them: every directory holding a `default.nix` is a project and is
imported, so a monorepo's root flake stops carrying a list that drifts.

```nix
outputs = inputs:
  import ./platform/nix/project/workspace.nix ./. {
    inherit inputs;
    systems = ["x86_64-linux"];
    nixDir = ./platform/nix;
    imports = [./platform/nix/flakelight-modules/lib.nix];  # what is not a project
    projects.exclude = ["platform/nix"];
  };
```

`projects.exclude` takes repo-relative paths and `projects.depth` bounds the
walk (default 4, enough for `area/group/project`).

`moduleDirs` is the other half of not listing things: every `.nix` file
directly inside one is a flakelight module. One level deep, and no
`default.nix` rule, because these are not projects - naming the directory is
the point, and a subdirectory of it is a library the modules share rather
than another module.

Everything else is passed to flakelight untouched.

Unlike `project`, this closes over nothing and is a plain function: a
published repo wants our nixpkgs, but a monorepo pins its own, and its
flakelight has to be the one its other modules were written against, so both
come from the caller's `inputs`. That also means a monorepo publishing this
very module can import it **by path**, without taking a published revision of
itself as an input.

When it replaced a hand-written list of 56 directories here, the evaluated
output surface was identical except for one project that had been missing
from the list for as long as it had existed.

## Projects that name themselves from where they are

A project may provide `project.nix` instead of `default.nix`. The workspace
applies it to its own label, so the file states what it builds and never
where the names come from:

```nix
label: {lib, ...}: {
  packages = label.names {
    default = ...;   # -> wclip
    dev = ...;       # -> wclip-dev
  };
  checks = label.names {
    test-version = ...;   # -> wclip-test-version
  };
}
```

`default` is the project itself, which is Bazel's `//foo/bar` meaning
`//foo/bar:bar`; every other key hangs off it. `label.qualify "dev"` does one
name at a time.

This is the dendritic idea and path-derived naming at once, which look
opposed until the file *receives* its identity rather than typing it: one
uniform kind of file, discovered by walking the tree, and the tree decides
the names. The reason it needs applying rather than a module argument is that
`_module.args` is evaluation-wide - the module system cannot hand one module
a different argument from another - and the reason applying is unambiguous is
that every file found this way is the same kind of thing.

A project written in a local vocabulary also cannot spell a name outside its
own namespace, which is otherwise something
`checks.namespace-ownership` has to go looking for after the fact.

`default.nix` keeps working as an ordinary flakelight module, so a tree
migrates one project at a time.

## Worked examples

### A crate, its dev build and its tests

`dev/wclip/project.nix`. Every name is local; the label supplies the rest.

```nix
label: {lib, ...}: {
  packages = label.names {
    default = {lib, ...}: lib.buildCargoProject { pname = "rust-wclip"; ... };
    dev     = {lib, ...}: lib.buildCargoProject { pname = "rust-wclip-dev"; release = false; ... };
  };

  checks = label.names {
    test-version = pkgs: import ./testsuite.nix { inherit pkgs; name = "version"; };
  };
}
```

```text
packages.wclip          packages.wclip-dev          checks.wclip-test-version
```

`pname` stays the crate's own name. A label names a target; a crate is
resolved against `Cargo.lock` and keeps its identity.

### A project that contributes what nixDir contributes

`safety/oxidized/nixos/project.nix` produces a devshell, two NixOS
configurations and six checks - output types that would otherwise mean four
directories named after them, in a tree far from the project:

```nix
label: {
  devShells = label.names { default = pkgs: { packages = [pkgs.just]; }; };

  nixosConfigurations = label.names {
    default = _: { system = "x86_64-linux"; modules = [./base.nix ./systemd.nix]; };
  } // {
    # Named for what it is rather than for the project that keeps it.
    nixos-nix = _: { system = "x86_64-linux"; modules = [./base.nix]; };
  };

  checks = label.names {
    boot           = pkgs: import ./nixos-test.nix {inherit pkgs;};
    rung1-tmpfiles = pkgs: import ./rung1-tmpfiles-test.nix {inherit pkgs;};
  };
}
```

```text
devShells.oxidized-nixos              nixosConfigurations.oxidized-nixos
checks.oxidized-nixos-boot            nixosConfigurations.nixos-nix
checks.oxidized-nixos-rung1-tmpfiles
```

Before this the same file wrote `oxidized-nixos` into nine names by hand. It
now appears nowhere in it.

## How this relates to nixDir

They are the same idea reached from opposite ends, and both are in use.

**nixDir** puts the output type in the *directory* and the entry name in the
*file*: `platform/nix/packages/datui.nix` is `packages.datui`, and
`platform/nix/nixos-modules/services/openssh.nix` is a `nixosModules` entry.
Nothing inside those files says what they are or what they are called. That is
already path-derived naming, which is why converting them to self-declaring
modules would be a step backwards, and why they stay.

**A project module** puts the output type in the *file* and the name in the
*path*: `safety/oxidized/nixos/project.nix` says `nixosConfigurations` and
`checks`, and the label says `oxidized-nixos`.

nixDir is the better fit when the thing belongs to no project - a package
wrapping upstream software, a NixOS module about `services.flatpak`. The
project module is the better fit when it does, because it keeps a project's
outputs with the project and stops the name being repeated in each one. The
rule is the same either way: **the path is the address.**

## Options

| option | default | for |
|-|-|-|
| `name` | — | fallback package name; a workspace root has no `[package]` |
| `description` | `""` | the package's `meta.description` |
| `root` | the call's first argument | the repo root, so the module can read its `Cargo.toml` |
| `subdir` | `""` | the crate is one level down (see below) |
| `nativeBuildInputs` | `[]` | nixpkgs attribute names of build-time tools |
| `buildInputs` | `[]` | nixpkgs attribute names of libraries linked against |
| `doCheck` | `true` | run the crate's tests during the build |
| `cargoTestFlags` | `[]` | extra test arguments, e.g. `--skip` |
| `env` | `_: {}` | build-time environment, as a function of pkgs |
| `toolchain` | `false` | take rustc from the repo's `rust-toolchain.toml` |
| `hooks` | seven | the pre-commit hooks `nix develop` installs |

`subdir` exists because a crate whose `Cargo.toml` has a path dependency on a
sibling is published as several directories, so that `path = "../pcre2"` still
resolves. Its own crate is then one level down, and the build needs both
`cargoRoot` and `buildAndTestSubdir`, which do different jobs.

`toolchain` exists for compiler plugins: they link against rustc's internals,
whose API differs between releases, so they need the exact nightly named in
their own `rust-toolchain.toml` rather than whatever rustc nixpkgs ships. The
consuming flake supplies the `rust-overlay` overlay for it, through
`withOverlays`, which the call passes through:

```nix
inputs = {
  project.url = "github:overby-me/nix-project";
  rust-overlay.url = "github:oxalica/rust-overlay";
};

outputs = inputs:
  inputs.project ./. {
    name = "fe-c";
    toolchain = true;
    withOverlays = [inputs.rust-overlay.overlays.default];
  };
```

An overlay is its own flake, so carrying it here would make every published
repo fetch it for the one project that pins a toolchain.

## Contributing

Development happens in the monorepo, in `platform/nix/project`. This repo is a
read-only mirror, rebuilt by josh, and a commit made here is overwritten by the
next sync.