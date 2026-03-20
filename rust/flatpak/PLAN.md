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
- [x] Set portal-related environment variables (`FLATPAK_PORTAL_PID`)
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

- [x] Bind-mount `/sys` subdirectories read-only (`/sys/block`, `/sys/bus`,
  `/sys/class`, `/sys/dev`, `/sys/devices`)
- [x] Enable `--new-session` by default (prevents TIOCSTI terminal injection)
- [x] Use memfd + `--ro-bind-data` for `.flatpak-info` instead of temp files
- [x] Generate `/etc/passwd` and `/etc/group` via memfd + `--ro-bind-data`
- [x] Set up timezone symlink (`/etc/localtime` → `/usr/share/zoneinfo/<TZ>`)
  and write `/etc/timezone`
- [x] Bind-mount host font directories into the sandbox
  (`/usr/share/fonts`, `/usr/local/share/fonts`, `~/.local/share/fonts`,
  `/etc/fonts`)
- [x] Bind-mount host icon theme directories
- [x] Set up per-app shared `/tmp` directories (persistent across instances
  of the same app, isolated from other apps)
- [x] Mount `/run/host/fonts`, `/run/host/icons` for host resource access
- [x] Write `/run/host/container-manager` via memfd
- [ ] Regenerate `ld.so.cache` by running `ldconfig` in a sub-bwrap when
  extensions add library paths
- [ ] Create `/run/flatpak/.flatpak/<instance-id>` and bind-mount into sandbox

## Phase 11: Native Deflate and Local Object Cache

Remove the python3 dependency for decompression and avoid re-downloading
objects that have already been fetched.

### Tasks

- [x] Implement raw deflate decompression natively via `miniz_oxide` (no
  more python3 dependency)
- [x] Store fetched OSTree objects locally in `<installation>/repo/objects/`
  with the standard `<XX>/<YY...>.<ext>` layout
- [x] Check local cache before fetching objects from the remote
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

- [x] Implement proper `clone` flag inspection for `CLONE_NEWUSER` using
  BPF_ALU AND instruction to mask and test the flags argument
- [ ] Add GPG signature verification for OSTree summary and commit objects
  (either shell out to `gpg` or implement minimal OpenPGP parsing)
- [ ] Harden `personality` filtering to only allow known-safe values
- [x] Block `prctl(PR_SET_MM)` which can manipulate memory mappings

## Phase 13: Instance Tracking Completion

Wire up bwrap's `--info-fd` to capture the actual child PID and process info.

### Tasks

- [x] Create a pipe and pass the write end via `--info-fd` to bwrap
- [x] Parse bwrap's JSON output (`{"child-pid": N}`) from the pipe
- [x] Write the actual child PID to the instance directory's `pid` file
- [x] Capture and write `bwrapinfo.json` to the instance directory
- [x] Use the real PID for `flatpak ps`, `flatpak kill`, `flatpak enter`

## Phase 14: D-Bus Proxy Completion

### Tasks

- [x] Proxy the accessibility bus (`AT_SPI_BUS_ADDRESS`)
- [x] Handle `sockets=inherit-wayland-socket` (pass through existing Wayland
  socket from parent sandbox)
- [x] Wire up `--log` flag for proxy debugging (via `FLATPAK_DBUS_PROXY_LOG`)
- [x] Support `[Accessibility Bus Policy]` from metadata

## Phase 15: Extension Completion

### Tasks

- [x] Implement `merge-dirs` — create symlink-based merged directories from
  multiple extensions
- [ ] Auto-download missing extensions when running an app (prompt user,
  then pull via OSTree client)
- [x] Regenerate `ld.so.cache` when extensions add `add-ld-path` entries
  (run `ldconfig` in a sub-bwrap to generate the cache, then bind-mount it)

## Phase 16: Portal Implementation

Replace the portal stubs with real D-Bus client implementations.

### Tasks

- [x] Implement D-Bus client via `gdbus`/`busctl` subprocess calls
- [x] Document portal: talk to `org.freedesktop.portal.Documents` for
  `document-export`, `document-unexport`, `document-info`
- [x] Permission store: talk to `org.freedesktop.impl.portal.PermissionStore`
  for `permission-show`, `permission-set`, `permission-remove`, `permission-reset`
- [x] Mount the document portal socket (`/run/user/<uid>/doc`) into the sandbox
- [x] Set `FLATPAK_PORTAL_PID` environment variable

## Phase 17: Remaining CLI and Formats

### Tasks

- [x] `flatpak search` — search remote refs by keyword across all configured
  remotes
- [x] `flatpak history` — event log with timestamps, stored in
  `<installation>/history.log`, written on install/uninstall
- [x] `flatpak create-usb` — export refs to a USB sideload directory
  (`~/.flatpak-usb/` on the mount point)
- [ ] `build-bundle` — implement proper Flatpak bundle format (OSTree commit
  in a single file with metadata header) instead of tar
- [ ] `build-commit-from` — implement full OSTree commit creation from an
  existing ref's content tree
- [ ] `build-sign` — implement GPG signing of OSTree commits
- [x] `.flatpakrepo` file parsing for `remote-add --from=<file>` (supports
  both local files and HTTP URLs)
- [x] Support `--columns` flag for `list` output formatting
- [x] Support `--arch=` flag for filtering `list` output
- [x] GPG signature verification via `gpgv` subprocess
  (`verify_gpg_signature`, `fetch_and_verify_summary`)
