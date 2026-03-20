# rust-flatpak Implementation Plan

## Current State

Working CLI with real Flatpak installation reading, metadata parsing, permission
merging, bwrap sandbox construction, local install/uninstall, remote management,
and permission overrides. Can list, inspect, and override real Flatpak apps
installed on the system.

## Phase 1: Seccomp Filter Generation

Generate the BPF seccomp filter that real Flatpak always applies. Without this,
sandboxed apps can call dangerous syscalls.

### Tasks

- [x] Implement BPF instruction builder (`sock_filter` structs)
- [x] Block dangerous syscalls with `EPERM`: `syslog`, `uselib`, `acct`,
  `quotactl`, `add_key`, `keyctl`, `request_key`, `move_pages`, `mbind`,
  `get_mempolicy`, `set_mempolicy`, `migrate_pages`, `unshare`, `setns`,
  `mount`, `umount`, `umount2`, `pivot_root`, `chroot`
- [ ] Block `clone` when `CLONE_NEWUSER` flag is set (argument inspection —
  partially done, relies on `unshare` being blocked and bwrap `--disable-userns`)
- [x] Block `ioctl` with `TIOCSTI` and `TIOCLINUX` commands
- [x] Block new mount APIs with `ENOSYS`: `clone3`, `open_tree`, `move_mount`,
  `fsopen`, `fsconfig`, `fsmount`, `fspick`, `mount_setattr`
- [x] Conditionally block `perf_event_open`, `ptrace`, `personality` (allow
  with `--devel`)
- [x] Socket family allowlist: only permit `AF_UNSPEC`, `AF_LOCAL`, `AF_INET`,
  `AF_INET6`, `AF_NETLINK` (plus `AF_CAN`/`AF_BLUETOOTH` based on features)
- [x] Write compiled BPF to a memfd and pass via `--seccomp <fd>` to bwrap
- [x] Add unit tests for filter generation

## Phase 2: Instance Tracking

Track running sandbox instances so `flatpak ps`, `flatpak enter`, and
`flatpak kill` work.

### Tasks

- [x] On `flatpak run`, create `/run/user/<uid>/.flatpak/<instance-id>/`
- [x] Write `info` file (copy of `/.flatpak-info` content)
- [ ] Write `pid` file with the bwrap child PID (API exists, needs bwrap
  `--info-fd` integration to get the actual child PID)
- [ ] Parse `--info-fd` output from bwrap to get `bwrapinfo.json`
- [x] Clean up instance directory on sandbox exit
- [x] Implement `flatpak ps` reading from instance directories (with stale
  instance cleanup)
- [x] Implement `flatpak enter` using `nsenter` into the running sandbox
- [x] Implement `flatpak kill` sending `SIGTERM`/`SIGKILL` to instance PID
- [x] Clean up temp files (`/tmp/.flatpak-info-*`, `/tmp/.flatpak-passwd-*`)
  on exit using a cleanup handler

## Phase 3: Capability Handling

Wire up the parsed `--cap-add`/`--cap-drop` to actual bwrap arguments.

### Tasks

- [x] Map capability names (`CAP_SYS_ADMIN`, `CAP_NET_RAW`, `ALL`, etc.) to
  bwrap `--cap-add`/`--cap-drop` arguments
- [x] Apply capability operations in order (matching real Flatpak behavior
  where default is to drop all, then apply ops sequentially)
- [x] Pass through to bwrap command line
- [x] Parse `--cap-add`/`--cap-drop` from `flatpak run` CLI

## Phase 4: D-Bus Proxy

Integrate `xdg-dbus-proxy` for filtered D-Bus access.

### Tasks

- [x] Find `xdg-dbus-proxy` binary on `PATH`
- [x] Parse `[Session Bus Policy]` and `[System Bus Policy]` from metadata
- [x] Build proxy filter arguments from policy (own/talk/see/none per bus name)
- [x] Launch proxy process before the main sandbox
- [x] Create proxy socket in a temp directory
- [x] Bind-mount proxy socket into the sandbox at the expected D-Bus path
- [x] Handle `sockets=session-bus` (direct, unfiltered access) vs. filtered
- [x] Handle `sockets=system-bus` similarly
- [x] Clean up proxy process on sandbox exit (Drop impl on RunningProxy)
- [x] Default policy: allow portal access, Flatpak bus, dconf, GTK VFS
- [ ] Support `--log` for proxy debugging

## Phase 5: OSTree Client

Implement the minimal OSTree client needed to pull from Flatpak remotes
(Flathub). This is the largest piece of work.

### Tasks

