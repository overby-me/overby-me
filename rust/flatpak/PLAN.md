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

## Priority Order

1. **Seccomp** (Phase 1) — security critical, without it the sandbox is weak
2. **Instance tracking** (Phase 2) — needed for ps/enter/kill and temp cleanup
3. **Capabilities** (Phase 3) — small, completes sandbox feature parity
4. **D-Bus proxy** (Phase 4) — most GUI apps need D-Bus access
5. **OSTree client** (Phase 5) — enables remote install/update from Flathub
6. **Extensions** (Phase 6) — needed to run most real-world apps
7. **Portals** (Phase 7) — needed for proper desktop integration
8. **Remaining CLI** (Phase 8) — polish and completeness
9. **Build commands** (Phase 9) — for developers building Flatpak apps
