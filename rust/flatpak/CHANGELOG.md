# Changelog

All notable changes to `rust-flatpak` are documented in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0] — Initial release

A from-scratch Rust reimplementation of Flatpak with feature parity across
all 24 planned implementation phases and the full upstream-inspired test
suite (124 sandbox-free Nix checks + 75+ NixOS VM tests across 15 categories).

### Sandbox & seccomp (Phases 1, 3, 10, 12)

- BPF seccomp filter generation that matches upstream Flatpak's blocklist:
  `syslog`, `uselib`, `acct`, `quotactl`, key/keyring syscalls, mempolicy/
  pages syscalls, namespace syscalls (`unshare`, `setns`), mount syscalls
  (`mount`, `umount`, `umount2`, `pivot_root`, `chroot`), and the new mount
  API (`open_tree`, `move_mount`, `fsopen`, etc.) returning `ENOSYS`.
- `clone(CLONE_NEWUSER)` blocked via BPF_ALU AND flag inspection.
- `ioctl(TIOCSTI/TIOCLINUX)` blocked to prevent terminal injection.
- `perf_event_open`, `ptrace`, `personality` conditionally blocked
  (allowed under `--devel`); `prctl(PR_SET_MM)` always blocked.
- Socket family allowlist restricted to `AF_UNSPEC`, `AF_LOCAL`, `AF_INET`,
  `AF_INET6`, `AF_NETLINK` (+ optional `AF_CAN`/`AF_BLUETOOTH`).
- Compiled BPF passed to `bwrap` via memfd + `--seccomp <fd>`.
- `--cap-add`/`--cap-drop` parsing and pass-through to `bwrap`.
- Sandbox fidelity: read-only `/sys` subdirectories, `--new-session` by
  default, memfd-based `.flatpak-info`/`passwd`/`group`, timezone symlinks,
  host font/icon bind-mounts, per-app shared `/tmp`, `/run/host/{fonts,
  icons,container-manager}`.

### Instance tracking (Phases 2, 13)

- `flatpak ps`, `flatpak enter` (via `nsenter`), `flatpak kill`.
- `--info-fd` pipe parses `bwrapinfo.json` to capture the real child PID.
- Stale instance cleanup on startup; temp file cleanup on exit.

### D-Bus proxy (Phases 4, 14)

- Integrates `xdg-dbus-proxy` for filtered session, system, and
  accessibility bus access driven by `[*Bus Policy]` metadata sections.
- Default policy allows portals, the Flatpak bus, dconf, and GTK VFS.
- `sockets=session-bus`/`system-bus`/`inherit-wayland-socket` honoured.
- `FLATPAK_DBUS_PROXY_LOG` env var enables `--log` for proxy debugging.

### OSTree client (Phases 5, 11, 19, 21, 22)

- Native Rust implementation of OSTree commit/dirtree/dirmeta/file objects
  with content-addressed storage at `repo/objects/<XX>/<YY>.<ext>`.
- GVariant binary serializer (`gvariant.rs`) supporting Bool, Byte,
  Uint32, Uint64, Str, ByteArray, Array, Tuple, DictEntry, Variant with
  proper alignment and framing offset handling.
- HTTPS pull via rustls + webpki-roots; HTTP redirects, chunked transfer
  encoding, connection reuse via global pool, parallel object fetching.
- Native deflate via `miniz_oxide` (no `python3` dependency).
- Local object cache short-circuits redundant downloads.
- Full **static delta** support: superblock parsing, all delta opcodes
  (`OPEN_SPLICE_AND_CLOSE`, `OPEN`, `WRITE`, `SET/UNSET_READ_SOURCE`,
  `CLOSE`, `BSPATCH`), and a complete bspatch implementation handling
  both standard `BSDIFF40` and OSTree raw inline format with
  offset-encoded signed 64-bit integers, diff+old addition, extra
  verbatim copy, and seek adjustment.
- Real OSTree commit creation (`build-export`, `build-commit-from`).
- GPG verification via `gpgv` for both summary and commit objects.

### Bundle format (Phase 18)

- Proper Flatpak bundle format with magic header, ref name, metadata,
  and deflate-compressed tar payload (replaces the original tar-only
  prototype). `build-import-bundle` retains tar fallback for legacy bundles.
- `flatpak update --bundle=PATH` re-imports an updated bundle and
  appends an `update` history entry.

