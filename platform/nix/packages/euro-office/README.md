# euro-office

A from-source Nix build of **Euro-Office DesktopEditors**, a fork of
[ONLYOFFICE DesktopEditors](https://github.com/ONLYOFFICE/DesktopEditors), for
`x86_64-linux` and `aarch64-darwin`.

Upstream builds through Docker and Xcode. This replaces that orchestration with
native derivations, so the whole graph is reproducible and no step reaches for a
container or a prebuilt binary.

## Layout

| file | what it builds |
|-|-|
| `default.nix` | the attribute set tying the phases together |
| `core.nix` | the editor core |
| `desktop-sdk.nix` | the desktop SDK |
| `desktop-common.nix` | the editors' JS/WASM payload |
| `app.nix` / `app-linux.nix` | the GUI application, per platform |
| `cef.nix` | the Chromium Embedded Framework dependency |
| `fonts.nix` | fonts, dictionaries and templates |
| `sources.nix` | pinned upstream revisions |

The 13 patches in [`patches/`](./patches) fall into two groups: `0001`-`0009`
teach the core's CMake to use system third-party libraries and to build on macOS
without Xcode-only paths, and the four `desktop-sdk-*` patches do the same for
the SDK.

## Status

Read [`PLAN.md`](./PLAN.md) before changing any sub-derivation — it maps the
full graph and records which phases build. As of its last update everything
builds from source on `aarch64-darwin` except the final storyboard compile,
which needs Xcode's `ibtool`.
