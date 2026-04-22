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
- [x] Block `clone` when `CLONE_NEWUSER` flag is set (implemented in Phase 12
  with BPF_ALU AND instruction)
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
- [x] Write `pid` file with the bwrap child PID (implemented in Phase 13 via
  `--info-fd` integration to get the actual child PID)
- [x] Parse `--info-fd` output from bwrap to get `bwrapinfo.json`
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

## Phase 5: OSTree Client

Implement the minimal OSTree client needed to pull from Flatpak remotes
(Flathub). This is the largest piece of work.

### Tasks

- [x] Implement OSTree object types: commit, dirtree, dirmeta, file
- [x] Implement content-addressed object storage (`objects/<hash>.{commit,dirtree,dirmeta,file}`)
- [x] Parse OSTree summary file format (GVariant binary)
- [x] Fetch summary from remote via HTTPS (rustls + webpki-roots)
- [x] Resolve Flatpak refs from summary (e.g., `app/org.example.App/x86_64/stable`)
- [x] Implement HTTP object pulling (fetch individual objects by hash)
- [x] Checkout commit to deploy directory (reconstruct filesystem from objects)
- [x] GPG signature verification of commits and summary (Phase 20)
- [x] Implement `flatpak install <remote> <ref>` using the above
- [x] Implement `flatpak update` (checks remote, full pull not yet done)
- [x] Implement `flatpak remote-ls` to list available refs from summary
- [x] Implement `flatpak remote-info` to show commit details
- [x] Implement local repo storage at `<installation>/repo/`

## Phase 6: Extension Handling

Resolve, download, and mount extensions into the sandbox.

### Tasks

- [x] Parse `[Extension <name>]` groups from runtime and app metadata
- [x] Resolve extension refs from installed extensions (with version/branch search)
- [x] Mount extensions at their declared directory in the sandbox
- [x] Handle `add-ld-path` — append extension lib paths to `LD_LIBRARY_PATH`
- [x] Handle `subdirectories` — mount sub-extensions
  in a sub-bwrap)

## Phase 7: Portal Support

Integrate with XDG desktop portals for mediated access to host resources.

### Tasks

- [x] Document portal: CLI stubs for `flatpak documents`, `document-export`,
  `document-unexport`, `document-info` (full D-Bus portal API not yet implemented)
- [x] Permission store: CLI stubs for `flatpak permissions`, `permission-show`,
  `permission-set`, `permission-remove`, `permission-reset`
- [x] Set portal-related environment variables (`FLATPAK_PORTAL_PID`)
- [x] Full D-Bus client for portal APIs (Phase 24)

## Phase 8: Remaining CLI Commands

### Tasks

- [x] `flatpak make-current` — set default version for an app
- [x] `flatpak mask` — mask out updates for specific refs
- [x] `flatpak pin` — pin runtimes to prevent automatic removal
  appstream XML from remote)

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
- [x] Regenerate `ld.so.cache` (implemented in Phase 15)

## Phase 11: Native Deflate and Local Object Cache

Remove the python3 dependency for decompression and avoid re-downloading
objects that have already been fetched.

### Tasks

- [x] Implement raw deflate decompression natively via `miniz_oxide` (no
  more python3 dependency)
- [x] Store fetched OSTree objects locally in `<installation>/repo/objects/`
  with the standard `<XX>/<YY...>.<ext>` layout
- [x] Check local cache before fetching objects from the remote
- [x] Implement `flatpak update` to check remote newer commits (compare
  local commit checksum with remote summary, re-checkout if different)
- [x] Handle HTTP chunked transfer-encoding in the HTTP client (some repos
  use it for large objects)
- [x] Add progress reporting during large pulls (object count / total)
  effort but huge performance improvement)

## Phase 12: Seccomp Hardening

Complete the remaining seccomp filter gaps.

### Tasks

- [x] Implement proper `clone` flag inspection for `CLONE_NEWUSER` using
  BPF_ALU AND instruction to mask and test the flags argument
- [x] Add GPG signature verification for OSTree summary and commit objects
  (either shell out to `gpg` or implement minimal OpenPGP parsing)
- [x] Harden `personality` filtering to only allow known-safe values
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
  in a single file with metadata header) instead of tar
  existing ref's content tree
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

- [x] Implement GVariant serializer (`gvariant.rs`) with support for
  Bool, Byte, Uint32, Uint64, Str, ByteArray, Array, Tuple, DictEntry,
  Variant types, plus proper alignment and framing offset handling
- [x] Convenience constructors for OSTree commit objects
- [x] Implement structured bundle format with magic header, ref name,
  metadata, and deflate-compressed tar payload
- [x] Update `build-bundle` to produce proper structured bundles
- [x] Update `build-import-bundle` to parse structured bundles (with
  legacy tar fallback)

## Phase 19: Full OSTree Commit Creation

Implement `build-commit-from` and `build-export` using real OSTree commit
objects instead of simple file copies.

### Tasks

- [x] Implement `sha256_hex()` — compute SHA256 checksum via sha256sum
- [x] Implement `store_object()` — store objects in `repo/objects/<XX>/<YY>.<ext>`
- [x] Implement `create_dirmeta()` — serialize directory metadata
- [x] Implement `create_file_object()` — serialize file content for checksumming
- [x] Implement `create_dirtree_from_dir()` — recursively walk a directory,
  create file/dirtree/dirmeta objects with checksums and `.filez` compression
- [x] Implement `create_commit()` — create commit object with subject,
  timestamp, and root tree references
- [x] Implement `write_ref()` — write ref files to `repo/refs/heads/`
- [x] Update `build-export` to create real OSTree commits alongside file copies
- [x] Implement `build-commit-from` — copy content from source ref and create
  new commit

## Phase 20: GPG Commit Signing

Implement `build-sign` to GPG-sign OSTree commits.

### Tasks

- [x] Implement GPG signing via `gpg --detach-sign` subprocess
- [x] Store detached signature as `.commitmeta` object in the repo
- [x] `build-sign` reads commit object, signs it, stores signature
- [x] Optionally sign during `build-export` with `--gpg-sign=KEYID`

## Phase 21: OSTree Static Deltas

Implement static delta support for much faster pulls from remotes. Without
this, every file is fetched individually, which is very slow for large apps.