- [x] Implement OSTree object types: commit, dirtree, dirmeta, file
- [ ] Implement content-addressed object storage (`objects/<hash>.{commit,dirtree,dirmeta,file}`)
- [x] Parse OSTree summary file format (GVariant binary)
- [x] Fetch summary from remote via HTTPS (rustls + webpki-roots)
- [x] Resolve Flatpak refs from summary (e.g., `app/org.example.App/x86_64/stable`)
- [x] Implement HTTP object pulling (fetch individual objects by hash)
- [ ] Implement static delta support (optional, for faster pulls)
- [x] Checkout commit to deploy directory (reconstruct filesystem from objects)
- [ ] GPG signature verification of commits and summary
- [x] Implement `flatpak install <remote> <ref>` using the above
- [x] Implement `flatpak update` (checks remote, full pull not yet done)
- [x] Implement `flatpak remote-ls` to list available refs from summary
- [x] Implement `flatpak remote-info` to show commit details
- [ ] Handle `.flatpakrepo` file parsing for `remote-add --from=<file>`
- [ ] Implement local repo storage at `<installation>/repo/`

## Phase 6: Extension Handling

Resolve, download, and mount extensions into the sandbox.

### Tasks

- [x] Parse `[Extension <name>]` groups from runtime and app metadata
- [x] Resolve extension refs from installed extensions (with version/branch search)
- [ ] Auto-download missing extensions (requires Phase 5)
- [x] Mount extensions at their declared directory in the sandbox
- [x] Handle `add-ld-path` — append extension lib paths to `LD_LIBRARY_PATH`
- [ ] Handle `merge-dirs` — overlay extension directories
- [x] Handle `subdirectories` — mount sub-extensions
- [ ] Regenerate `ld.so.cache` when extensions add library paths (run `ldconfig`
  in a sub-bwrap)

## Phase 7: Portal Support

Integrate with XDG desktop portals for mediated access to host resources.

### Tasks

- [x] Document portal: CLI stubs for `flatpak documents`, `document-export`,
  `document-unexport`, `document-info` (full D-Bus portal API not yet implemented)
- [x] Permission store: CLI stubs for `flatpak permissions`, `permission-show`,
  `permission-set`, `permission-remove`, `permission-reset`
- [ ] Mount `xdg-document-portal` socket into sandbox
- [ ] Set portal-related environment variables (`FLATPAK_PORTAL_PID`, etc.)
- [ ] Full D-Bus client for portal APIs

## Phase 8: Remaining CLI Commands

### Tasks

- [x] `flatpak make-current` — set default version for an app
- [x] `flatpak mask` — mask out updates for specific refs
- [x] `flatpak pin` — pin runtimes to prevent automatic removal
- [ ] `flatpak history` — track install/update/uninstall events in a log
- [ ] `flatpak search` — query Flathub appstream data (requires fetching
  appstream XML from remote)
- [ ] `flatpak create-usb` — export refs to removable media

## Phase 9: Build Commands

Implement the `flatpak build-*` workflow for building Flatpak apps.

### Tasks

- [x] `flatpak build-init` — initialize a build directory with runtime/SDK
- [x] `flatpak build` — run a command inside the build sandbox (with SDK
  mounted as /usr, writable /app, network access for package downloads)
- [x] `flatpak build-finish` — finalize metadata, permissions, and exports
  (desktop files, icons, appdata, D-Bus services)
- [x] `flatpak build-export` — export build to a local repository
- [x] `flatpak build-bundle` / `build-import-bundle` — tar-based bundles
- [x] `flatpak build-sign` — stub (GPG signing not yet implemented)
- [x] `flatpak build-update-repo` — regenerate summary file
- [x] `flatpak build-commit-from` — stub (requires full OSTree commit creation)
- [x] `flatpak repo` — show repository information

## Phase 10: Sandbox Fidelity

Bring the sandbox setup to parity with real Flatpak so apps work correctly.

### Tasks

- [ ] Bind-mount `/sys` subdirectories read-only (`/sys/block`, `/sys/bus`,
  `/sys/class`, `/sys/dev`, `/sys/devices`)
- [ ] Enable `--new-session` by default (prevents TIOCSTI terminal injection)
- [ ] Use memfd + `--ro-bind-data` for `.flatpak-info` instead of temp files
- [ ] Generate `/etc/passwd` and `/etc/group` via memfd + `--ro-bind-data`
- [ ] Set up timezone symlink (`/etc/localtime` → `/usr/share/zoneinfo/<TZ>`)
  and write `/etc/timezone`
- [ ] Bind-mount host font directories into the sandbox
  (`/usr/share/fonts`, `/usr/local/share/fonts`, `~/.local/share/fonts`,
  `/etc/fonts`)
- [ ] Bind-mount host icon theme directories
- [ ] Set up per-app shared `/tmp` and `/dev/shm` directories (persistent
  across instances of the same app, isolated from other apps)
- [ ] Regenerate `ld.so.cache` by running `ldconfig` in a sub-bwrap when
  extensions add library paths
- [ ] Mount `/run/host/fonts`, `/run/host/icons` for host resource access
- [ ] Create `/run/flatpak/.flatpak/<instance-id>` and bind-mount into sandbox

## Phase 11: Native Deflate and Local Object Cache

Remove the python3 dependency for decompression and avoid re-downloading
objects that have already been fetched.

### Tasks

