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
| **Full skip** | **15** | Script replaced by `exit 77`. Nothing runs. |
| **Fake pass** | **0** | Script replaced by `touch /testok`. Nothing runs and it reports success. |
| **Mid-test skip** | **2** | Runs partway, then `touch /skipped; exit 0`. |
| **Partial mask** | **12** | Assertions deleted from a real upstream script. |
| **Substitute** | **9** | Upstream subtest replaced by a hand-written one. |

38 tests carry a real override.

**Retired so far:** 44-LOG-NAMESPACE (was a fake pass, now runs the upstream
script to `/testok`). 34-DYNAMICUSERMIGRATE was the other fake pass; it is now an
honest skip carrying its real first failure, so no test claims success without
running. There are no fake passes left.

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
test opts in, and the opt-in list is the audit surface. 24 tests currently set it: the 17
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
| 44-LOG-NAMESPACE | "LogNamespace not yet implemented in journald" | *Fixed.* journald already ran namespace instances and journalctl already had `--list-namespaces`; only the exec-side wiring was missing |
| 34-DYNAMICUSERMIGRATE | "StateDirectory alias and DynamicUser migration not yet implemented" | *Partly fixed.* The `private/<name>` layout was already built; the 0<->1 migration and the `dir:alias:ro` syntax for Runtime/Cache/Logs directories are now implemented. Baselined to a different, real blocker (see Tier 2) |
| 26-SYSTEMCTL | "`--global` flag not implemented" | `systemctl/src/main.rs:59,624,1071,1154` handles `--global` |
| 05-RLIMITS | replaces `systemd-run -t` with `--pipe` | `crates/run/src/main.rs:86` implements `-t`/`--pty` |
| 74-AUX-UTILS cgls | "user manager does not place transient units under app.slice" | `control.rs:3593-3604` defaults user transient units to `app.slice` |
| 35-LOGIN, 82-SOFTREBOOT, 84-STORAGETM, 60-MOUNT-RATELIMIT | baselined 2026-07-22 | still accurate |

Re-baselining (delete the override, run once, record the real first failure) is the
highest-information action available and costs one VM run each.

## Progress log

Landed since the ledger was written (each regression-tested before push):

| Commit | What it fixed |
|--------|---------------|
| `8d929e42` | `expectedSkip` + `extraUnits`; a `/skipped` no longer scores as a pass |
| `a348d327` | Six exec-directory defects; 44-LOG-NAMESPACE green |
| `d112eef5` | `DynamicUser=` services can reach their `private/` exec dirs |
| `84cab4c8` | Backslash-escaped colons in `ExecDirectory=` |
| `d206d5f2` | Three `unreachable!()` panics removed from PID 1's helper wait |
| `dd941ad7` | `systemctl start --wait` returned for no `Type=oneshot` unit at all |
| `3348e6bd` | Nested `ExecDirectory=` paths (four defects) |
| `57c9a771` | `systemd.unit-dropin.*` / `systemd.extra-unit.*` credentials |
| `5c34f5a2` | Generators re-run on `daemon-reload`, as upstream does |
| `9b8752b4` | 05-RLIMITS override removed; the test passes with the real `systemd-run -t` |
| `0e8e2e08` | 26-SYSTEMCTL `--global` override removed |
| `35a1bd4e` | repart writes the empty image it is asked for; fresh GPT reports `first-lba 2048` |
| `bf1b465e` | `--empty=create` implies `--dry-run=no`, so the image reaches disk |
| `3cc24e09` | udev resolves symlink collisions by `link_priority` instead of last-writer-wins |
| `20b75d20` | repart honours `--include-partitions=`, rejects over-long `Label=`, defaults `GrowFileSystem=` on |
| `560ca74a` | repart redistributes space a capped partition cannot use; 4096-byte grain |
| `ef9c52bb` | udev unquotes imported properties; **67-INTEGRITY green with no override** |
| `0692de1d` | a udev rule program gets a deadline; udev.conf was never parsed at all |
| `b2a4b17b` | `ProtectSystem=strict` stops opening `/run` and `/var/log`; 34's `test_check_writable` green |
| `6004be66` | `systemd-repart --copy-from=`; `CopyBlocks=` had been parsed and discarded |
| `45c404c3` | repart derives labels from the type designator, numbering repeats |
| `e7c7c8a9` | repart settles space claims sequentially, keeping grain remainders |
| `c4d604db` | repart honours `--defer-partitions=` |
| `8bdeea73` | repart allocates per free area, not across their sum |
| `0a28d1d8` | a free area's span includes the partition before it, so an existing one can claim a total size |
| (this) | repart fills the lowest free slots, applies `--size=` to an existing image, and copies `CopyBlocks=` contents from a definition |
| `45c404c3` | repart derives partition labels from the type designator, numbering repeats |
| `e7c7c8a9` | repart settles space claims sequentially, keeping grain remainders |
| `c4d604db` | repart honours `--defer-partitions=` |

