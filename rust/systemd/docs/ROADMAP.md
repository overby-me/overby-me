# Roadmap

## Where the port stands

NixOS boots with rust-systemd as PID 1 and reaches `multi-user.target` with a login
prompt in roughly 6 seconds under cloud-hypervisor, with networking via networkd and
resolved.

93 crates, about 370,000 lines of Rust, about 9,700 unit test functions.

The original six-phase port plan is finished as a structuring device: every phase has
shipped its components. What is left is not "phase 6", it is three concrete lists:

1. **[TEST-OVERRIDES.md](TEST-OVERRIDES.md)** is the functional gap list. 40 of the 440
   registered integration tests still carry an override. That file says what each one
   needs.
2. **[ARCHITECTURE.md](ARCHITECTURE.md)** is the structural gap list: the PID 1
   concurrency model and the invariants it has not yet adopted.
3. **Differential testing** below, which is the only part of the old plan with
   infrastructure still unbuilt.

## Component inventory

| Area | Crates |
|------|--------|
| Core | `libsystemd`, `systemd` (PID 1), `systemctl` |
| Journal | `journald`, `journalctl`, `journal-gatewayd`, `journal-remote`, `journal-upload`, `cat`, `bsod` |
| Devices | `udevd`, `udevadm` |
| Login and sessions | `logind`, `loginctl`, `user-sessions`, `pam-systemd`, `inhibit` |
| Network | `networkd`, `networkctl`, `networkd-wait-online`, `resolved`, `resolvectl`, `network-generator`, `timesyncd` |
| System configuration | `hostnamed`/`hostnamectl`, `localed`/`localectl`, `timedated`/`timedatectl` |
| Containers and images | `nspawn`, `machined`/`machinectl`, `portabled`/`portablectl`, `sysext`, `dissect`, `vpick`, `vpick-core` |
| Storage | `cryptsetup`, `veritysetup`, `integritysetup`, `repart`, `mount` |
| Home directories | `homed`, `homectl` |
| Resource control | `oomd`, `oomctl`, `cgls`, `cgtop` |
| Boot and power | `sd-boot`, `sd-stub`, `bootctl`, `shutdown`, `sleep`, `firstboot`, `random-seed`, `update-done`, `battery-check`, `ac-power`, `backlight`, `rfkill`, `vconsole-setup`, `modules-load`, `binfmt`, `sysctl` |
| Files and users | `tmpfiles`, `sysusers`, `fstab-generator`, `machine-id-setup` |
| Credentials and prompts | `creds`, `ask-password`, `tty-ask-password-agent`, `varlinkctl` |
| Diagnostics | `analyze`, `coredump`, `coredumpctl`, `pstore`, `delta`, `detect-virt`, `report`, `mute-console` |
| Small utilities | `escape`, `id128`, `notify`, `path`, `run` (also `run0`), `socket-activate` |
| Test support | `difftest`, `difftest-macros`, `test-journal-append`, `test-sleep`, `test-thp` |

Upstream components with no rust crate, each blocking a named test:
`systemd-importd` (TEST-25-IMPORT), `systemd-storagetm` (TEST-84-STORAGETM),
`systemd-sysupdate` and `systemd-sysupdated` (TEST-72-SYSUPDATE, currently self-skipping).

## Unit file directive coverage

414 of 429 upstream directives, from the last full audit. Re-derive before relying on
the exact numbers; the per-section shape is stable.

| Section | Supported | Partial | Unsupported | Total |
|---------|-----------|---------|-------------|-------|
| systemd.unit | 87 | 0 | 1 | 88 |
| systemd.service | 34 | 0 | 0 | 34 |
| systemd.exec | 145 | 2 | 0 | 147 |
| systemd.socket | 58 | 0 | 2 | 60 |
| systemd.resource-control | 46 | 0 | 2 | 48 |
| sd_notify | 15 | 0 | 0 | 15 |
| systemd.kill | 7 | 0 | 0 | 7 |
| systemd.timer | 14 | 0 | 0 | 14 |
| systemd.path | 7 | 0 | 1 | 8 |
| systemd.slice | 3 | 0 | 0 | 3 |
| systemd.swap | 4 | 0 | 0 | 4 |
| systemd.device | 1 | 0 | 0 | 1 |

Directive coverage is a weak signal on its own: a directive counts as supported once it
parses and is applied, which is not the same as matching upstream's semantics under
load. `ARCHITECTURE.md` and `TEST-OVERRIDES.md` are the better measures.

## Differential testing

`crates/difftest` provides the harness: a `DiffTest` trait with `run_systemd` and
`run_systemd_rs`, a `TestOutput` enum (structured JSON, raw text, binary blob, exit code,
file tree snapshot, D-Bus property map), a `DiffResult` of `Identical` /
`Equivalent(notes)` / `Divergent(explanation)`, normalizers for the usual
non-determinism, snapshot approval, JUnit and JSON reports, and a `#[difftest]`
registration macro. A golden-file corpus is in place.

Still unbuilt:

- **Dual-VM environment.** One VM on real systemd, one on rust-systemd, from identical
  NixOS configurations, coordinated over vsock or the serial console. Note that
  `default.nix` already registers a `c-systemd-test-<name>` variant of every integration
  test, which covers the single-VM comparison case; the dual-VM runner is for
  daemon-level state comparison that a single VM cannot express.
- **CI integration.** Per-change runs, a version matrix to catch systemd-release
  regressions, and a tracked `known-divergences.toml` so only new divergences fail.