- [ ] Implement raw deflate decompression natively (either minimal pure-Rust
  inflate or add `flate2`/`miniz_oxide` as a dependency)
- [ ] Store fetched OSTree objects locally in `<installation>/repo/objects/`
  with the standard `<XX>/<YY...>.<ext>` layout
- [ ] Check local cache before fetching objects from the remote
- [ ] Implement `flatpak update` to actually pull newer commits (compare
  local commit checksum with remote summary, re-checkout if different)
- [ ] Handle HTTP chunked transfer-encoding in the HTTP client (some repos
  use it for large objects)
- [ ] Add progress reporting during large pulls (object count / total)
- [ ] Implement static delta support for faster pulls (optional — large
  effort but huge performance improvement)

## Phase 12: Seccomp Hardening

Complete the remaining seccomp filter gaps.

### Tasks

- [ ] Implement proper `clone` flag inspection for `CLONE_NEWUSER` using
  BPF_ALU (AND instruction) to mask and test the flags argument
- [ ] Add GPG signature verification for OSTree summary and commit objects
  (either shell out to `gpg` or implement minimal OpenPGP parsing)
- [ ] Harden `personality` filtering to only allow known-safe values
- [ ] Block `prctl(PR_SET_MM)` which can manipulate memory mappings

## Phase 13: Instance Tracking Completion

Wire up bwrap's `--info-fd` to capture the actual child PID and process info.

### Tasks

- [ ] Create a pipe and pass the read end via `--info-fd` to bwrap
- [ ] Parse bwrap's JSON output (`{"child-pid": N}`) from the pipe
- [ ] Write the actual child PID to the instance directory's `pid` file
- [ ] Capture and write `bwrapinfo.json` to the instance directory
- [ ] Use the real PID for `flatpak ps`, `flatpak kill`, `flatpak enter`

## Phase 14: D-Bus Proxy Completion

### Tasks

- [ ] Proxy the accessibility bus (`AT_SPI_BUS_ADDRESS`)
- [ ] Handle `sockets=inherit-wayland-socket` (pass through existing Wayland
  socket from parent sandbox)
- [ ] Wire up `--log` flag for proxy debugging
- [ ] Support `[Accessibility Bus Policy]` from metadata

## Phase 15: Extension Completion

### Tasks

- [ ] Implement `merge-dirs` — create overlay directories that merge content
  from multiple extensions into a single mount point
- [ ] Auto-download missing extensions when running an app (prompt user,
  then pull via OSTree client)
- [ ] Regenerate `ld.so.cache` when extensions add `add-ld-path` entries
  (run `ldconfig` in a sub-bwrap to generate the cache, then bind-mount it)

## Phase 16: Portal Implementation

Replace the portal stubs with real D-Bus client implementations.

### Tasks

- [ ] Implement a minimal D-Bus client (authenticate, call methods, read
  replies) — either pure Rust or use `dbus-send`/`gdbus` subprocess
- [ ] Document portal: talk to `org.freedesktop.portal.Documents` for
  `document-export`, `document-unexport`, `document-info`
- [ ] Permission store: talk to `org.freedesktop.impl.portal.PermissionStore`
  for `permission-show`, `permission-set`, `permission-remove`, `permission-reset`
- [ ] Mount the document portal socket (`/run/user/<uid>/doc`) into the sandbox
- [ ] Set `FLATPAK_PORTAL_PID` environment variable

## Phase 17: Remaining CLI and Formats

### Tasks

- [ ] `flatpak search` — fetch and parse Flathub appstream XML/catalog data
- [ ] `flatpak history` — implement an event log (install/update/uninstall
  events with timestamps, stored in `<installation>/history.log`)
- [ ] `flatpak create-usb` — export refs to a USB sideload directory
- [ ] `build-bundle` — implement proper Flatpak bundle format (OSTree commit
  in a single file with metadata header) instead of tar
- [ ] `build-commit-from` — implement full OSTree commit creation from an
  existing ref's content tree
- [ ] `build-sign` — implement GPG signing of OSTree commits
- [ ] `.flatpakrepo` file parsing for `remote-add --from=<file>`
- [ ] Support `--columns` flag for `list`, `remote-ls`, `ps` output formatting
- [ ] Support `--arch` flag for cross-architecture operations

## Priority Order (Phases 10-17)

1. **Sandbox fidelity** (Phase 10) — highest impact, apps fail without /sys,
   fonts, timezone
2. **Native deflate + cache** (Phase 11) — removes python3 dep, makes install
   usable for real apps
3. **Seccomp hardening** (Phase 12) — security improvement
4. **Instance tracking** (Phase 13) — correctness for ps/kill/enter
5. **D-Bus proxy completion** (Phase 14) — needed for a11y and edge cases
6. **Extension completion** (Phase 15) — needed for apps using GL/codecs
7. **Portal implementation** (Phase 16) — needed for file access and desktop
   integration
8. **Remaining CLI** (Phase 17) — polish and full compatibility