Both fake passes are gone. Several of these are user-facing bugs well beyond the
tests that exposed them: `systemctl start --wait` hung on every oneshot,
generated units could not change without a reboot, `systemd-repart
--empty=create` silently wrote nothing, and `/dev/disk/by-uuid/` resolved
arbitrarily whenever two devices carried the same filesystem signature.

### A rationale is only as good as the tree it was measured on

67-INTEGRITY's wrapper claimed for six commits that `extraPackages` supplied
`cryptsetup`, while the attribute was never actually committed. Every finding
attributed to it came from an uncommitted working copy and was void: the run
that "proved" `test_one crc32c 0` passes had in fact died much earlier, on
`integritysetup: command not found`. Confirm a wrapper's attributes are
committed before trusting any evidence attributed to them.

Two other retired rationales for the same reason:

- 58-REPART: "definition discovery is the gap" was wrong. `--definitions=` was
  always honoured; the `No partition definitions found.` line came from
  `systemd-repart.service` running at boot with no `repart.d`, which is benign
  and was never the failing command.
- 67-INTEGRITY: neither recorded candidate was right. `udevadm wait` passes and
  the dm device is created correctly; the failure was `blkid -U` resolving the
  filesystem to the underlying loop device.

Instrumentation is part of the tree under test. An `ERR` trap that writes to
stdout fires inside `$(...)` too, where its output is captured into the
caller's variable and corrupts the very comparison being diagnosed; that cost a
VM run. Diagnostics must write to stderr.

### Open, with evidence recorded in the test wrappers

- **34-DYNAMICUSERMIGRATE** clears all four `test_directory` phases and
  `test_check_writable`. Remaining: `test_check_idmapped_mounts`, which the
  kernel here is new enough to run and which has not been investigated.

  Its recorded rationale was **inverted**, not merely stale, and that is the
  lesson worth keeping. It said the exec directories ended up read-only and the
  service "sees 0" writable directories; instrumenting the mount table showed
  all of them already writable. The service asserts `find / -type d -writable`
  returns exactly 8, so it failed because *too much* was writable. Three
  approaches had been tried and reverted trying to make writable something that
  already was. The defect was in `ProtectSystem=strict` restoring `/run`,
  `/tmp`, `/var/tmp` and `/var/log` to read-write, which upstream's
  `protect_system_strict_table` does not.
- **55-OOMD** line 12 needs three things. Two are done (credentials, reload-time
  generators). The third: `init.scope` is not a unit in rust-systemd, only a
  cgroup path constant, so `[Scope]` resource control never reaches PID 1's
  cgroup. That also underpins `systemctl show`/`set-property init.scope`.
- **67-INTEGRITY** is GREEN with its override removed: all ten `test_one` cases
  pass. The two defects behind it were both in udev, not in
  `crates/integritysetup`, and both are user-facing well beyond this test.
  Imported properties kept their quotes, so `dmsetup udevflags` emitting
  `DM_UDEV_PRIMARY_SOURCE_FLAG='1'` never matched `10-dm.rules`, which disabled
  `13-dm-disk.rules` and left every dm device carrying a filesystem without its
  `/dev/disk/by-uuid/` symlink.
- **17-udev-failed-event** is left honestly RED with no override, and its
  wrapper carries the diagnosis. A `PROGRAM=` now gets a deadline, which fixed a
  real wedge, but it did not green the subtest; the wrapper records the next
  measurement to make. The second half needs udev workers to be processes rather
  than threads, which is architectural.