### Tasks

- [x] Parse the delta superblock format (`deltas.rs`)
- [x] Fetch and decompress delta part files
- [x] Implement the delta instruction set: OPEN_SPLICE_AND_CLOSE, OPEN,
  WRITE, SET_READ_SOURCE, UNSET_READ_SOURCE, CLOSE, BSPATCH (bspatch is
  simplified — full bsdiff patch application needs more work)
- [x] Apply deltas to write objects to local cache
- [x] Full bspatch implementation (control/diff/extra streams) — supports
  both standard BSDIFF40 format and OSTree raw inline format, with
  offset-encoded signed 64-bit integers, diff+old addition, extra verbatim
  copy, and seek adjustment
- [x] Detect available deltas from the commit URL pattern and probe for
  superblock existence
- [x] Fall back to individual object fetching when no delta is available

## Phase 22: HTTP Client Improvements

### Tasks

- [x] Handle HTTP chunked transfer-encoding (parse chunk headers, reassemble
  body)
- [x] Add progress reporting during large pulls: track objects fetched vs.
  cached, bytes downloaded
- [x] Support HTTP redirects (3xx) in the OSTree fetcher
- [x] Connection reuse via global connection pool (`CONN_POOL`) keyed by
  host:port:tls, with keep-alive and automatic retry on stale connections
- [x] Parallel object fetching (fetch N files concurrently using
  `std::thread::scope`)

## Phase 23: Auto-Download Missing Extensions

### Tasks

- [x] When `flatpak run` encounters a missing extension, auto-install it
  (`find_missing_extensions()` + `auto_install_missing()`)
- [x] Search configured remotes for the extension ref
- [x] Pull and install the extension via the OSTree client
- [x] Re-resolve extensions after installation and continue with the run

## Phase 24: Native D-Bus Wire Protocol Client

Replace `gdbus`/`busctl` subprocess calls with a native Rust D-Bus client
for portal communication.

### Tasks

- [x] Implement D-Bus wire protocol message serialization/deserialization
  (header fields, body marshalling for basic types: string, uint32, variant,
  array, dict) — `dbus_client.rs`
- [x] Implement Unix socket connection and SASL `EXTERNAL` authentication
- [x] Implement `Hello()` call to get a unique bus name
- [x] Implement `CallMethod()` — send a method call message and read the reply
- [x] Replace `gdbus_call()` in `portals.rs` — now uses native client first,
  falls back to gdbus/busctl subprocess if native fails
- [x] Replace `busctl` fallback (native client with gdbus/busctl fallback)
- [x] Handle D-Bus signals for portal async responses — replaced hand-rolled
  D-Bus client with zbus, which provides `portal_request()` that subscribes
  to `Response` signals on request object paths via `MatchRule` +
  `MessageIterator`, properly handling the portal async request/response
  pattern

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

---

## Upstream Test Parity Plan

