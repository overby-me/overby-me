# Test Override Ledger

Every place where `integration-tests/*.nix` weakens, skips, or replaces an upstream
systemd test, with what it would take to remove it.

The goal is to run the upstream suite unmodified. This file is the gap list.
Audited 2026-07-26 against nixpkgs systemd v258
(`/nix/store/kban61mm86a1nhq05rzg771n4l7qfjgw-source`).

## Summary

440 registered test wrappers:

| Class | Count | Meaning |
|-------|-------|---------|
| Clean | 179 | No `patchScript`. Runs upstream verbatim. |
| Environment-only | 224 | Patches NixOS-specific facts (absolute binary paths, `nobody`'s home, the harness `exit 123` line, `/testok` for standalone subtests). Not a coverage gap. |
| **Full skip** | **14** | Script replaced by `exit 77`. Nothing runs. |
| **Fake pass** | **2** | Script replaced by `touch /testok`. Nothing runs and it reports success. |
| **Mid-test skip** | **2** | Runs partway, then `touch /skipped; exit 0`. |
| **Partial mask** | **12** | Assertions deleted from a real upstream script. |
| **Substitute** | **9** | Upstream subtest replaced by a hand-written one. |

40 tests carry a real override.

Environment-only patches are legitimate and are not tracked here. They are:
deleting `systemctl --no-block exit 123` (upstream's own VM-teardown line, which the
NixOS driver replaces with the `/testok` marker); rewriting bare `sleep`/`true`/`touch`
to `/run/current-system/sw/bin/...` (NixOS has no `/bin`, and inline unit files are
subject to a compiled-in `DEFAULT_PATH_NORMAL`); `propagatesstopto-indirect` ->
`propagatestopto-indirect` (a real upstream typo, TEST-03-JOBS.sh:149 vs :150);
`WorkingDirectory='~'` expecting `/var/empty` rather than `/` (NixOS gives `nobody` a
real home); appending `touch /testok` to subtests that upstream runs under a parent
harness; and a `/proc`-walk replacing `killall sleep` (the kernel `comm` holds the full
exec path, so `killall` misses it).

## Two structural problems

### 1. The harness scored a skip as a pass (fixed)

`testsuite.nix` used to assert `test -f /testok -o -f /skipped`, and a script exiting 77
gets `/skipped` created for it. Every override in this file was therefore invisible:
`nix build .#checks.x86_64-linux.rust-systemd-test-46-homed` succeeded while running
zero lines of TEST-46-HOMED.sh.

Fixed by the `expectedSkip` argument. A `/skipped` marker now fails the check unless the
test opts in, and the opt-in list is the audit surface. 23 tests currently set it: the 16
carrying a forced `exit 77` override, plus 7 where the upstream script takes its own skip
path in this VM (06-SELINUX is not on a supported distro ID, 08-INITRD sees
`InitRDTimestampMonotonic == 0` because `boot.initrd.systemd.enable` is off, 21-DFUZZER
has no `dfuzzer`, 62-RESTRICT-IFACES reports `-BPF_FRAMEWORK`, 72-SYSUPDATE has no
binary, 75-RESOLVED has no `knotc`, 88-UPGRADE has no `/usr/host-pkgs`).

`expectedSkip` accepts either marker and prints a NOTE when the test reaches `/testok`,
so a baseline run of a newly un-skipped test is safe: it tells you to drop the flag
rather than failing.

### 2. Unit files are not wired into the VM

rust-systemd is packaged by overlaying its binaries onto the nixpkgs C systemd
derivation (`default.nix:270-300`), so the VM's unit files come from the C package.
`testsuite.nix:416-424` symlinks only eight of them into `/usr/lib/systemd/system`,
with the note that linking all of them "can overwhelm PID 1".

The C package ships these in `${systemd}/example/systemd/system/`, unlinked:

    systemd-homed.service        systemd-oomd.service       systemd-oomd.socket
    systemd-repart.service       systemd-repart@.service    systemd-storagetm.service
    systemd-importd.service      integritysetup.target      integritysetup-pre.target
    systemd-sysupdate.service    systemd-sysupdated.service

So several tests skipped as "not implemented" are really "the unit was never
installed". Addressed by the `extraUnits` argument: a test names the units it needs and
`testsuite.nix` symlinks them from either `example/systemd/system` or
`lib/systemd/system`, failing loudly on a typo, so the boot-time unit count stays
bounded.

The "overwhelm PID 1" note is itself worth a look: upstream loads its full unit set
routinely. If loading a few hundred extra units destabilises boot, that is a scaling
bug in the load path, not a reason to keep the whitelist.

## Stale rationale

Most skip comments predate the feature they name. Verified today:

| Test | Comment says | Actually |
|------|--------------|----------|
| 58-REPART | "systemd-repart stub only" | `crates/repart` is 6,364 lines; repart image building is VM-verified via 87-aux-utils-vm-validatefs |
| 46-HOMED | "no systemd-homed service unit" | `crates/homed` is 7,411 lines; the unit exists in the C package, just unlinked |
| 55-OOMD | "systemd-oomd stub only" | `crates/oomd` is 1,432 lines |
| 67-INTEGRITY | "systemd-integritysetup stub only" | `crates/integritysetup` is 3,764 lines |
| 44-LOG-NAMESPACE | "LogNamespace not yet implemented in journald" | journald runs namespace instances (`crates/journald/src/main.rs:3825-4031`); journalctl has `--list-namespaces` and `--namespace=` (`main.rs:359-363`) |
| 34-DYNAMICUSERMIGRATE | "StateDirectory alias and DynamicUser migration not yet implemented" | `exec_helper.rs:1898-1922` already builds the `private/<name>` layout with the symlink |
| 26-SYSTEMCTL | "`--global` flag not implemented" | `systemctl/src/main.rs:59,624,1071,1154` handles `--global` |
| 05-RLIMITS | replaces `systemd-run -t` with `--pipe` | `crates/run/src/main.rs:86` implements `-t`/`--pty` |
| 74-AUX-UTILS cgls | "user manager does not place transient units under app.slice" | `control.rs:3593-3604` defaults user transient units to `app.slice` |
| 35-LOGIN, 82-SOFTREBOOT, 84-STORAGETM, 60-MOUNT-RATELIMIT | baselined 2026-07-22 | still accurate |

Re-baselining (delete the override, run once, record the real first failure) is the
highest-information action available and costs one VM run each.

## Work tiers

### Tier 0: make the ledger enforceable (done)

1. `expectedSkip` argument so `/skipped` fails by default. Done.
2. `extraUnits` argument so a test can pull in specific C-package units. Done.

Neither touches rust code. Both are prerequisites for trusting anything below.

### Tier 1: re-baseline, cheap

Remove the override, run, record. Ordered by expected payoff.

| Test | Override | Why it is likely close |
|------|----------|------------------------|
| 44-LOG-NAMESPACE | fake pass | 26-line test. Both ends exist; see below for the one missing link |
| 34-DYNAMICUSERMIGRATE | fake pass | 240 lines of `RuntimeDirectory`/`StateDirectory`/`CacheDirectory`/`LogsDirectory` across `DynamicUser=0` -> `1`; the private/ layout is built |
| 58-REPART | full skip | needs `extraUnits` + real binary, both present |
| 46-HOMED | full skip | needs `extraUnits`. Note: the suite is 1,060 lines and leans on `userdbctl`, which has no rust crate, so expect the baseline to stop early |
| 55-OOMD | full skip | needs `extraUnits` + oomd binary |
| 67-INTEGRITY | full skip | needs `extraUnits` (`integritysetup.target`) + binary |
| 26-SYSTEMCTL | `edit --global` hunk deleted | flag implemented |
| 05-RLIMITS rlimit | `-t` rewritten to `--pipe` | `-t` implemented |
| 74-AUX-UTILS cgls | `--user-unit` lines deleted | `app.slice` default implemented |
| 65-ANALYZE | substitute, 1167 upstream lines -> 154 | `verify`/`security` were de-weakened since; the substitute's skip list is stale |
| 80-NOTIFYACCESS | substitute, 175 -> 86 | comment blames `systemd-run --wait`, `busctl`, `Type=notify-reload`, all of which now work |
| 07-PID1 private-pids | substitute, 176 -> 24 | biggest single coverage loss in the 07 family |
| 07-PID1 protect-hostname | substitute, 121 -> 54 | |
| 07-PID1 start-limit | substitute, 46 -> 38 | |
| 74-AUX-UTILS run | substitute, 316 -> 289 | upstream script is deleted outright (`rm -f`) |
| 59-RELOADING-RESTART | substitute, 179 -> 181 | only `ReloadLimitBurst` and `RestartMode=debug` are named as missing |

`07-pid1-exec-context` is a substitute too, but its replacement is 1,185 lines against
upstream's 447. It is broader, not weaker. Still worth running upstream's version to
find what the hand-written one does not cover.

### Tier 2: bounded feature work

| Test | Needed |
|------|--------|
| 44-LOG-NAMESPACE | `LogNamespace=` is parsed into `unit.rs:3350` and carried through `from_parsed_config.rs:799`, then dropped. Nothing in `exec_helper.rs` reads it. Needs: route the service's stdout/stderr to `/run/systemd/journal.<ns>/stdout` instead of the default socket, and add an implicit dependency on `systemd-journald@<ns>.socket` so the namespace instance is socket-activated |
| 63-PATH | The `issue-24577` block asserts a queued job is visible in `list-jobs`. rust-systemd resolves dependencies inline and has no job objects, so nothing is ever pending. Needs minimal job objects (also the largest remaining item from the old upstream divergence map) |
| 45-TIMEDATE | `testcase_timesyncd` needs a networkd dummy interface carrying link-local NTP servers so timesyncd picks them up |
| 30-ONCLOCKCHANGE | The `alternate-path` section needs timedated to notify PID 1 of a timezone change over D-Bus (`SYSTEMD_ETC_LOCALTIME` override) |
| 54-CREDS | `ImportCredential=`, the creds Varlink interface, and the `run0` credential path. Also restore the deleted `(! unshare -m ...)` assertion, which checks that the system credential directory is not visible inside a private mount namespace |
| 26-SYSTEMCTL | Interactive `systemctl edit` (the `EDITOR=... script -ec` lines) is blocked by a separate live bug: util-linux `script(1)` hangs under rust-systemd as PID 1 (parent-side termios/poll setup). The `override.conf` `cmp` assertions go with it |
| 07-PID1 protect-control-groups | `testcase_delegate_subgroup_pam` needs unprivileged PAM session management |
| 18-FAILUREACTION | The deleted phases exercise `SuccessAction=reboot` and `FailureAction=exit`. `allowReboot` and `useBootLoader` now exist in `testsuite.nix` (09-REBOOT uses them), so the reboot phase is reachable. `FailureAction=exit` kills PID 1 and still needs driver work |
| 23-UNIT-FILE ExecStopPost | Deleted `Type=dbus` and `Type=notify` sections. Both service types work now; re-run and see |
| 23-UNIT-FILE type-exec | Deleted the `busctl` block for issue #20933 |
| 07-PID1 issue-30412 | `socat` is backgrounded and killed after 2s instead of running in the foreground, so the test no longer proves the socket fd is dropped when `ExecStart` fails with 203. That is exactly what issue #30412 is about |

### Tier 3: new subsystems

| Test | Needed | Notes |
|------|--------|-------|
| 82-SOFTREBOOT | Soft reboot in PID 1: `systemctl soft-reboot`, stop units, re-exec (optionally into `/run/nextroot`), preserve fdstore and the `SoftRebootsCount` property across the re-exec | `varlink.rs:349` hardcodes `SoftRebootsCount: 0`. Multi-iteration test, so the driver must also survive the re-exec |
| 25-IMPORT | `systemd-importd` does not exist as a crate; `machinectl import-raw` needs it | |
| 84-STORAGETM | `systemd-storagetm` does not exist as a crate. The VM does have nvme-cli and `nvmet_tcp`, so the test runs for real | |
| 60-MOUNT-RATELIMIT | Event-source rate limiting for the mountinfo watcher, plus delayed mount start-jobs while it is throttled. Today a post-burst `systemctl start` races the backlogged monitor | The mountinfo monitor itself is implemented |
| 35-LOGIN | Full logind session/seat suite past `testcase_ambient_caps`: real session management and PAM | `crates/logind` is 7,237 lines and `PAMName=` now runs the PAM stack, so this may be closer than the comment suggests |
| 04-JOURNAL journal | Two things. (a) journald stores boot-time stdout streams in the fd store but never sends `FDSTORE=1` for a stream opened at runtime, so `systemctl restart systemd-journald` loses it. (b) `journalctl --follow` needs stream reconnection | The `journalctl -b <script>` mask is environmental: the NixOS driver runs the script from the backdoor shell, not as a unit, so no entry has a matching `_EXE` |

### Tier 4: VM provisioning, not rust code

These need test-image infrastructure that the nixosTest VM does not build. Upstream
provisions them with mkosi.

| Test | Needs |
|------|-------|
| 24-CRYPTSETUP | An encrypted `/var` partition created before boot |
| 64-UDEV-STORAGE | Multi-device storage topology: LVM, LUKS, MD, iSCSI |
| 86-MULTI-PROFILE-UKI | A real UKI boot with a stub binary. The VM boots kernel plus initrd directly |
| 43-PRIVATEUSER-UNPRIV | A `testuser` plus an extension image from upstream's mkosi setup |
| 50-DISSECT cluster, 58-REPART fixtures | Signed minimal verity images plus `mkosi.crt`/`mkosi.key` |

### Permanent exclusions

Document these as out of scope rather than carrying them as debt.

- **02-UNITTESTS** runs several hundred C `test-*` binaries from the systemd build tree.
  A Rust port does not produce them. The equivalent coverage is `cargo test --workspace`
  (9,700 test functions). Keep the skip, change the rationale.
- **71-HOSTNAME `testcase_nss-myhostname`** exercises the glibc NSS module from the C
  package resolving `*.localhost`. Not rust-systemd code.
- **23-UNIT-FILE whoami** asserts the running unit is `TEST-23-UNIT-FILE.service`.
  The NixOS driver runs subtests inside `backdoor.service`. Structural difference in
  the harness, not a defect.

## Cross-cutting risk

Several Tier 3 items (82-SOFTREBOOT, 60-MOUNT-RATELIMIT, 04-JOURNAL) stress the
concurrency model that `docs/ARCHITECTURE.md` describes. Two of its invariants are
still open in the code:

- `lock_ext.rs:82-97` still spins in `try_write` with a 5 ms sleep and no deadline.
- `units/unitset_manipulation/activate.rs:1151` still holds the `RuntimeInfo` read
  guard across `activate_unit`, which blocks internally on exec waits.

A bounded `writer_pending()` gate (10 s) was added at `activate.rs:1142`, which
mitigates but does not remove the starvation window. Expect hangs in this class when
working on the deep tests, and check `docs/ARCHITECTURE.md` before attributing one to
the feature under test.