- [x] `FLATPAK_PORTAL_PID` set when document portal is available
- [x] D-Bus proxy `--log` support via `FLATPAK_DBUS_PROXY_LOG` env var

## Phase 18: Proper Flatpak Bundle Format

Replace the tar-based bundle with the real Flatpak bundle format (an OSTree
commit packed into a single file with a metadata header).

### Tasks

- [ ] Implement OSTree commit object serialization (the reverse of parsing):
  build `(a{sv}aya(say)sstayay)` GVariant from a directory tree
- [ ] Implement dirtree and dirmeta object serialization
- [ ] Implement content object (`.filez`) creation: GVariant header + raw
  deflate compressed content
- [ ] Compute SHA256 checksums for all objects
- [ ] Pack commit + all referenced objects into the Flatpak bundle format:
  a GVariant file containing the commit, metadata, and a map of object
  checksums to object data
- [ ] Update `build-bundle` to produce proper bundles
- [ ] Update `build-import-bundle` to parse proper bundles

## Phase 19: Full OSTree Commit Creation

Implement `build-commit-from` and `build-export` using real OSTree commit
objects instead of simple file copies.

### Tasks

- [ ] Implement `hash_object()` — compute the OSTree checksum for a file,
  dirtree, dirmeta, or commit object
- [ ] Implement `write_object()` — serialize and store an object in the
  local repo
- [ ] Implement `create_dirtree()` — recursively walk a directory, create
  file/dirtree/dirmeta objects, return the root dirtree + dirmeta checksums
- [ ] Implement `create_commit()` — wrap a root tree in a commit object
  with subject, timestamp, and parent commit
- [ ] Update `build-export` to create real OSTree commits
- [ ] Implement `build-commit-from` — read an existing commit's tree and
  create a new commit pointing to the same (or modified) tree

## Phase 20: GPG Commit Signing

Implement `build-sign` to GPG-sign OSTree commits.

### Tasks

- [ ] Implement OSTree commit metadata signature format (detached GPG
  signature stored as a `.commitmeta` object)
- [ ] Shell out to `gpg --detach-sign` to produce the signature
- [ ] Store the signature in the repo's `objects/` directory
- [ ] Update `build-sign` to sign an existing commit
- [ ] Optionally sign during `build-export` with `--gpg-sign=KEYID`

## Phase 21: OSTree Static Deltas

Implement static delta support for much faster pulls from remotes. Without
this, every file is fetched individually, which is very slow for large apps.

### Tasks

- [ ] Parse the delta superblock format (GVariant at
  `<repo>/deltas/<from>-<to>/superblock`)
- [ ] Parse delta part files (`<repo>/deltas/<from>-<to>/<partN>`)
- [ ] Implement the delta instruction set: copy, open, write, set-read-source,
  unset-read-source, close, bspatch
- [ ] Apply deltas to reconstruct objects without fetching them individually
- [ ] Detect available deltas from the summary file's `ostree.static-deltas`
  metadata
- [ ] Fall back to individual object fetching when no delta is available

## Phase 22: HTTP Client Improvements

### Tasks

- [ ] Handle HTTP chunked transfer-encoding (parse chunk headers, reassemble
  body) — some OSTree repos and CDNs use chunked encoding for large objects
- [ ] Add progress reporting during large pulls: track number of objects
  fetched vs. total, bytes downloaded, and print a progress bar to stderr
- [ ] Support HTTP redirects (3xx) in the OSTree fetcher (currently only
  the curl rewrite handles redirects)
- [ ] Connection reuse / keep-alive for fetching many objects from the same
  host (currently opens a new TCP+TLS connection per object)
- [ ] Parallel object fetching (fetch N objects concurrently using threads)

## Phase 23: Auto-Download Missing Extensions

### Tasks

- [ ] When `flatpak run` encounters a missing extension (resolved by
  `extensions.rs` but not found on disk), prompt the user to install it
- [ ] Search configured remotes for the extension ref
- [ ] Pull and install the extension via the OSTree client
- [ ] Re-resolve extensions after installation and continue with the run

## Phase 24: Native D-Bus Wire Protocol Client

Replace `gdbus`/`busctl` subprocess calls with a native Rust D-Bus client
for portal communication.

### Tasks

- [ ] Implement D-Bus wire protocol message serialization/deserialization
  (header fields, body marshalling for basic types: string, uint32, variant,
  array, dict)
- [ ] Implement Unix socket connection and SASL `EXTERNAL` authentication
- [ ] Implement `Hello()` call to get a unique bus name
- [ ] Implement `CallMethod()` — send a method call message and read the reply
- [ ] Replace `gdbus_call()` in `portals.rs` with the native client
- [ ] Replace `busctl` fallback
- [ ] Handle signals (for portal async responses)

## Priority Order (Phases 18-24)

1. **Static deltas** (Phase 21) — without this, installing real apps from
   Flathub is impractically slow (thousands of individual HTTP requests)
2. **HTTP improvements** (Phase 22) — connection reuse and parallel fetching
   make non-delta pulls viable
3. **Auto-download extensions** (Phase 23) — needed for running most real
   apps (GL drivers, codecs)
4. **OSTree commit creation** (Phase 19) — needed for a working build-export
5. **Proper bundle format** (Phase 18) — interoperability with real Flatpak
6. **GPG signing** (Phase 20) — needed for publishing repos
7. **Native D-Bus client** (Phase 24) — removes gdbus/busctl dependency,
   enables async portal interactions
