# rust-flatpak

A from-scratch Rust implementation of [Flatpak](https://flatpak.org/) — the
Linux application sandboxing and distribution framework. It provides a single
`flatpak` binary that can install, run, build, sign, and distribute Flatpak
applications without depending on the upstream C codebase or `libostree`.

## Status

All 24 implementation phases are complete. The test suite consists of:

- **124 sandbox-free Nix checks** covering CLI surface, override management,
  config & history, build commands, metadata, install/uninstall/list/info,
  remote management, and miscellaneous commands.
- **75+ NixOS VM tests** across 15 categories (sandbox execution, override
  effects, config, bundle roundtrip, build-update-repo, repair, info,
  history, seccomp, extensions, D-Bus proxy, documents/permissions,
  remote+network, metadata validation, misc/security).

Run the full suite with `nix flake check`.

## Features

- **Sandboxing** — bubblewrap-based sandbox with full seccomp filter
  generation (BPF, blocks dangerous syscalls including new mount APIs,
  `clone(CLONE_NEWUSER)`, `TIOCSTI`/`TIOCLINUX` ioctls, restricted socket
  families), capability handling, and instance tracking (`ps`, `enter`,
  `kill`, `bwrapinfo.json` parsing).
- **OSTree client** — native implementation of OSTree commit/dirtree/dirmeta/
  file objects, GVariant binary summary, content-addressed object storage,
  HTTPS pull (rustls + webpki-roots), local object cache, native deflate
  via `miniz_oxide`, parallel object fetching with connection reuse, HTTP
  redirect/chunked transfer support, and full **static delta** support
  including a complete bspatch implementation.
- **Build pipeline** — `build-init`, `build`, `build-finish`, `build-export`,
  `build-bundle`/`build-import-bundle` (proper Flatpak bundle format with
  GVariant header + deflate-compressed payload), `build-sign` (GPG via
  `gpg --detach-sign`), `build-update-repo` (with `--title`, `--redirect-url`,
  `--default-branch`), `build-commit-from`, and `repo` info.
- **D-Bus proxy** — integrates `xdg-dbus-proxy` to filter session, system,
  and accessibility buses based on `[*Bus Policy]` metadata sections.
- **Portals** — native D-Bus client (via `zbus`) for the document portal
  (`document-export`, `document-info`, `document-unexport`) and the
  permission store (`permission-set`, `permission-show`, `permission-remove`,
  `permission-reset`), with on-disk fallback when the real portals are
  unreachable (useful for tests).
- **Extensions** — parses `[Extension *]` groups, mounts extensions with
  `add-ld-path`/`merge-dirs`/`subdirectories` support, regenerates
  `ld.so.cache` via a sub-bwrap, and **auto-installs missing extensions**
  on `flatpak run`.
- **Sandbox fidelity** — read-only `/sys` subdirectories, `--new-session`
  by default, memfd-based `.flatpak-info`/`passwd`/`group`, timezone
  symlinks, host font/icon bind-mounts, per-app shared `/tmp`,
  `/run/host/{fonts,icons,container-manager}`.
- **Security hardening** — CVE-2024-32462 (run command escaping),
  CVE-2024-42472 (`--persist` symlink/path-traversal escapes),
  CVE-2021-43860 (NUL-byte rejection in metadata), `--no-setuid`,
  `--no-world-writable`, GPG signature verification of summaries and
  commits via `gpgv`.
- **CLI** — install/uninstall/list/info/run/ps/kill/enter/override/config/
  history/repair/search/mask/pin/make-current/remote-add/-delete/-modify/
  -ls/-info/create-usb/documents/permissions/complete (tab completion).
  Supports `--user`/`--system`, `--columns`, `--arch`, `--subpath`,
  `--require-version`, `--bundle`, and many more upstream flags.

## Building

The project lives in a Nix flake-based monorepo. From the repository root:

```text
# Debug build via Cargo:
cd rust/flatpak && cargo build

# Release build via Nix:
nix build .#rust-flatpak

# Run the full test suite (sandbox-free + VM tests):
nix flake check
```

The binary is produced at `target/{debug,release}/flatpak` (Cargo) or
`result/bin/flatpak` (Nix).

## Usage

Drop-in compatible with the upstream `flatpak` CLI for the supported
subcommands:

```text
flatpak --user remote-add flathub https://flathub.org/repo/flathub.flatpakrepo
flatpak --user install flathub org.gnome.Calculator
flatpak run org.gnome.Calculator
flatpak --user list --app
flatpak --user info org.gnome.Calculator --show-permissions
flatpak --user override org.gnome.Calculator --filesystem=~/Documents
```

## Project Layout

| Path | Contents |
|---|---|
| `src/main.rs` | CLI dispatch and command implementations |
| `src/sandbox.rs` | bubblewrap sandbox construction |
| `src/seccomp.rs` | BPF seccomp filter generation |
| `src/ostree.rs` | OSTree client (objects, summary, pull, checkout) |
| `src/deltas.rs` | OSTree static delta parser + bspatch |
| `src/gvariant.rs` | GVariant binary serializer |
| `src/dbus_proxy.rs` | `xdg-dbus-proxy` integration |
| `src/installation.rs` | Installation discovery and ref resolution |
| `src/portals.rs` | Document + permission portal client (`zbus`) + native D-Bus wire client |
| `src/build.rs` | `build-*` subcommand implementations |
| `src/metadata.rs` | INI-style metadata/override parser |
| `src/instance.rs` | Instance tracking (`ps`, `enter`, `kill`) |
| `src/extensions.rs` | Extension resolution and mounting |
| `tests/` | Sandbox-free shell test scripts (`testsuite.nix`) |
| `vmtests/` | NixOS VM test scripts (`vmtest.nix`) |
| `default.nix` | Nix package + checks attrset |
| `PLAN.md` | Detailed implementation plan and test parity tracker |

## Dependencies

Runtime: `bubblewrap`, `xdg-dbus-proxy` (optional, for D-Bus filtering),
`gpg`/`gpgv` (optional, for signing/verification).

Rust crates (see `Cargo.toml`): `libc`, `serde`, `miniz_oxide` (deflate),
`rustls` + `webpki-roots` (HTTPS), `zbus` (D-Bus client).

## License

MIT