- **58-REPART** got SIXTEEN real defects fixed against it and now matches upstream
  byte for byte through `testcase_basic` steps 1 to 5, including the whole
  six-partition `--copy-from=` table, `--defer-partitions=`, the deferred
  refill, both resizes and the `CopyBlocks=` contents check. It is masked again
  at step 6, where `--size=auto` has to grow an image that is already full and
  rust only honours `auto` while creating one.

  Step 6 is blocked on a missing feature rather than a defect: `Encrypt=` has no
  LUKS or cryptsetup implementation at all, and `CopyFiles=` is parse-only, so
  the rest of `testcase_basic` is out of reach without LUKS2 support.

  SIX repart settings turned out to be parsed into a struct, unit tested for
  parsing, and then never consulted anywhere in the logic:
  `--include-partitions=`, `--exclude-partitions=`, `--defer-partitions=` and
  `CopyBlocks=` (all now fixed), plus `CopyFiles=` and `Encrypt=` (still open).
  `CopyBlocks=` is the sharpest: the partition *table* was byte-for-byte correct
  while the partition *contents* were never written, so no table assertion could
  have caught it. Assert the effect, not the table — and treat a passing parse
  test as no evidence at all that a flag is honoured.

  Four of the ten defects were options parsed into the argument struct, unit
  tested for parsing, and then never consulted: `--include-partitions=`,
  `--exclude-partitions=`, `CopyBlocks=` and `--defer-partitions=`. A passing
  parse test is not evidence a flag is honoured, and the rest of the crate is
  worth auditing the same way.

## Work tiers

### Tier 0: make the ledger enforceable (done)

1. `expectedSkip` argument so `/skipped` fails by default. Done.
2. `extraUnits` argument so a test can pull in specific C-package units. Done.

Neither touches rust code. Both are prerequisites for trusting anything below.

### Tier 1: re-baseline, cheap

Remove the override, run, record. Ordered by expected payoff.

| Test | Override | Why it is likely close |
|------|----------|------------------------|
| 58-REPART | full skip | needs `extraUnits` + real binary, both present |
| 46-HOMED | full skip | needs `extraUnits`. Note: the suite is 1,060 lines and leans on `userdbctl`, which has no rust crate, so expect the baseline to stop early |
| 55-OOMD | full skip | needs `extraUnits` + oomd binary |
| 67-INTEGRITY | full skip | needs `extraUnits` (`integritysetup.target`) + binary |
| 26-SYSTEMCTL | `edit --global` hunk deleted | flag implemented |
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
| 34-DYNAMICUSERMIGRATE | All four `test_directory` phases (State/Runtime/Cache/Logs) now pass in full, including both `DynamicUser=` directions. Remaining: `test_check_writable`, which needs nested exec directories (`quux/pief`, `aaa/bbb`, `xxx/yyy:aaa/111`), and then idmapped mounts on kernels >= 5.12. The wedge that blocked this was NOT a race or lock starvation: `systemctl start --wait` polled for `Stopped`, while a completed `Type=oneshot` deliberately stays `Started` to avoid boot activation-graph races, so `--wait` on any oneshot could never return |

### Exec directories: the `private/` rule

Worth writing down, because the prose comment in upstream's `exec-invoke.c` is
easy to misread and the authoritative version is elsewhere.

`DynamicUser=` puts an exec directory at `<base>/private/<name>` with a
`<base>/<name>` symlink. `private/` is mode 0700 owned root:root **on purpose**:
it is a security boundary stopping unprivileged host users from reading state
that belongs to a dynamic UID which may later be reused. It must not be loosened.
The service reaches its own directory because the mount namespace replaces
`private/` with a permissive tmpfs into which only that service's directories are
bound. Binding `<base>/private/<name>` onto the `<base>/<name>` symlink instead
does nothing: the kernel resolves the destination straight back to the source.

Which directory types get this treatment is decided by
`exec_directory_is_private` (`src/core/execute.c:377`), not by the comment:

    dynamic_user must be set, and
    the type must be one that gets chown'd (so never ConfigurationDirectory), and
    RuntimeDirectory is excluded only when RuntimeDirectoryPreserve=no

So `RuntimeDirectory=` *does* use `private/` whenever `RuntimeDirectoryPreserve=`
is not `no`, which is why TEST-34-DYNAMICUSERMIGRATE sets it on every command and
then asserts `/run/private/zzz` exists.

**Open divergence:** rust-systemd applies `private/` to runtime directories
unconditionally. Matching upstream needs `runtime_directory_preserve` plumbed
from `exec_config` into `ExecHelperConfig`, which it is not today.

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
