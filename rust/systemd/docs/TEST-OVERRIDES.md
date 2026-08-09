# Test Override Ledger

Every place where `integration-tests/*.nix` weakens, skips, or replaces an upstream
systemd test, with what it would take to remove it.

The goal is to run the upstream suite unmodified. This file is the gap list.
Audited 2026-07-26 against nixpkgs systemd v260.2
(`/nix/store/kban61mm86a1nhq05rzg771n4l7qfjgw-source`; the header previously said
v258, but that store path's `meson.version` is 260.2).

## Summary

440 registered test wrappers:

| Class | Count | Meaning |
|-------|-------|---------|
| Clean | 179 | No `patchScript`. Runs upstream verbatim. |
| Environment-only | 224 | Patches NixOS-specific facts (absolute binary paths, `nobody`'s home, the harness `exit 123` line, `/testok` for standalone subtests). Not a coverage gap. |
| **Full skip** | **15** | Script replaced by `exit 77`. Nothing runs. |
| **Fake pass** | **0** | Script replaced by `touch /testok`. Nothing runs and it reports success. |
| **Mid-test skip** | **1** | Runs partway, then `touch /skipped; exit 0`. |
| **Partial mask** | **10** | Assertions deleted from a real upstream script. |
| **Substitute** | **9** | Upstream subtest replaced by a hand-written one. |

37 tests carry a real override.

## Coverage denominator

Honest denominators against upstream systemd 260.2 (`test/integration-tests/`),
so a subset is never read as a total:

- **Upstream integration suite: 67 families** (`TEST-01-BASIC` through
  `TEST-89-RESOLVED-MDNS`), plus 25 standalone unit-test data directories
  (`test-execute/`, `test-fstab-generator/`, `test-network-generator-conversion/`,
  `test-sysusers/`, and so on) exercised outside the VM.
- **rust/systemd registers 65 of the 67 families**, expanded per subtest into
  443 `integration-tests/*.nix` wrappers. The two not yet mirrored are
  **TEST-69-SHUTDOWN** and **TEST-85-NETWORK**, both driven by systemd's newer
  Python integration framework (TEST-69 ships only as `test/units/TEST-69-SHUTDOWN.py`
  and TEST-85 ships no `test/units` script at all), which the shell-script harness
  here does not run. Closing them is a harness-capability task, not a per-feature
  gap.
- **Pass counts are deliberately omitted here.** They require booting each VM and
  are recorded per test in the wrappers, not asserted in this file. Of the 443
  wrappers, 37 carry a real override (the classes above); that breakdown was last
  fully audited at 440 wrappers on 2026-07-26, so the three wrappers added since
  are not yet reclassified.

Separately, the parsers and CLIs are frozen against the C binaries without a VM
by the in-process differential oracles (`just differential`): `systemd-escape`,
`systemd-analyze`, `systemd-id128`, `systemd-creds`, `systemd-journal-remote`,
`systemd-network-generator`, and `systemd-fstab-generator`, plus in-tree
robustness fuzzers. See `docs/TESTING.md`.

Not every CLI is oracle-clean. `systemd-path` in particular is a hardcoded
approximation rather than a port of C's path computation: measured against
C 260.2 its name set is off (5 of C's 77 names missing, 8 extra), and its
values are environment- and install-prefix-dependent, so it is deliberately
outside the differential corpus. A faithful rewrite of its search-path logic
is open work.

## Subtest registration audit, 2026-07-28

Every upstream `TEST-NN-FAMILY.subtest.sh` was compared against the registered
wrappers, not just one family. **209 upstream subtests, 207 registered.**
Coverage is far better than the override counts alone suggest.

The gaps:

| Upstream subtest | Status |
|------------------|--------|
| `TEST-07-PID1.alias-corruption` | Was unregistered; now registered, and it FAILED on its first run with a real MainPID bug. |
| `TEST-29-PORTABLE.user` | Unregistered. Self-skips in this VM anyway: it requires systemd-mountfsd.socket, systemd-nsresourced.socket, mksquashfs, the BPF LSM, libbpf, kernel >= 6.5, polkit >= 124 and a BTF build. Registering it would add a skip, not coverage. |
| `TEST-50-DISSECT.encrypted` | Unregistered. Belongs to the DISSECT cluster already blocked on missing mkosi image fixtures. |

Three others looked missing but are registered under shortened names, so do not
re-report them: `stopped-socket-activation` → `04-journal-stopped-socket`,
`SYSTEMD_JOURNAL_COMPRESS` → `04-journal-compress`, `JoinsNamespaceOf` →
`23-unit-file-joinsnamespace-of`.

Note also that the 07 family carries 91 wrappers against 51 upstream subtests,
so roughly 40 are hand-written additions rather than substitutions.

## Sweep, 2026-07-28

Four tests not previously examined were run to sample the suite rather than
keep polishing one failing test:

| Test | Result |
|------|--------|
| 07-PID1 exec-deserialization | PASSES. The note calling it "the closest-to-green test, blocked on the invariant-I1 lock decoupling" is STALE — it is green. |
| 05-RLIMITS effective-limit | PASSES |
| 07-PID1 concurrency | PASSES. `ConcurrencyHardMax=`/`ConcurrencySoftMax=` are parsed (SliceConfig) and enforced by a new `slice_concurrency` module porting systemd's `slice_get_currently_active` and the soft/hard reached checks (subtree count including sub-slices, walking up parents). The hard limit refuses the start at enqueue; the soft limit parks the start (job kept Waiting, client blocked) and is released event-driven: the dispatcher notifies a condvar whenever a unit settles inactive, and slice stops mark the slice unit Stopped (deactivate_unit) so nested "two slots (slice+service)" accounting is right. Scoped to concurrency-limited slices, so normal starts are untouched. |
| 07-PID1 delegate-namespaces | Full upstream test still FAILS (all-or-nothing over 7 testcases), but 5 of the 7 now pass in isolation and are shipped as focused subtests. `DelegateNamespaces=` parses (unit key + `systemd-run -p` transient) into a Vec of mnt/net/pid/uts/ipc/cgroup; a non-empty value implies `PrivateUsers=self` (start_service.rs); and for a delegated namespace its setup is DEFERRED until after the service's user namespace, so the namespace is owned by that user ns and the service (uid 0 in it) holds CAP_SYS_ADMIN/CAP_NET_ADMIN over it. VM-verified focused subtests: `07-pid1-delegate-net` (testcase_network: `ip link add` refused EPERM without delegation, succeeds with `DelegateNamespaces=net`), `07-pid1-delegate-uts` (testcase_uts: `hostname abc` refused without delegation, succeeds with `DelegateNamespaces=uts`), and `07-pid1-delegate-identity` (the exact testcase_implied_private_users_self, incl. the `PrivateUsersEx=identity` range map), and `07-pid1-delegate-pid` (testcase_pid: writing `/proc/sys/kernel/ns_last_pid` is refused without delegation and succeeds with `DelegateNamespaces=pid`). The identity/full RANGE uid_map `0 0 65536` cannot be self-written after unshare(CLONE_NEWUSER) (the kernel needs CAP_SETUID over the parent ns); this is now handled IN exec_helper by forking a helper that stays in the parent ns and writes `/proc/<us>/{setgroups,uid_map,gid_map}` from outside (async-signal-safe), no PID 1 rework. pid delegation is created in exec_helper after the user ns (`unshare(CLONE_NEWPID)` then fork, exec_helper becoming a wait/exit-propagate shim that re-raises a terminating signal; plain `PrivatePIDs=` keeps its `clone(CLONE_NEWPID)` path unchanged). The remaining 2 testcases are deep: user_manager (needs `systemctl start user@0` + `systemd-run --machine --user`), multiple_features (needs the minimal_0.raw squashfs). |

Also found by comparing upstream's `TEST-07-PID1.*.sh` against the registered
set: 52 upstream subtests, 51 registered. `alias-corruption` had no wrapper and
had never run. It is registered now with no override.

Note the 07 family has 91 wrappers against 52 upstream subtests, so ~39 are
hand-written additions rather than substitutions; upstream coverage there is
essentially complete, which the "9 substitutes" row understates.

**Retired so far:** 44-LOG-NAMESPACE (was a fake pass, now runs the upstream
script to `/testok`). 34-DYNAMICUSERMIGRATE was the other fake pass; it is now an
honest skip carrying its real first failure, so no test claims success without
running. There are no fake passes left.

## Verification sweep, 2026-07-28 (later the same day)

About 60 wrappers were run across roughly 20 families, chosen to spread over
tests that had never been sampled rather than to re-run known ones. The point
was to check the ledger's claims from the outside, especially "no fake passes
left".

Two real failures surfaced.

| Test | Result |
|------|--------|
| 07-PID1 multi-exec-start | FAILED, now FIXED. A `Type=oneshot` with several `ExecStart=` ignored a failing preliminary command, because the deferred driver discarded `wait_for_helper_child`'s result while `Service::run_cmd` checks it. Fixing that exposed a second defect: reporting the failure without moving the unit out of `Starting` re-ran the whole sequence about every 61s forever. |
| 59-RELOADING-RESTART | PASSES as of 2026-08-02. The wrapper once described the graceful-SIGTERM fix as though the subtest passed while ExecMainStatus read back empty; the real cause was the Stop handler unloading stopped transient units before `systemctl show` could read them, fixed by letting them linger until reset-failed or daemon-reload. |

Everything else was either a genuine pass or an honest declared skip. Short
logs are common and are usually fine: `31-DEVICE-ENUMERATION` traces three
lines because the upstream script really is ten lines, and
`66-DEVICE-ISOLATION`, `44-LOG-NAMESPACE`, `52-HONORFIRSTSHUTDOWN` are
similarly small. `43-PRIVATEUSER-UNPRIV` and `64-UDEV-STORAGE` produce no
script trace at all, and both are `expectedSkip` entries whose wrappers
replace the script with `exit 77`, which is what a declared skip should look
like.

So the "no fake passes" claim held everywhere it was tested. When judging a
short or empty log, read the wrapper for `expectedSkip`/`patchScript`, read
the upstream `test/units/<NAME>.sh`, and grep the log for `testok|skipped`;
length alone proves nothing. A zero-length log with a non-zero exit is
usually a mistyped attribute name rather than a failure.

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
| 35-LOGIN | "logind session suite not implemented" | *Wrong, twice.* `crates/logind` is 7,237 lines. The follow-up claim that rust had no user manager was also wrong: `run_user_manager()` existed and already sent `READY=1`. logind simply never started any unit. Fixed; `testcase_background` now passes end to end |
| 82-SOFTREBOOT, 84-STORAGETM | baselined 2026-07-22 | still accurate |

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
| `b2a4b17b` | `ProtectSystem=strict` stops opening `/run` and `/var/log` (a real sandbox hole; it did NOT reliably green 34's `test_check_writable`, see below) |
| `6004be66` | `systemd-repart --copy-from=`; `CopyBlocks=` had been parsed and discarded |
| `45c404c3` | repart derives labels from the type designator, numbering repeats |
| `e7c7c8a9` | repart settles space claims sequentially, keeping grain remainders |
| `c4d604db` | repart honours `--defer-partitions=` |
| `8bdeea73` | repart allocates per free area, not across their sum |
| `0a28d1d8` | a free area's span includes the partition before it, so an existing one can claim a total size |
| (this) | repart fills the lowest free slots, applies `--size=` to an existing image, and copies `CopyBlocks=` contents from a definition |
| (this) | `udevadm info` accepts a `/dev` symlink such as `/dev/mapper/<name>` |
| `3d640d43` | mounts are no longer stacked over an existing mount; `/` was mounted twice with contradictory flags |
| `b779bc24` | `PrivateUsers=` drops to in-namespace ids; `TemporaryFileSystem=` mount points created before the read-only pass |
| `9e42a37b` | id-mapped mount groundwork (`open_tree`/`mount_setattr`/`move_mount`), not yet working |
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

- **34-DYNAMICUSERMIGRATE** clears all four `test_directory` phases.
  `test_check_writable` is NOT fixed, and the claim that it was is retracted: it
  passed in exactly one VM run and does not reproduce on a tree that is
  code-identical in every libsystemd file. Treat one green run on an exact-count
  assertion as no evidence at all.

  The `ProtectSystem=strict` change behind that claim is still correct on its
  own merits, and stays: rust restored `/run`, `/tmp`, `/var/tmp` and
  `/var/log` to read-write where upstream's `protect_system_strict_table`
  restores only `/proc`, `/sys` and `/dev`, which let any strict service write
  across the runtime and log trees. It simply does not make this assertion pass
  reliably, so something else is also wrong.

  `test_check_idmapped_mounts` is now the only failing phase, and 34 is left
  RED ON PURPOSE rather than masked: an `expectedSkip` replaces the whole script
  with `exit 77`, which would stop exercising the four `test_directory` phases
  and `test_check_writable` that now pass. A green tick is not worth losing that.

  Three defects fell out of that phase, each visible only once the previous was
  cleared: the doubly-mounted root (which also caused an
  `unshare(CLONE_NEWUSER)` EPERM, so one fix closed two bugs); the privilege
  drop targeting the outside uid/gid inside a `PrivateUsers=` namespace, where
  the default map `0 <uid> 1` makes id 0 the only representable one; and
  `TemporaryFileSystem=` mount points being created after `ProtectSystem=` had
  made `/` read-only, so the mkdir failed EROFS and the mount then failed
  ENOENT.

  What remains is a feature: id-mapped mounts. The groundwork is in and safe
  (the plain bind still happens; the idmap is an overlay attempted on top, so a
  failure only logs), but `mount_setattr(MOUNT_ATTR_IDMAP)` returns EPERM while
  `open_tree` and the detached mount succeed. Check the exec helper's effective
  capabilities at that point: the kernel's `can_idmap_mount()` gives EPERM
  without `CAP_SYS_ADMIN` in the *superblock's* user namespace, and EINVAL when
  the filesystem lacks `FS_ALLOW_IDMAP`.

  `test_check_writable` now passes, on two independent runs.

  The cause was `remount_read_only()` binding every path onto itself before
  remounting read-only. For a path already a mount point that stacks a second
  mount, and for `/` it left the root mounted twice with contradictory flags
  plus a duplicated subtree: `touch` resolved through the writable view and
  `access(W_OK)`, which `find -writable` calls, through the read-only one. The
  bind is now skipped when `/proc/self/mountinfo` already lists the path.

  Three wrong causes were asserted before that one, and the reasons are worth
  keeping. The recorded rationale said the service sees zero writable
  directories, which was right; I overturned it on an instrument that called
  `access(W_OK)` as **root**, before the service drops to its dynamic uid, so it
  measured a principal the assertion was never about. Then traversal was
  suspected, but the service could `touch` the directory fine. What settled it
  was putting the `find`, the write probe and `/proc/self/mountinfo` in ONE
  process: stitching conclusions across separate runs is what produced every
  wrong answer.

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
| 26-SYSTEMCTL | `edit --global` mask removed 2026-07-27; ALL interactive EDITOR hunks UN-MASKED + passing 2026-08-07 (scaffold no-op-discard + upstream `+4` editor arg), including the `user@0` template-instance edit (#26483 regression). patchScript now only strips the `script` TTY wrapper | done |
| 65-ANALYZE | substitute, 1167 upstream lines -> ~184 | de-weakened 2026-08-05: `architectures`, `filesystems`, `syscall-filter` and `transient-settings` added with faithful upstream assertions (VM-validated), joining `verify`/`security`/`condition --unit`/`cat-config`. Still unported: `dump`, `plot` (SVG), `architectures --json`, and the `--global` flag on `filesystems`/`syscall-filter` (rust rejects it, which the negative assertions exploit) |
| 80-NOTIFYACCESS | substitute, de-substituted toward the real upstream | GREEN (VM-verified 2026-08-05). Covers NotifyAccess= enforcement, the status-error triad (`StatusErrno`/`StatusBusError`/`StatusVarlinkError`), the `Type=notify-reload` reload substates (`reload-signal`->`reload-notify`->`running` with `ReloadResult=timeout`), and (2026-08-05) the full upstream fd-store pinning lifecycle: `FileDescriptorStoreMax=`/`$FDSTORE`, `systemd-notify --fd/--fdname` (added -- sends the fd via SCM_RIGHTS with `FDSTORE=1`), `NFileDescriptorStore`, `FileDescriptorStorePreserve=yes` vs `restart`, survival across `systemctl restart` but release on a full `systemctl stop` unless pinned (release wired on BOTH the inline and live-dispatcher stop paths, since the VM uses the dispatcher), the `SubState=dead-resources-pinned` pinned-dead state, and `systemctl clean --what=fdstore`. The `NotifyAccess=none` case now polls for the failed state instead of racing the async transition. `systemd-analyze fdstore` introspection (text line count + the `--json=short` fdname/type/devno/inode/rdevno/path/flags shape) is now covered too, via a new `fdstore-dump` control method that fstat/readlink/F_GETFLs PID 1's live stored fds. The analyze section is lightly adapted: NotifyAccess=all instead of upstream's `--pid=parent` and no `$FDSTORE` env check, since rust does not implement `--pid=parent` or export `$FDSTORE`. Effectively at full upstream coverage now |
| 07-PID1 private-pids | substitute; procfs check strengthened 2026-08-05 | Biggest single coverage loss in the 07 family. The procfs-mount assertion now checks the full `rw,nosuid,nodev,noexec` option set (was `nosuid` only), VM-verified. The SIGKILL/`Result=signal` part of testcase_basic (upstream SIGKILLs the namespace PID 1 and asserts `Result=signal`/`ExecMainStatus=9`) was a real bug when this substitute was written but is now FIXED (by the later signal-death `ChildTermination` work); it is covered and VM-verified by the focused subtest `07-pid1-privatepids-sigkill` (a `PrivatePIDs=yes --remain-after-exit` main SIGKILLed via `kill -9 $ExecMainPID` records `Result=signal`/`ExecMainStatus=9`). Still substitutes the rest (testcase_analyze: its core PrivatePIDs=yes-vs-Type=forking incompatibility check is now implemented in `verify_unit_file` and covered by `07-pid1-privatepids-verify` + unit tests, but the full upstream form additionally needs `systemd-analyze --recursive-errors=no` accepted BEFORE the verb, a clap CLI-ordering gap vs C getopt where global options may precede the verb; testcase_multiple_features; testcase_unpriv which needs the user manager + `/home/testuser` blocked by #35) |
| 07-PID1 protect-hostname | substitute; `yes` cases de-weakened 2026-08-05 | `ProtectHostname=yes` now installs the port's FIRST seccomp filter (a minimal BPF denying `sethostname`/`setdomainname` with EPERM, `exec_helper::seccomp_block_hostname`), so the `yes` cases assert the real block (`(! ... hostname foo)`, and `yes:hoge` sets the hostname but cannot change it) rather than the old isolation-only stand-in; `private` still permits changes. VM-verified. Still a substitute: omits the `hostnamectl`/`hostnamed` intro and the specifier/invalid cases of the full upstream script. Seccomp is now seeded here and could later back `SystemCallFilter=`/`RestrictFileSystems=` |
| 07-PID1 start-limit | substitute, 46 -> 38 | |
| 74-AUX-UTILS run | SUBSTITUTE REMOVED; the real upstream script runs. Seven systemd-run defects fixed out of it: --slice-inherit, empty --working-directory= reset, --same-dir, --expand-environment= / the `:` no-env-expand prefix, transient-unit collection on stop, and (2026-08-05) two user-manager identity/IO gaps -- (a) the transient stdout/stderr written to the root-owned system `/run/systemd/transient` (EPERM) now runtime-scoped to `$XDG_RUNTIME_DIR/systemd/transient` in `control.rs` gated on `SYSTEMD_USER_MANAGER` (system path byte-for-byte unchanged); (b) a numeric `User=` with no `Group=` (e.g. user@%i.service's `User=%i`) dropped to the manager's GID instead of the user's primary group, so `id -ng` was `root` -- `resolve_gid_with_user_fallback` now resolves `pw_gid` via `getpwuid_r` for a numeric UID too, not only `getpwnam_r` for a name. Both VM-verified 2026-08-05: the `--user --machine=testuser@` cgroup-match AND `[[ "$(id -nu)" == testuser && "$(id -ng)" == testuser ]]` assertions now pass. A third numeric-`User=` login-env gap was fixed alongside: `HOME`/`USER`/`LOGNAME`/`SHELL` are now resolved from the passwd record for a numeric `User=` (getpwuid), not only a name (getpwnam), so the per-user manager (`user@%i.service`, `User=%i`) and everything it spawns carry `HOME=/home/testuser` (VM-verified). Still RED at the next line, `[[ "$PWD" == /home/testuser && -n "$INVOCATION_ID" ]]`. INVOCATION_ID is set. The home mechanism is now traced -- systemd's `unit_patch_contexts` (unit.c) defaults a user-manager service's `WorkingDirectory` to `get_home_dir()` when none is given -- and the manager resolves the home correctly (`chosen=/home/testuser`). But the assertion stays RED for a deeper reason surfaced in-VM: `/home/testuser` is not an accessible directory (`is_dir=false` from the manager at euid 1001), so no chdir to home can succeed and upstream's own `working_directory_missing_ok` fallback would land at `/`. ROOT CAUSE (in-VM confirmed): `/home/testuser` is never created. NixOS creates `createHome` user homes (as root) during system activation via `nixos-activation.service`, but in the failing VM only the *user* manager (uid 1001) ever touches `nixos-activation.service` -- the system manager (PID 1) never runs it -- and an unprivileged run cannot make the home. (The user manager loading `nixos-activation.service` is itself expected: NixOS ships that unit in `/etc/systemd/user/default.target.wants/`, so a C `systemd --user` loads it too; that is a red herring, not a scoping bug.) So the fix is system-side -- PID 1 must run the NixOS activation, or otherwise create testuser's home -- after which the already-understood manager-side working-directory-to-home default (unit_patch_contexts) lands the assertion. This is a boot/activation-wiring gap, distinct from working-directory defaulting; the numeric-`User=` HOME/GID/login-env fixes above stand on their own |
| 59-RELOADING-RESTART | substitute, 179 -> 181 | only `ReloadLimitBurst` and `RestartMode=debug` are named as missing |

`07-pid1-exec-context` is a substitute too, but its replacement is 1,185 lines against
upstream's 447. It is broader, not weaker. Still worth running upstream's version to
find what the hand-written one does not cover.

### Tier 2: bounded feature work

| Test | Needed |
|------|--------|
| 63-PATH | The `issue-24577` block asserts a queued job is visible in `list-jobs`. rust-systemd resolves dependencies inline and has no job objects, so nothing is ever pending. Needs minimal job objects (also the largest remaining item from the old upstream divergence map) |
| 45-TIMEDATE | `testcase_timesyncd` needs a networkd dummy interface carrying link-local NTP servers so timesyncd picks them up |
| 54-CREDS | Updated 2026-08-03: `cmd_list` now matches C's `verb_list` (exit 1/ENXIO when no credentials resolve, exit 0 for a set-but-empty dir; verified differentially against the C binary), so the wrapper skips honestly before the genuinely-unsatisfiable line 95 (`systemd-creds --system`; this VM has no `/run/credentials/@system`) instead of fake-traversing past it via the old exit-0 bug. Still blocked on: `ImportCredential=`, the creds Varlink interface, the `run0` credential path, and a PID 1 import-creds path, which is what would make line 95 satisfiable and let the script reach the deleted `(! unshare -m ...)` assertion at line 171 |
| 26-SYSTEMCTL (all edits pass) | The full interactive `systemctl edit` block RUNS + passes (un-masked 2026-08-07: the drop-in scaffold no longer seeds `[Service]` so a no-op discards, and the editor gets the upstream `+4` line arg so `mv +4 <path>` swaps in the prepared file). EDITOR=true/mv + their `override.conf` assertions and the `user@0` #26483 regression all pass. The only adaptation left is that the patchScript strips the util-linux `script` TTY wrapper (`systemctl edit` needs no TTY here); fixing the underlying `script(1)`-under-PID-1 hang (parent-side termios/poll) would let the lines run verbatim, but that is a separate deep bug, not 26-edit work |
| 07-PID1 protect-control-groups | `testcase_delegate_subgroup_pam` needs unprivileged PAM session management |
| 18-FAILUREACTION | Phase 1 (`SuccessAction=reboot`) restored 2026-07-30 and MEASURED RED 2026-08-02: the reboot fires and the second boot is healthy, but the test driver's root shell never reconnects (09-REBOOT harness class), so the test does not reach its post-reboot assertions and DOES NOT PASS. Only the `FailureAction=exit` line is still deleted, and not because it "kills PID 1": upstream degrades `exit` to `poweroff` for a system manager (emergency-action.c:153-170), so the machine ends cleanly either way, and our harness checks `/testok` from the host once the script returns. Its `FailureActionExitStatus=123` assertion is covered by unit tests, the setting having been silently ignored until that assertion was read closely |
| 23-UNIT-FILE ExecStopPost | PASSES as of 2026-08-03, the EVENT-LOOP inc 3 expected flip. Deferred-start failures now run `ExecStopPost=` as a dispatcher poststop chain before finalizing, which is exactly the lock decoupling this row said the in-place fix needed (the historical in-place attempt deadlocked PID 1 under the read+write guards). Getting every section green also required three real fixes: the transient-property loop dropped `BusName=` so Type=dbus transients started unconfigured, a Type=dbus main exiting before its name appeared left a stale parked wait, and a Type=notify main exiting before READY=1 was treated as a clean deactivation instead of upstream's protocol failure |
| 07-PID1 issue-30412 | `socat` is backgrounded and killed after 2s instead of running in the foreground, so the test no longer proves the socket fd is dropped when `ExecStart` fails with 203. That is exactly what issue #30412 is about |
| 34-DYNAMICUSERMIGRATE | All four `test_directory` phases (State/Runtime/Cache/Logs) now pass in full, including both `DynamicUser=` directions. `test_check_writable` now PASSES (fixed via the doubly-mounted-root and ProtectSystem=strict corrections; full record in `integration-tests/34-dynamicusermigrate.nix`). The remaining blocker is `test_check_idmapped_mounts`: a missing feature needing id-mapped exec directories (`mount_setattr(MOUNT_ATTR_IDMAP)`) with correct ownership translation (a write must land on-disk as nobody and read back inside as the service's uid). The `create_mapped_userns`/`idmapped_bind` groundwork is in but the translation map is not yet right (12 mechanisms investigated; next step is instrumenting the exec path after drop_privileges). The wedge that first blocked this family was NOT a race or lock starvation: `systemctl start --wait` polled for `Stopped`, while a completed `Type=oneshot` deliberately stays `Started` to avoid boot activation-graph races, so `--wait` on any oneshot could never return |

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
| 60-MOUNT-RATELIMIT `testcase_issue_23796` | An external mount(8) type helper (`mount.mytmpfs`) driven by a `--no-block` start that must survive `daemon-reexec`. Two separable gaps: (a) `activate_mount` issues the mount(2) syscall directly, so an unknown `Type=` is never handed to `/sbin/mount.<type>` the way C does; (b) background mount jobs are not serialized across reexec | Skipped via `TEST_SKIP_TESTCASES` (per-subtest, not a whole-file skip). The other three subtests pass on BOTH rust and the C oracle: `testcase_issue_20329` (post-burst `systemctl start` re-mounts a stale-`Started` mount + monitor skips a stop for a freshly re-mounted path) and `testcase_long_path` (over-long mount paths hashed via a faithful `unit_name_hash_long`/SipHash-2-4 port) are now fixed, and `testcase_mount_ratelimit` passes with the mountinfo watcher's event-source rate limiting now implemented (a faithful port of upstream's interval-1s/burst-5 throttle; the `(mount-monitor-dispatch) entered/left rate limit` transitions fire and the subtest's stricter grep branch validates them) |
| 35-LOGIN | Autologin sessions in `testcase_list_users_sessions_seats`: the agetty session opens and closes at once | No override. `testcase_ambient_caps` and all of `testcase_background` now pass, which is further than the C oracle reaches in this VM (it fails in `testcase_ambient_caps`), so the oracle cannot arbitrate environmental-vs-defect here |
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
- **83-BTRFS did not exist upstream**, so its wrapper was removed on 2026-07-28.
  systemd 260.2 ships TEST-80, 81, 82, 84, 86, 87, 88 and 89, and no TEST-83 of any
  kind. The wrapper (added 2026-04-23, `name = "83-BTRFS"` and nothing else) therefore
  ran `./TEST-83-BTRFS.sh` and got `No such file or directory`, failing permanently for
  a reason unrelated to rust. `expectedSkip` could not have rescued it either: the
  harness only creates `/skipped` on exit **77** (`testsuite.nix:846`), and a missing
  script exits 127. A sweep of every wrapper's `name =` against the 66 upstream
  families found this was the **only** such phantom registration. If systemd is ever
  bumped to a version that ships TEST-83-BTRFS, re-add the wrapper.

## Known flakes

These fail intermittently on an unchanged tree. Treat a failure here as a flake only
after re-running the *same* configuration, never on the strength of the name alone.

- **03-JOBS**, at `systemctl stop --job-mode=replace-irreversibly unstoppable.service`,
  reporting `Stop failed: ... reached its timeout` roughly 750 traced lines in.
  Measured 2026-07-28: red once, then green on an immediate re-run of the identical
  tree. Timing-sensitive by construction, since the subtest is about a service that
  refuses to stop.
- **07-PID1** `testcase_delegate_subgroup_control`.
- **16-EXTEND-TIMEOUT** and **59-RELOADING-RESTART**, since the inc 2 slice-3
  tree (60471c75): roughly every other run, with a shared signature of
  dispatcher event application stalling for seconds under simultaneous-start
  load (START-TIMEOUT kmsg at exactly the base deadline for notify units
  whose READY/extends were already sent, and 59's notify-reload script
  SIGKILLed mid-trap reading ExecMainStatus 9). This is NOT dismissed as
  environment: it is a timing margin the slice narrowed, tracked as a real
  regression with the analysis and fix options in the task ledger
  (dispatcher throughput vs activation writers; the blind second escalation
  stage should also verify the process survived SIGTERM before failing).

Beware a trap when baselining one of these. Reverting the change and rebuilding often
resolves to a **cached** derivation, so nix returns `exit=0` in a handful of lines
without booting a VM. That proves the parent passed at *some* point, which is exactly
what a flaky test looks like too, so it cannot separate a flake from a regression.
Re-run the *failing* configuration instead: failed derivations are not cached, so that
run genuinely executes.

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