### Build pipeline (Phase 9, 19, 20)

- `build-init` (with `--extension-tag`, full subdir layout).
- `build` runs commands inside a build sandbox with SDK at `/usr`,
  writable `/app`, and network access.
- `build-finish` writes `--command`, `--share`, `--socket`, `--device`,
  `--filesystem`, `--persist`, `--sdk`, `--require-version` into metadata
  and exports desktop files, icons, appdata, and D-Bus services.
- `build-export` creates real OSTree commits alongside file copies; can
  optionally sign with `--gpg-sign=KEYID`.
- `build-bundle`/`build-import-bundle` use the structured bundle format.
- `build-sign` signs commits via `gpg --detach-sign`, storing the
  signature as a `.commitmeta` object.
- `build-update-repo` regenerates the GVariant binary summary and
  persists `--title`, `--redirect-url`, `--default-branch` in INI config.

### Portals (Phases 7, 16, 24)

- Native D-Bus client built on `zbus` with `MatchRule`-based subscription
  to portal `Response` signals.
- Document portal: `document-export`, `document-info`, `document-unexport`.
- Permission store: `permission-set`, `permission-show`, `permission-remove`,
  `permission-reset`.
- On-disk fallback portal (`$XDG_DATA_HOME/flatpak/portal/`) activates
  when the real portals are unreachable, enabling tests in headless
  environments.
- Document portal socket bind-mounted into the sandbox; `FLATPAK_PORTAL_PID`
  env var set when available.

### Extensions (Phases 6, 15, 23)

- Parses `[Extension *]` groups with `add-ld-path`, `merge-dirs`,
  `subdirectories` semantics.
- Mounts extensions at their declared directories.
- Regenerates `ld.so.cache` via a sub-bwrap when `add-ld-path` is used.
- Auto-installs missing extensions on `flatpak run` by searching
  configured remotes and pulling via the OSTree client.

### CLI surface (Phases 8, 17)

- Subcommands: `install`, `uninstall`, `update`, `list`, `info`, `run`,
  `ps`, `kill`, `enter`, `override`, `config`, `history`, `repair`,
  `search`, `mask`, `pin`, `make-current`, `remote-add`, `remote-delete`,
  `remote-modify`, `remote-ls`, `remote-info`, `create-usb`, `documents`,
  `permissions`, `permission-*`, `complete` (tab completion), and the
  `build-*` family.
- Flags: `--user`/`--system`, `--columns`, `--arch`, `--subpath`,
  `--require-version`, `--bundle`, `--persist`, `--nofilesystem`,
  `--show-commit`, `--show-location`, `--show-runtime`, `--show-sdk`,
  `--show-extensions`, `--file-access`, and others.
- `.flatpakrepo` parsing for `remote-add --from=<file|URL>`.

### Security

- **CVE-2024-32462** — sanitised `flatpak run` command escaping (covered
  by `vm-run-cve-2024-32462`).
- **CVE-2024-42472** — `--persist` rejects `..`, absolute paths, and
  symlinks (covered by `vm-persist-symlink-escape`,
  `vm-persist-path-traversal`).
- **CVE-2021-43860** — NUL-byte rejection in `Metadata::from_file` and
  `Metadata::parse` (covered by `metadata-nul-byte-rejected`).
- `--no-setuid` / `--no-world-writable` enforcement during install.
- GPG signature verification on summary fetch and commit checkout.

### Tests

- 124 sandbox-free Nix checks (`testsuite.nix`) covering CLI surface,
  override management, config & history, build commands, metadata,
  install/uninstall/list/info, remote management, and miscellaneous.
- 75+ NixOS VM tests (`vmtest.nix`) across 15 categories with full
  coverage of sandbox execution, override effects, config, bundle
  roundtrip, build-update-repo, repair, info-deep, history, seccomp,
  extensions, D-Bus proxy, documents/permissions, remote+network,
  metadata validation, and misc/security regressions.

### Known limitations

- The native D-Bus client lives inside `portals.rs` rather than a
  dedicated `dbus_client.rs` module.
- `build-sign` and GPG verification shell out to `gpg`/`gpgv`; no native
  OpenPGP implementation.
- A minor `clippy::manual_repeat_n` warning remains in `src/deltas.rs`
  test code (only triggered by `cargo clippy --all-targets`, which is
  not enforced by pre-commit).