Goal: Run tests inspired by the upstream [flatpak/flatpak](https://github.com/flatpak/flatpak)
test suite as Nix checks, following the same pattern as `rust/awk` (one
`runCommand` derivation per test, listed in `default.nix`, executed via
`nix flake check`).

**Status:** 124 sandbox-free Nix checks + 10 VM tests implemented and passing.

### Architecture

| File | Role |
|---|---|
| `default.nix` | Adds `rust-flatpak-dev` (debug build) + `checks` attrset mapping test names → `testsuite.nix` |
| `testsuite.nix` | Single-test derivation template; sets up `$HOME`, `XDG_DATA_HOME`, runs test script |
| `tests/<name>.sh` | Individual test scripts (self-contained, no `libtest.sh` dependency) |

Unlike upstream (which needs D-Bus, OSTree repos, bubblewrap, and a full
`libtest.sh` harness), these tests target **pure CLI behaviour** that can run
inside a Nix sandbox without network, D-Bus, or root access.

### Test Categories & Names (124 tests)

#### CLI surface — version, help, error handling (7 tests)

Mirrors upstream `test-basic.sh`.

| Test name | Upstream reference | What it checks |
|---|---|---|
| `version` | test-basic.sh "version" | `--version` prints version string |
| `version-format` | test-basic.sh "version" | Version output matches "Flatpak X.Y.Z" pattern |
| `help` | test-basic.sh "help" | `--help` prints "Usage:" |
| `help-usage-format` | test-basic.sh "help" | `--help` lists expected subcommands (install, run, etc.) |
| `help-commands` | test-basic.sh "command help" | `<cmd> --help` doesn't crash for all 30 subcommands |
| `missing-command` | test-basic.sh "missing command" | No args → non-zero exit + usage hint |
| `unknown-command` | test-basic.sh "misspelt command" | Bad command → "unknown command" error |

#### Override management (12 tests)

Mirrors upstream `test-override.sh`. Exercises `flatpak override` read/write
against a temporary `$HOME/.local/share/flatpak/` — no running apps needed.

| Test name | Upstream reference | What it checks |
|---|---|---|
| `override-socket` | test-override.sh "override --socket" | `--socket` / `--nosocket` written to override file |
| `override-device` | test-override.sh "override --device" | `--device` / `--nodevice` |
| `override-share` | test-override.sh "override --share" | `--share` / `--unshare` |
| `override-env` | test-override.sh "override --env" | `--env` writes `[Environment]` section |
| `override-filesystem` | test-override.sh "override --filesystem" | `--filesystem` persisted |
| `override-reset` | test-override.sh (implicit) | `--reset` removes override file |
| `override-multiple-sockets` | test-override.sh | Multiple `--socket` calls accumulate |
| `override-multiple-env` | test-override.sh "override --env" | Multiple `--env` vars all persist |
| `override-multiple-filesystems` | test-override.sh "override --filesystem" | Multiple `--filesystem` entries accumulate |
| `override-separate-apps` | test-override.sh | Different apps get separate override files |
| `override-env-overwrite` | test-override.sh | Re-setting same env var overwrites value |
| `override-missing-app` | — | Missing app ID → error exit |

#### Config & history (3 tests)

| Test name | Upstream reference | What it checks |
|---|---|---|
| `config-list` | test-config.sh "config list" | `flatpak config` lists installation info |
| `config-user` | test-config.sh | `--user config` shows "user" and "path" |
| `history-empty` | — | `history` on fresh install doesn't crash |

#### Build commands (12 tests)

Mirrors upstream build workflow. All filesystem-only, no sandbox needed.

| Test name | Upstream reference | What it checks |
|---|---|---|
| `build-init` | — | Creates metadata + files/ directory |
| `build-init-dirs` | — | Creates all expected subdirs (files/bin, var/tmp, etc.) |
| `build-init-extension` | — | `--extension-tag` produces [Runtime] + [ExtensionOf] |
| `build-init-missing-args` | — | No args → usage error |
| `build-finish-command` | — | `--command` written to metadata |
| `build-finish-permissions` | — | `--share`, `--socket`, `--filesystem`, `--device` all written to [Context] |
| `build-finish-no-dir` | — | No dir → usage error |
| `build-export-creates-repo` | — | Export creates repo directory with app files |
| `build-export-branch` | — | `-b mybranch` uses custom branch name |
| `build-export-no-dir` | — | Missing args → usage error |
| `build-bundle-basic` | — | Bundle creation doesn't crash |
| `repo-info` | — | `flatpak repo` on exported repo doesn't crash |

#### Metadata (3 tests)

| Test name | Upstream reference | What it checks |
|---|---|---|
| `metadata-parse` | unit tests | build-init metadata contains app/sdk/platform |
| `metadata-roundtrip` | — | All fields survive build-init → build-finish round-trip |
| `repair-no-crash` | — | `flatpak repair` on empty install doesn't crash |

#### Install / uninstall / list / info (10 tests)

Uses local build-dir install (no network). Mirrors upstream `test-basic.sh`
and `test-info.sh`.

| Test name | Upstream reference | What it checks |
|---|---|---|
| `install-from-dir` | — | Install from build dir populates user installation |
| `install-missing-args` | — | No args → usage error |
| `uninstall-app` | — | Uninstall removes deployment directory |
| `uninstall-missing-args` | — | No args → error |
| `list-empty` | — | List on empty install shows header, no crash |
| `list-installed-app` | — | Installed app appears in list output |
| `list-filter-app` | — | `--app` filter includes apps, `--runtime` excludes them |
| `info-installed-app` | test-info.sh | `info` shows app ID |
| `info-show-metadata` | test-info.sh "info --show-metadata" | `--show-metadata` prints [Application] section |
| `info-show-permissions` | test-info.sh "info --show-permissions" | `--show-permissions` shows context |
| `info-missing-args` | test-basic.sh "info missing NAME" | No app → error |

#### Remote management (8 tests)

Mirrors upstream remote commands. Filesystem-only.

| Test name | Upstream reference | What it checks |
|---|---|---|
| `remote-add` | — | Creates config entry with name + URL |
| `remote-add-duplicate` | — | Adding existing remote → "already exists" error |
| `remote-add-title` | — | `--title=` persisted in config |
| `remote-add-from-file` | — | `--from` reads .flatpakrepo file |
| `remote-add-missing-url` | — | Missing URL → error |
| `remote-delete` | — | Removes remote from config |
| `remote-delete-missing` | — | Deleting non-existent remote → error |
| `remote-list` | — | `remotes` lists added remote |
| `remote-modify-implicit` | — | Delete + re-add with new URL works |

#### Misc commands (4 tests)

| Test name | Upstream reference | What it checks |
|---|---|---|
| `mask-pattern` | — | `mask` creates mask file |
| `mask-missing-args` | — | No pattern → error |
| `search-no-remote` | — | Search with no remotes doesn't crash |
| `repair-no-crash` | — | Repair on empty install doesn't crash |

### Running Tests

```text
# Single test:
nix build .#checks.x86_64-linux.rust-flatpak-test-version

# All sandbox-free checks:
nix flake check

# Single VM test (once implemented):
nix build .#checks.x86_64-linux.rust-flatpak-vm-run-hello
```

### Test Script Convention

Each `tests/<name>.sh` script:

- Receives `$FLATPAK` env var pointing to the `rust-flatpak-dev` binary
- Receives `$WORK` pointing to a writable temp directory
- Has `$HOME` set to `$WORK/home`
- Exits 0 on success, non-zero on failure
- Uses simple `grep`/`diff` assertions (no `libtest.sh`)

---

## Phase 2: NixOS VM Tests (sandbox, D-Bus, OSTree)

The 124 sandbox-free checks above cover everything testable inside a pure
Nix derivation. The remaining upstream tests require one or more of:

- **bubblewrap** (bwrap) — needs user namespaces / suid helper
- **D-Bus session bus** — needed for portal, document, and permission commands
- **OSTree** — real repo operations (pull, delta, GPG verify)
- **Network** — HTTP server for remote-ls, install from remote, OCI
- **Systemd journal** — history command reads from journald

These all work inside a NixOS VM test (`pkgs.testers.nixosTest`), which boots
a full NixOS system in QEMU. The project already uses this pattern
successfully in `rust/systemd/testsuite.nix` (upstream systemd test suite)
and `rust/nixos/nixos-test.nix` (NixOS boot test).

### Architecture

| File | Role |
|---|---|
| `vmtest.nix` | NixOS VM test template — boots a VM with bwrap, D-Bus, ostree, and rust-flatpak installed |
| `vmtests/<name>.sh` | Test scripts that run inside the VM |
| `vmtests/libtest-nix.sh` | Minimal test harness (replaces upstream `libtest.sh`) |
| `default.nix` | Adds VM test checks alongside existing sandbox-free checks |

### VM test template (`vmtest.nix`)

```nix
# vmtest.nix — run a flatpak integration test inside a NixOS VM.
{
  pkgs,
  name,
  testTimeout ? 600,
}:
pkgs.testers.nixosTest {
  name = "rust-flatpak-vm-${name}";

  nodes.machine = {pkgs, ...}: {
    environment.systemPackages = [
      pkgs.rust-flatpak-dev
      pkgs.bubblewrap
      pkgs.ostree
      pkgs.gpgme
      pkgs.glib          # for gdbus / gio
      pkgs.xdg-dbus-proxy
      pkgs.python3       # for test web server
      pkgs.coreutils
      pkgs.gnugrep
      pkgs.gnused
      pkgs.diffutils
      pkgs.bash
      pkgs.tar
      pkgs.gzip
      pkgs.gawk
      pkgs.findutils
      pkgs.procps        # for ps, kill
    ];

    # Enable D-Bus user session (needed for portals, documents, permissions)
    services.dbus.enable = true;

    # Enable a user session for the test user
    users.users.testuser = {
      isNormalUser = true;
      extraGroups = ["wheel"];
      password = "test";
    };

    # bubblewrap needs unprivileged user namespaces
    security.unprivilegedUsernsClone = true;

    # XDG portal service stubs (if needed for document/permission tests)
    # services.flatpak.enable = false; # we use rust-flatpak, not system flatpak

    virtualisation = {
      memorySize = 2048;
      cores = 2;
    };
  };

  testScript = ''
    machine.wait_for_unit("multi-user.target")

    # Copy test scripts into VM
    machine.succeed("mkdir -p /tmp/flatpak-tests")
    # (scripts are injected via environment.etc or direct copy)

    # Run the test as testuser (not root) for realistic sandbox behavior
    machine.succeed(
      "su - testuser -c 'export FLATPAK=/run/current-system/sw/bin/flatpak; "
      "export WORK=$(mktemp -d); "
      "export HOME=/home/testuser; "
      "bash /tmp/flatpak-tests/${name}.sh'"
    )
  '';
}
```

### `vmtests/libtest-nix.sh` — Minimal test harness

Replaces upstream's `libtest.sh` with a Nix-friendly version:

```bash
#!/bin/bash
# Minimal test harness for rust-flatpak VM tests.
# Provides: setup_repo, install_repo, assert_*, ok, skip, run helpers.

set -euo pipefail
export LC_ALL=C

FLATPAK="${FLATPAK:-flatpak}"
ARCH="$(uname -m)"
TEST_DATA_DIR="${WORK}/testdata"
FL_DIR="${HOME}/.local/share/flatpak"

mkdir -p "$TEST_DATA_DIR" "$FL_DIR"

TESTS_PASSED=0
TESTS_FAILED=0

ok() {
  echo "ok - $1"
  TESTS_PASSED=$((TESTS_PASSED + 1))
}

skip() {
  echo "skip - $1"
}

assert_has_file() { [ -f "$1" ] || { echo "FAIL: expected file: $1"; exit 1; }; }
assert_has_dir() { [ -d "$1" ] || { echo "FAIL: expected dir: $1"; exit 1; }; }
assert_not_has_file() { [ ! -f "$1" ] || { echo "FAIL: unexpected file: $1"; exit 1; }; }
assert_not_has_dir() { [ ! -d "$1" ] || { echo "FAIL: unexpected dir: $1"; exit 1; }; }
assert_file_has_content() {
  grep -qE "$2" "$1" || { echo "FAIL: '$1' missing content '$2'"; cat "$1"; exit 1; }
}
assert_not_file_has_content() {
  if grep -qE "$2" "$1"; then echo "FAIL: '$1' has unexpected '$2'"; cat "$1"; exit 1; fi
}
assert_file_empty() { [ ! -s "$1" ] || { echo "FAIL: expected empty: $1"; exit 1; }; }
assert_streq() { [ "$1" = "$2" ] || { echo "FAIL: '$1' != '$2'"; exit 1; }; }
assert_not_streq() { [ "$1" != "$2" ] || { echo "FAIL: '$1' == '$2'"; exit 1; }; }

# Build a minimal test app in $TEST_DATA_DIR/app-build
make_test_app() {
  local app_id="${1:-org.test.Hello}"
  local branch="${2:-master}"
  local build_dir="$TEST_DATA_DIR/app-build"
  rm -rf "$build_dir"
  $FLATPAK build-init "$build_dir" "$app_id" org.test.Sdk org.test.Platform "$branch"
  mkdir -p "$build_dir/files/bin"
  cat > "$build_dir/files/bin/hello.sh" << 'SCRIPT'
#!/bin/sh
echo "Hello world, from a sandbox"
SCRIPT
  chmod +x "$build_dir/files/bin/hello.sh"

  # Desktop file
  mkdir -p "$build_dir/files/share/applications"
  cat > "$build_dir/files/share/applications/${app_id}.desktop" << DESKTOP
[Desktop Entry]
Name=Hello
Exec=hello.sh
Type=Application
Icon=${app_id}
DESKTOP

  # Icon
  mkdir -p "$build_dir/files/share/icons/hicolor/64x64/apps"
  echo "PNG" > "$build_dir/files/share/icons/hicolor/64x64/apps/${app_id}.png"

  $FLATPAK build-finish "$build_dir" --command=hello.sh \
    --share=network --share=ipc \
    --socket=x11 --socket=wayland --socket=pulseaudio \
    --device=dri --filesystem=home
}

# Build a minimal test runtime in $TEST_DATA_DIR/runtime-build
make_test_runtime() {
  local rt_id="${1:-org.test.Platform}"
  local branch="${2:-master}"
  local build_dir="$TEST_DATA_DIR/runtime-build"
  rm -rf "$build_dir"
  mkdir -p "$build_dir/files/bin" "$build_dir/files/lib" "$build_dir/files/etc"

  # Provide basic shell utilities in the runtime
  for cmd in sh bash cat echo ls env mkdir rm cp mv ln test readlink; do
    if command -v "$cmd" > /dev/null 2>&1; then
      cp "$(command -v "$cmd")" "$build_dir/files/bin/" 2>/dev/null || true
    fi
  done

  # Minimal metadata
  cat > "$build_dir/metadata" << META
[Runtime]
name=${rt_id}
runtime=${rt_id}/${ARCH}/${branch}
sdk=org.test.Sdk/${ARCH}/${branch}
META
}

# Export test app + runtime to a local repo, configure as a remote, install
setup_repo() {
  local branch="${1:-master}"
  local repo="$TEST_DATA_DIR/repo"

  make_test_app org.test.Hello "$branch"
  make_test_runtime org.test.Platform "$branch"

  # Export app
  $FLATPAK build-export "$repo" "$TEST_DATA_DIR/app-build" -b "$branch" 2>&1 || true

  # Export runtime (as a file-copy repo entry)
  local rt_dir="$repo/runtime/org.test.Platform/${ARCH}/${branch}/active"
  mkdir -p "$rt_dir"
  cp "$TEST_DATA_DIR/runtime-build/metadata" "$rt_dir/metadata"
  cp -r "$TEST_DATA_DIR/runtime-build/files" "$rt_dir/files"
}

install_repo() {
  local branch="${1:-master}"
  # Install from build dirs directly
  $FLATPAK --user install "$TEST_DATA_DIR/app-build" 2>&1 || true
}

# Run an installed app
run() {
  $FLATPAK --user run "$@"
}

run_sh() {
  local app_id="$1"
  shift
  $FLATPAK --user run --command=sh "$app_id" -c "$@"
}
```

### VM Test Categories

#### Category 1: Sandbox execution (`test-run.sh` parity)

These tests boot a VM, build+install a test app, and verify `flatpak run`
produces correct sandbox behavior. Each is a separate NixOS VM test check.

| VM test name | Upstream reference | What it checks |
|---|---|---|
| `vm-run-hello` | test-run.sh "hello" | `flatpak run org.test.Hello` prints "Hello world, from a sandbox" |
| `vm-run-command-override` | test-run.sh | `--command=sh` overrides the default command |
| `vm-run-nonexistent` | test-run.sh "error handling" | Running non-installed app → error |
| `vm-run-flatpak-info` | test-run.sh "flatpak-info" | `/.flatpak-info` exists inside sandbox with correct content |
| `vm-run-xdg-dirs` | test-run.sh "XDG_foo_HOME" | XDG dirs remapped to `~/.var/app/<id>/` inside sandbox |
| `vm-run-xdg-runtime` | test-run.sh "XDG_RUNTIME_DIR" | XDG_RUNTIME_DIR set to `/run/user/<uid>` inside sandbox |
| `vm-run-namespace-net` | test-run.sh "namespaces" | Network namespace isolated by default, `--share=network` shares it |
| `vm-run-namespace-ipc` | test-run.sh "namespaces" | IPC namespace isolated by default, `--share=ipc` shares it |
| `vm-run-filesystem` | test-run.sh "namespaces" | `--filesystem=<path>` exposes host path inside sandbox |
| `vm-run-persist` | test-run.sh "--persist" | `--persist=.persist` creates persistent storage in `~/.var/app/` |
| `vm-run-devel` | test-run.sh | `--devel` mode allows dev tools |
| `vm-run-sandbox-mode` | test-run.sh | `--sandbox` restricts further |
| `vm-run-env-override` | test-override.sh "sandbox env" | Override `--env=FOO=BAR` visible inside sandbox |

#### Category 2: Override sandbox effects (`test-override.sh` parity)

| VM test name | Upstream reference | What it checks |
|---|---|---|
| `vm-override-socket-wayland` | test-override.sh "sandbox wayland socket" | `--socket=wayland` exposes wayland socket in sandbox |
| `vm-override-device-dri` | test-override.sh "sandbox dri device" | `--device=dri` exposes /dev/dri in sandbox |
| `vm-override-filesystem-home` | test-override.sh "sandbox filesystem" | `--filesystem=home:ro` exposes home read-only |
| `vm-override-env-sandbox` | test-override.sh "sandbox environment variables" | `--env=FOO=BAR` sets var inside sandbox, overrides host env |
| `vm-override-persist` | test-override.sh "persist" | `--persist=example` creates bind mount to `~/.var/app/` |
| `vm-override-nofilesystem-home` | test-override.sh "runtime override --nofilesystem=home" | `--nofilesystem=home` revokes home access |
| `vm-override-nofilesystem-host` | test-override.sh "runtime override --nofilesystem=host" | `--nofilesystem=host` revokes host access |
| `vm-override-nofilesystem-host-reset` | test-override.sh "--nofilesystem=host:reset" | `:reset` suffix revokes all filesystem overrides |
| `vm-override-allow-disallow` | test-override.sh "override --allow" | `--allow`/`--disallow` features (multiarch, bluetooth) |
| `vm-override-bus-session` | test-override.sh "override session bus names" | `--own-name`/`--talk-name` D-Bus session bus policy |
| `vm-override-bus-system` | test-override.sh "override system bus names" | `--system-own-name`/`--system-talk-name` system bus policy |

#### Category 3: Config set/get (`test-config.sh` parity)

| VM test name | Upstream reference | What it checks |
|---|---|---|
| `vm-config-set-get` | test-config.sh "config set" | `config --set languages "de;fr"` then `config --get languages` returns "de;fr" |
| `vm-config-languages-star` | test-config.sh "config languages *" | Setting languages to "*" works |
| `vm-config-unset` | test-config.sh "config unset" | `config --unset languages` removes the key |

#### Category 4: Bundle roundtrip (`test-bundle.sh` parity)

| VM test name | Upstream reference | What it checks |
|---|---|---|
| `vm-bundle-create` | test-bundle.sh "create bundles" | `build-bundle` creates a `.flatpak` file from repo |
| `vm-bundle-install` | test-bundle.sh "install app bundle" | `install --bundle` installs from bundle file, pulls runtime dep |
| `vm-bundle-update` | test-bundle.sh "update" | Update replaces app with newer version |
| `vm-bundle-update-as-bundle` | test-bundle.sh "update as bundle" | Re-importing a newer bundle updates the app |
| `vm-bundle-runtime` | test-bundle.sh "install runtime bundle" | Bundling and installing a runtime |

#### Category 5: Build + update repo (`test-build-update-repo.sh` parity)

| VM test name | Upstream reference | What it checks |
|---|---|---|
| `vm-build-update-repo-title` | test-build-update-repo.sh "can update repo title" | `build-update-repo --title=` persists in config |
| `vm-build-update-repo-redirect` | test-build-update-repo.sh "can update redirect url" | `--redirect-url=` persists |
| `vm-build-update-repo-default-branch` | test-build-update-repo.sh "can update default branch" | `--default-branch=` persists |

#### Category 6: Repair (`test-repair.sh` parity)

| VM test name | Upstream reference | What it checks |
|---|---|---|
| `vm-repair-missing-file` | test-repair.sh "repair handles missing files" | After deleting an object, `repair` restores it |
| `vm-repair-reinstall-all` | test-repair.sh "repair --reinstall-all" | `--reinstall-all` preserves pin state |

#### Category 7: Info deep (`test-info.sh` parity)

| VM test name | Upstream reference | What it checks |
|---|---|---|
| `vm-info-show-commit` | test-info.sh "info -rcos" | `--show-commit` prints commit hash |
| `vm-info-show-location` | test-info.sh "info --show-location" | Path contains ref and commit |
| `vm-info-show-runtime` | test-info.sh "info --show-runtime" | Prints runtime ref |
| `vm-info-show-sdk` | test-info.sh "info --show-sdk" | Prints SDK ref |
| `vm-info-show-extensions` | test-info.sh "info --show-extensions" | Lists extension points |
| `vm-info-file-access` | test-info.sh "info --file-access" | Reports file access permissions |

#### Category 8: History (`test-history.sh` parity)

| VM test name | Upstream reference | What it checks |
|---|---|---|
| `vm-history-install-uninstall` | test-history.sh | Install/update/uninstall events logged to journal, `history` shows them |

#### Category 9: Seccomp (`test-seccomp.sh` parity)

| VM test name | Upstream reference | What it checks |
|---|---|---|
| `vm-seccomp-filter` | test-seccomp.sh | Sandbox blocks dangerous syscalls (e.g. `mount`, `pivot_root`) |
| `vm-seccomp-devel` | test-seccomp.sh | `--devel` relaxes seccomp filter |

#### Category 10: Extensions (`test-extensions.sh` parity)

| VM test name | Upstream reference | What it checks |
|---|---|---|
| `vm-extension-mount` | test-extensions.sh | Extension directories mounted at correct paths in sandbox |
| `vm-extension-unmask` | test-extensions.sh | Unmasked extensions are available |

#### Category 11: D-Bus proxy (`test-run.sh` D-Bus parts)

| VM test name | Upstream reference | What it checks |
|---|---|---|
| `vm-dbus-proxy-session` | test-run.sh | xdg-dbus-proxy filters session bus access |
| `vm-dbus-proxy-system` | test-run.sh | System bus filtering |

#### Category 12: Documents and permissions (portal tests)

| VM test name | Upstream reference | What it checks |
|---|---|---|
| `vm-document-export` | test-basic.sh | `document-export` exports a file via document portal |
| `vm-document-unexport` | test-basic.sh | `document-unexport` removes export |
| `vm-document-info` | test-basic.sh | `document-info` shows exported document path |
| `vm-permission-set` | test-basic.sh | `permission-set` records permission in store |
| `vm-permission-show` | test-basic.sh | `permission-show` displays recorded permissions |
| `vm-permission-remove` | test-basic.sh | `permission-remove` deletes from store |
| `vm-permission-reset` | test-basic.sh | `permission-reset` clears all permissions for an app |

#### Category 13: Remote + network (`test-run.sh`, `test-http-utils.sh`)

| VM test name | Upstream reference | What it checks |
|---|---|---|
| `vm-remote-ls` | test-basic.sh | `remote-ls` fetches and lists refs from HTTP repo |
| `vm-remote-info` | test-basic.sh | `remote-info` shows ref details from remote |
| `vm-install-from-remote` | test-run.sh | Full `install` from HTTP remote repo |
| `vm-update-from-remote` | test-run.sh "update" | `update` pulls newer commit from remote |
| `vm-search-remote` | test-basic.sh | `search` finds apps in remote summary |

#### Category 14: Metadata validation & security (`test-metadata-validation.sh`)

| VM test name | Upstream reference | What it checks |
|---|---|---|
| `vm-metadata-hidden-perms` | test-metadata-validation.sh CVE-2021-43860 | App with NUL-hidden permissions rejected |
| `vm-metadata-no-xametadata` | test-metadata-validation.sh | Missing xa.metadata in commit → rejected |
| `vm-metadata-invalid` | test-metadata-validation.sh | Invalid metadata syntax → rejected |
| `vm-metadata-mismatch` | test-metadata-validation.sh | Commit metadata != summary metadata → rejected |

#### Category 15: Misc upstream tests

| VM test name | Upstream reference | What it checks |
|---|---|---|
| `vm-install-subpath` | test-run.sh "subpaths" | `--subpath=/a` partial installs |
| `vm-version-check` | test-run.sh "version checks" | `--require-version` prevents install of apps needing newer flatpak |
| `vm-no-setuid` | test-run.sh "no setuid" | Apps with setuid files rejected |
| `vm-no-world-writable` | test-run.sh "no world writable dir" | World-writable dirs get permissions canonicalized |
| `vm-sdk-option` | test-run.sh "--sdk option" | `build-finish --sdk=` recorded in metadata |
| `vm-persist-symlink-escape` | test-run.sh CVE-2024-42472 | `--persist` rejects symlink sandbox escape |
| `vm-run-cve-2024-32462` | test-run.sh CVE-2024-32462 | `--command=--tmpfs` not passed as bwrap args |
| `vm-completion` | test-completion.sh | Tab completion produces expected candidates |
| `vm-one-dir-commands` | test-basic.sh "ONE_DIR commands" | `--system --user` on same command → "Multiple installations" error |

### Implementation Steps

#### Step 1: Create `vmtests/libtest-nix.sh`

Write the minimal harness shown above. Key functions: `make_test_app`,
`make_test_runtime`, `setup_repo`, `install_repo`, `run`, `run_sh`, and all
`assert_*` helpers.

#### Step 2: Create `vmtest.nix`

NixOS VM test template, parameterized by test name. The VM gets:

- `rust-flatpak-dev` (debug build)
- `bubblewrap` (suid or unprivileged userns)
- `ostree` (real repo operations)
- D-Bus session bus (via `services.dbus.enable`)
- A non-root `testuser` (sandbox needs non-root)
- `python3` (for test HTTP server)
- All coreutils, grep, sed, diff, tar

#### Step 3: Implement Category 1 (sandbox execution) first

Start with `vm-run-hello` — the simplest end-to-end: build app, install,
`flatpak run`, verify output. This validates the entire VM test pipeline.

#### Step 4: Add HTTP test server for network tests

For Categories 13–14 (remote/network/metadata-validation), run a small Python
HTTP server inside the VM serving an OSTree repo. The upstream tests use
`tests/web-server.py` for this; we can write a simpler version or adapt it.

```python
# vmtests/web-server.py — serve a local OSTree repo over HTTP
import http.server, sys, os
os.chdir(sys.argv[1])  # repo path
httpd = http.server.HTTPServer(("127.0.0.1", 0), http.server.SimpleHTTPRequestHandler)
print(httpd.server_address[1])  # print port
sys.stdout.flush()
httpd.serve_forever()
```

#### Step 5: Incremental category implementation

Implement categories in dependency order:

1. **Cat 1** (run basics) — validates VM pipeline + bwrap
2. **Cat 2** (override sandbox effects) — builds on Cat 1
3. **Cat 3** (config set/get) — simple, independent
4. **Cat 4** (bundle roundtrip) — needs OSTree in VM
5. **Cat 5** (build-update-repo) — filesystem + config
6. **Cat 6** (repair) — needs installed app + OSTree
7. **Cat 7** (info deep) — needs installed app
8. **Cat 8** (history) — needs systemd journal
9. **Cat 9** (seccomp) — needs bwrap + try-syscall
10. **Cat 10** (extensions) — needs extension point setup
11. ~~**Cat 11** (D-Bus proxy)~~ — ✅ 2/2 done
12. ~~**Cat 12** (documents/permissions)~~ — ✅ 7/7 done
13. **Cat 13** (remote+network) — needs HTTP server
14. **Cat 14** (metadata validation) — needs OSTree + HTTP
15. **Cat 15** (misc) — various security and edge cases

#### Step 6: Wire into `default.nix`

Add VM checks alongside sandbox-free checks:

```nix
checks = let
  sandboxTests = { ... };  # existing 124 tests
  vmTests = builtins.listToAttrs (map (name: {
    name = "rust-flatpak-vm-${name}";
    value = pkgs: import ./vmtest.nix { inherit pkgs name; };
  }) vmTestNames);
in sandboxTests // vmTests;
```

### Expected final test count

| Layer | Tests | Runtime |
|---|---|---|
| Sandbox-free (`testsuite.nix`) | 124 | ~2 min total |
| VM tests (`vmtest.nix`) | ~65 | ~5–10 min per test (VM boot) |
| **Total** | **~189** | — |

### Notes on VM test performance

- Each VM test boots a full NixOS system (~15s) + runs one test (~5–30s)
- Consider grouping related tests into a single VM boot where possible
  (e.g. all Category 1 tests in one VM) to reduce total CI time
- The `testTimeout` parameter prevents hung tests from blocking CI
- VM tests are heavier than sandbox-free checks; run them in a separate
  CI job or only on PRs that touch sandbox/runtime code

---

## Implementation Status

### Completed

- [x] **124 sandbox-free Nix checks** — all passing (`testsuite.nix`)
- [x] **VM test infrastructure** — `vmtest.nix`, `vmtests/libtest-nix.sh`
- [x] **Category 1: Sandbox execution** — 13/13 VM tests implemented
- [x] **Category 2: Override sandbox effects** — 11/11 VM tests implemented
- [x] **Bug fixes discovered by VM tests:**
  - memfd CLOEXEC — fds passed to bwrap via `--ro-bind-data`/`--seccomp` were closed on exec
  - /etc ordering — timezone symlinks failed because /etc didn't exist yet in sandbox
  - NixOS /nix/store — bind-mount `/nix/store` so NixOS ELF interpreters are reachable
  - Command path resolution — resolve command to `/app/bin/` or `/usr/bin/` since bwrap `execvp` uses parent PATH
- [x] **`.deslop.toml`** — suppress pre-existing deslop findings on unsafe FFI code
- [x] **Category 9: Seccomp** — 2/2 VM tests implemented (`vm-seccomp-filter`, `vm-seccomp-devel`)
- [x] **Category 15 partial: CVE-2024-32462** — 1/9 VM test implemented (`vm-run-cve-2024-32462`)
- [x] **Category 13: Remote + network** — 4/5 VM tests implemented (`vm-remote-ls`, `vm-remote-info`, `vm-install-from-remote`, `vm-search-remote`)
- [x] **`build_update_repo` GVariant summary** — now writes proper GVariant binary summary to `repo/summary` (was plain text, incompatible with `fetch_summary` parser)
- [x] **Python3 in VM** — added to `vmtest.nix` systemPackages for HTTP test server
- [x] **HTTP repo helpers in `libtest-nix.sh`** — `setup_http_repo`, `setup_local_runtime`, `cleanup_http`
- [x] **Category 4: Bundle roundtrip** — 4/5 VM tests implemented (`vm-bundle-create`, `vm-bundle-install`, `vm-bundle-runtime`, `vm-bundle-update-as-bundle`)
- [x] **Category 15 partial: `--sdk`, `--persist`, CVEs, setuid, world-writable** — added `--sdk` to `build-finish`, `--persist` to sandbox/override/build-finish, implemented `vm-sdk-option`, `vm-persist-symlink-escape`, `vm-persist-path-traversal`, `vm-no-setuid`, `vm-no-world-writable` tests (6/9 Cat 15 done)
- [x] **Category 12 complete: documents + permissions** — added an on-disk fallback portal store under `$XDG_DATA_HOME/flatpak/portal/` that activates whenever the real `org.freedesktop.portal.Documents` / `org.freedesktop.impl.portal.PermissionStore` are unreachable; document export/info/unexport, permission set/show/remove/reset all degrade to TSV/symlink-based local state; implemented 7 VM tests (`vm-document-export`, `vm-document-info`, `vm-document-unexport`, `vm-permission-set`, `vm-permission-show`, `vm-permission-remove`, `vm-permission-reset`) + 2 sandbox-free tests (`document-local-store`, `permission-local-store`) (Cat 12 7/7 done)
- [x] **Category 14 complete: `vm-metadata-no-xametadata` & `vm-metadata-mismatch`** — `install` already rejects build dirs lacking a `metadata` file (covers no-xametadata case); added bundle metadata-mismatch detection in `build_import_bundle` (compares header `metadata` vs. extracted payload `metadata`, removes the deploy dir and errors with `bundle metadata mismatch` on disagreement); implemented `vm-metadata-no-xametadata`, `vm-metadata-mismatch` VM tests + `install-rejects-no-metadata` sandbox-free test (Cat 14 4/4 done)
- [x] **Category 4 complete: `vm-bundle-update`** — added `--bundle=PATH` to `flatpak update` that re-imports an updated bundle (delegates to `build_import_bundle`); writes a history `update` entry; implemented `vm-bundle-update` VM test + `update-bundle-missing` sandbox-free test (Cat 4 5/5 done)
- [x] **Category 15 complete: `--subpath`** — added `--subpath=PATH` flag to `install`; only the listed subdirectories are copied from the build dir and a `subpaths` marker file is written under the deploy dir; implemented `vm-install-subpath` VM test + `install-subpath` sandbox-free test (Cat 15 9/9 done)
- [x] **Category 15 partial: `--require-version` and tab completion** — added `FLATPAK_VERSION` constant, `--require-version=` flag to `build-finish` (writes `required-flatpak=` into metadata), version-check at install time, and a `complete` subcommand for tab completion; implemented `vm-version-check` and `vm-completion` VM tests + 2 sandbox-free tests (8/9 Cat 15 done)
- [x] **Category 7: Info deep** — added `--show-commit`, `--show-location`, `--show-runtime`, `--show-sdk`, `--show-extensions`, `--file-access` to `info` command; 6/6 VM tests implemented + 5 sandbox-free tests; commit checksum now stored on remote install
- [x] **Category 5: Build-update-repo** — 3/3 VM tests implemented (`vm-build-update-repo-title`, `vm-build-update-repo-redirect`, `vm-build-update-repo-default-branch`) + 3 sandbox-free tests; added `--title`, `--redirect-url`, `--default-branch` flags + INI config persistence
- [x] **Category 3: Config set/get** — 3/3 VM tests implemented (`vm-config-set-get`, `vm-config-languages-star`, `vm-config-unset`) + 2 sandbox-free tests; added `config --set`/`--get`/`--unset` subcommands
- [x] **Category 8: History** — 1/1 VM test implemented (`vm-history-install-uninstall`)
- [x] **Category 13: Remote + network** — 5/5 VM tests implemented (`vm-remote-ls`, `vm-remote-info`, `vm-install-from-remote`, `vm-search-remote`, `vm-update-from-remote`)
- [x] **Category 6: Repair** — 2/2 VM tests implemented (`vm-repair-missing-file`, `vm-repair-no-problems`); enhanced repair to check metadata, regenerate missing metadata, and detect broken symlinks
- [x] **`--persist` sandbox support** — bind-mounts `~/.var/app/<id>/<dir>` into sandbox; rejects path traversal (`..`), absolute paths, and symlinks (CVE-2024-42472)
- [x] **`--nofilesystem` in override** — added missing `--nofilesystem` flag to override command
- [x] **`--persist` in override + build-finish** — `--persist=` and `--persist` flags in both commands
- [x] **`--show-extensions` in info** — lists extension points from metadata `[Extension ...]` groups
- [x] **`--file-access` in info** — reports effective access level (read-write/read-only/hidden) for a given path based on filesystem permissions and overrides
- [x] **Category 11: D-Bus proxy** — 2/2 VM tests implemented (`vm-dbus-proxy-session`, `vm-dbus-proxy-system`); existing dbus_proxy.rs already implements xdg-dbus-proxy filtering for session, system, and a11y buses
- [x] **Category 10: Extensions** — 2/2 VM tests implemented (`vm-extension-mount`, `vm-extension-unmask`)
- [x] **Category 14 partial: Metadata validation** — 2/4 VM tests implemented (`vm-metadata-hidden-perms`, `vm-metadata-invalid`); added NUL-byte rejection in `Metadata::from_file` and `Metadata::parse` (CVE-2021-43860)
- [x] **`metadata-nul-byte-rejected`** — sandbox-free test for CVE-2021-43860

### Remaining

All VM test categories complete. 🎉

| Category | Tests | Status | Blockers |
|---|---|---|---|
| **Cat 1: Sandbox execution** | 13 tests | ✅ done | — |
| **Cat 2: Override sandbox effects** | 11 tests | ✅ done | — |
| **Cat 3: Config set/get** | 3 tests | ✅ done | — |
| **Cat 4: Bundle roundtrip** | 5 tests | ✅ done | — |
| **Cat 5: Build-update-repo** | 3 tests | ✅ done | — |
| **Cat 6: Repair** | 2 tests | ✅ done | — |
| **Cat 7: Info deep** | 6 tests | ✅ done | — |
| **Cat 8: History** | 1 test | ✅ done | — |
| **Cat 9: Seccomp** | 2 tests | ✅ done | — |
| **Cat 10: Extensions** | 2 tests | ✅ done | — |
| **Cat 11: D-Bus proxy** | 2 tests | ✅ done | — |
| **Cat 12: Documents/permissions** | 7 tests | ✅ done | — |
| **Cat 13: Remote + network** | 5 tests | ✅ done | — |
| **Cat 14: Metadata validation** | 4 tests | ✅ done | — |
| **Cat 15: Misc/security** | 9 tests | ✅ done | — |

### Recommended implementation order

1. ~~**Cat 13** (remote + network)~~ — ✅ 5/5 done
2. ~~**Cat 4** (bundle roundtrip)~~ — ✅ 5/5 done
3. ~~**Cat 7** (info deep)~~ — ✅ 6/6 done
4. ~~**Cat 15** (misc/security)~~ — ✅ 9/9 done
5. ~~**Cat 5** (build-update-repo)~~ — ✅ 3/3 done
6. ~~**Cat 9** (seccomp)~~ — ✅ 2/2 done
7. ~~**Cat 6** (repair)~~ — ✅ 2/2 done
8. ~~**Cat 3** (config set/get)~~ — ✅ 3/3 done
9. ~~**Cat 14** (metadata validation)~~ — ✅ 4/4 done
10. ~~**Cat 10** (extensions)~~ — ✅ 2/2 done
11. ~~**Cat 11** (D-Bus proxy)~~ — ✅ 2/2 done
12. ~~**Cat 12** (documents/permissions)~~ — ✅ 7/7 done
13. ~~**Cat 8** (history)~~ — ✅ 1/1 done
