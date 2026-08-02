# Roadmap

## Where the port stands

NixOS boots with rust-systemd as PID 1 and reaches `multi-user.target` with a login
prompt in roughly 6 seconds under cloud-hypervisor, with networking via networkd and
resolved.

93 crates, about 370,000 lines of Rust, about 9,700 unit test functions.

The original six-phase port plan is finished as a structuring device: every phase has
shipped its components. The gap lists remain authoritative for *what* is missing:

1. **[TEST-OVERRIDES.md](TEST-OVERRIDES.md)** is the functional gap list. 37 of the 440
   registered integration tests still carry a real override. That file says what each
   one needs.
2. **[ARCHITECTURE.md](ARCHITECTURE.md)** is the structural gap list: the PID 1
   concurrency model and the invariants it has not yet adopted.
3. **Differential testing** below, which is the only part of the old plan with
   infrastructure still unbuilt.

What to work on next, however, is no longer "remove the next override". That stopped
measuring progress once the cheap seam was exhausted. The strategy below is the plan.

## Strategy, 2026-08-02

A full review of the approach, measured against the tree (375k lines of Rust against
1.02M lines of C in the pinned v260.2 `src/`), reached this verdict: the premise is
sound and the parity-plus-ledger method is the right one, but three course corrections
follow from where it strains.

### Why the priorities changed

1. **The concurrency model is the largest liability.** Upstream PID 1 is one thread,
   one event loop, a job engine: nothing can starve. The thread pool plus global
   RwLock is the one place this port reimagined instead of followed, and it is the
   source of the wedge class (the open invariants in ARCHITECTURE.md). Its
   structural-work list is, read honestly, a plan to become upstream's event loop.
   Do that deliberately, not invariant by invariant.
2. **Suite-green is not the finish line.** The remaining overrides encode missing
   subsystems (job objects, the user manager, autofs, importd/storagetm/sysupdate,
   mkosi-class fixtures), so the marginal green test no longer tracks user value.
   Shippable increments do.
3. **The memory-safety payoff is in the parsers.** PID 1's inputs are root-trusted;
   systemd's worst CVEs lived in resolved, networkd's DHCP and journald's wire
   protocols. Those crates have had the least test pressure here.

### The plan, in order

1. **Ship one increment.** One rust daemon (journald, udevd, or tmpfiles/sysusers)
   under C PID 1 on one real, low-stakes NixOS machine, as a module override with
   generation rollback. This exercises design principle 5 for the first time and
   opens a production feedback channel that VM tests cannot provide.
2. **Event-loop convergence**, as a named project with a design doc: minimal job
   objects plus a single state-changed dispatcher queue. Retires invariants I1-I6
   wholesale and unblocks TEST-63-PATH, TEST-60-MOUNT-RATELIMIT and the
   ExecStopPost deadlock as side effects.
3. **The user manager** (config-driven `systemd --user`), the largest single unlock
   for both tests and real usability.
4. **Version-matrix CI** (see Differential testing below): run the suite against new
   upstream pins on a schedule, so upstream drift arrives as failing tests instead
   of silent parity decay.
5. **Differential fuzzing of the remote-facing parsers** (resolved, networkd DHCP,
   journald stream and remote protocols) against the C implementations, reusing
   upstream's fuzz corpora and the `difftest` harness.
6. **Publish.** Extract or mirror the tree, with a license and attribution review;
   the harness and the ledger method are independently valuable.

### Falsification checkpoint

If, after item 2 lands and a real machine has run rust components for a month, PID 1
wedge-class bugs still surface at a steady rate, freeze the PID 1 ambition and ship
daemons under C PID 1. Decide then, not never.

### Method rules

The discipline that kept the port honest, kept explicit:

1. Every weakening of a test lives as an entry in TEST-OVERRIDES.md. There are no
   silent skips (`expectedSkip` enforces this) and no fake passes.
2. Before debugging any failure, run the `c-systemd-test-<name>` oracle to classify
   it as environmental or real.
3. Follow upstream's architecture. Every deviation is documented debt in
   ARCHITECTURE.md, carrying its consequences.
4. Schedule outside-in audits (random sampling sweeps of wrappers, as on
   2026-07-28); assume the green metric gets gamed.
5. "Done" means a shippable increment someone runs, not a green count.

These rules are generalized for every port in the portfolio in
[../../PORTING.md](../../PORTING.md); this port is the exemplar its rules point at.

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
