# Integration Test Plan

This document tracks the status of all upstream systemd integration tests run against rust-systemd via the NixOS VM test framework. The goal is to pass all tests without modifying test scripts.

Run a test: `nix build .#checks.x86_64-linux.rust-systemd-test-<name>`

## Test Status Summary

| Status | Count | Description |
|--------|-------|-------------|
| PASS | ~100+ | Tests passing reliably |
| FAIL (fixable) | ~10 | Failures in rust-systemd code that can be fixed |
| FAIL (architectural) | ~15 | Missing major features (D-Bus, Type=notify, udev) |
| Boot hang (transient) | ~5 | Non-deterministic QEMU boot failures (~30% rate) |
| Untested | ~150 | 74-aux-utils batch not yet fully run |

## Passing Tests

### Core (all pass)

- 01-basic, 03-jobs, 05-rlimits (both), 18-failureaction
- 26-systemctl, 30-onclockchange, 31-device-enumeration, 32-oompolicy
- 44-log-namespace, 45-timedate, 52-honorfirstshutdown, 54-creds
- 63-path, 66-device-isolation, 68-propagate-exit-status, 71-hostname
- 73-locale, 76-sysctl, 34-dynamicusermigrate, 53-timer, 53-issue-16347

### 04-JOURNAL (all 14 pass)

- bsod, cat, compress, corrupted-journals, fss, gatewayd, invocation
- journal-append, journal-corrupt, journal, logfilterpatterns, reload, remote, stopped-socket

### 07-PID1 (most pass, ~60 subtests)

- PASS: condition-negation, condition-virt, daemon-reload, drop-in-override, enable-disable, exec-context, exec-reload, exec-reload-failure, exec-start-pre-post, exec-start-pre-post-order, exec-stop-post, exec-stop-post-failure, exec-timestamps, forking-pidfile, is-enabled, issue-14566, issue-16115, issue-1981, issue-27953, issue-3166, issue-31752, issue-33672, issue-2467, issue-3171, kill-mode, list-units, mask, multi-exec-start, on-failure, ordering, poll-limit, pr-31351, private-network, private-users, protect-hostname, remain-after-exit, requires-mounts-for, resource-limits, restart-behavior, restart-on-failure-oneshot, runtime-directory, service-dependencies, set-environment, socket-pass-fds, standard-output-file, start-limit, state-logs-directory, success-exit-status, success-exit-status-custom, systemctl-kill, systemctl-restart, systemctl-show, systemctl-show-props, systemd-run-exit-code, target-ordering, timeout-stop, transient, type-exec-parallel, umask, wantedby-target, working-directory-custom, working-directory

### 19-CGROUP (all pass)

- cleanup-slice, exittype-cgroup, keyed-properties

### 22-TMPFILES (19 of 21 pass)

- 01-05, 07-21 all pass
- 06: persistent boot hang (transient)

### 23-UNIT-FILE (8 of 20 pass)

- PASS: exec-command-ex, execreload, joinsnamespace-of, oneshot-restart, percentj-wantedby, runtimedirectory, standardoutput, start-stop-no-reload, utmp, verify-unit-files

### 81-GENERATORS (4 of 5 pass)

- PASS: debug-generator, getty-generator, run-generator (after --man=no fix), system-update-generator
- FAIL: fstab-generator (D-Bus)

### 74-AUX-UTILS (63/151 tested, batch in progress)

- PASS (59): add-wants, after-timestamp, analyze-cal-iter, analyze-calendar, analyze-calendar-more, analyze-edge, analyze-standalone, analyze-timespan, analyze-timestamp, analyze-unit-paths, can-operations, cat, cat-content, cat-dropin, cat-dropin-content, cat-single, cg-options, cgtop, control-pid, daemon-reload, default-deps, delta, dep-props, description-check, detect-virt, enable-disable, enable-wantedby, enter-timestamp, environment, env-manager, escape, exec-main-props, exec-status, exec-timestamps, fragment-path, get-default, id128, invocation-id, is-active-states, is-enabled-patterns, is-queries, isolate-target, journal-json, journal-ops, journal-vacuum, kill-signal, list-dependencies, list-deps-advanced, list-deps-basic, list-failed, list-jobs, list-sockets, list-uf-pattern, list-unit-files, list-units, list-units-pattern, load-state, log-level, machine-id-setup
- FAIL (transient, pass on retry): cgls, delta, is-system-running
- Remaining ~88 tests being batched

## Failing Tests — Categorized by Root Cause

### 1. Missing D-Bus Interface (org.freedesktop.systemd1)

rust-systemd does not expose the `org.freedesktop.systemd1` D-Bus interface. Tests that use `busctl call` or rely on D-Bus monitoring fail with timeout or "No such file or directory".

**Affected tests:**

- 07-pid1-issue-34104
- 15-dropin
- 81-generators-fstab-generator

**Fix complexity:** Very high — requires implementing the full systemd D-Bus API. This is a major architectural feature.

### 2. Type=notify Service Lifecycle

Type=notify services expect the main process to send `READY=1` via sd_notify before transitioning to active. Several related notifications (RELOADING=1, STOPPING=1) also not handled.

**Affected tests:**

- 38-freezer (systemd-run --property Type=notify doesn't activate)
- 59-reloading-restart (RELOADING=1 notification)
- 80-notifyaccess (NotifyAccess=none not properly failing)
- 23-unit-file-success-failure (OnSuccess/OnFailure with Type=notify)
- 65-analyze (unit-shell test section, Type=notify interaction)

**Fix complexity:** Medium-high — Type=notify basics work for sd_notify READY=1, but advanced notification states (RELOADING, STOPPING) and proper timeout handling for services that never send READY=1 need implementation.

### 3. Missing Service Features

**Upholds= directive:**

- 23-unit-file-upholds
- Fix: Implement Upholds= unit dependency

**ExecStopPost execution:**

- 23-unit-file-execstoppost (systemd-run --wait hangs after ExecStopPost)
- Fix: Fix systemd-run --wait to properly track ExecStopPost completion

**StateDirectory/ConfigurationDirectory:**

- 23-unit-file-statedir
- Fix: Implement StateDirectory= and related directives

**OpenFile=:**

- 23-unit-file-openfile
- Fix: Implement OpenFile= directive for passing file descriptors

**ExtraFileDescriptors=:**

- 23-unit-file-extrafiledescriptors
- Fix: Implement ExtraFileDescriptors= directive

**BindPaths=/BindReadOnlyPaths= at runtime:**

- 23-unit-file-runtime-bind-paths
- Fix: Implement runtime bind mount operations

**PrivatePIDs=:**

- 07-pid1-private-pids
- Fix: Implement PID namespace isolation

**MessageQueue socket options:**

- 07-pid1-mqueue-ownership
- Fix: Implement POSIX message queue socket options

**systemctl freeze/thaw:**

- 38-freezer (also needs Type=notify)
- Fix: Implement cgroup freezer integration

### 4. User/Group Resolution Edge Cases

- 07-pid1-issue-30412: ExecMainStatus returns 0 instead of 203 for bad binary via socket activation
- 23-unit-file-type-exec: Group=idontexist error format differs
- 23-unit-file-whoami: Related user resolution issue

### 5. Exec Deserialization

- 07-pid1-exec-deserialization: ExecStart commands added after daemon-reload not picked up during running oneshot. Requires exec index tracking across daemon-reload.

### 6. udev Tests (all fail — C binary limitations)

All 23 udev tests fail because the C `udevadm` binary in the overlay lacks features (`udevadm cat` subcommand, etc.) or the tests exercise udev daemon behavior. These are NOT rust-systemd failures — they test the C udev subsystem.

**Affected:** All 17-udev-* tests (23 total)

**Fix:** Not fixable without reimplementing udevadm in Rust.

### 7. NixOS Framework Limitations

- 07-pid1-main-PID-change: Test expects to run AS a systemd service, but NixOS framework runs tests via shell
- 07-pid1-mount-invalid-chars: /etc/fstab is read-only on NixOS
- 16-extend-timeout: EXTEND_TIMEOUT_USEC service timeout interaction

### 8. systemd-analyze CLI Gaps

- 23-unit-file-clean-unit: Unknown failure (needs investigation)
- 78-sigqueue: Signal queuing behavior difference

### 9. Transient Boot Hangs

~30% of test runs experience a non-deterministic boot hang where backdoor.service never starts. Retrying usually succeeds.

**Affected (intermittent):** 07-pid1-user-group, 07-pid1-protect-control-groups, 07-pid1-issue-3171, 22-tmpfiles-06

## Prioritized Fix Plan

### Priority 1: Quick Wins (test config fixes, CLI flags)

- [x] Fix 17-udev test configs to match actual upstream subtest names
- [x] Fix 53-issue-16347 test config naming
- [x] Add --man=no and --recursive-errors support to systemd-analyze verify
- [x] Add lock-free atomic MainPID/ExecMainStatus/ExecMainPID
- [x] Add missing 23-unit-file subtest configs
- [ ] Run full 74-aux-utils batch (151 tests)

### Priority 2: Medium Effort Features

- [x] Implement Restart=on-failure for oneshot services (already works)
- [ ] Fix systemd-run --wait to track ExecStopPost properly
- [ ] Implement Upholds= dependency directive
- [ ] Implement StateDirectory=/ConfigurationDirectory=
- [ ] Fix ExecMainStatus for bad binary exec (issue-30412)
- [ ] Fix user/group resolution edge cases (type-exec, whoami)

### Priority 3: Major Features

- [ ] Complete Type=notify lifecycle (RELOADING, STOPPING notifications, READY timeout)
- [ ] Implement systemctl freeze/thaw (cgroup freezer)
- [ ] Implement PrivatePIDs= (PID namespace)
- [ ] Implement OpenFile= directive
- [ ] Implement ExtraFileDescriptors= directive
- [ ] Implement runtime BindPaths= / BindReadOnlyPaths=
- [ ] Implement MessageQueue socket options
- [ ] Implement exec deserialization across daemon-reload

### Priority 4: Architectural (very high effort)

- [ ] D-Bus interface (org.freedesktop.systemd1) — blocks ~3 tests
- [ ] Rust udevadm reimplementation — blocks 23 tests

## Architecture Notes

See [docs/plan/](docs/plan/) for the original phased implementation plan covering the full project structure and workspace layout.

Key architectural constraints:

- **Lock contention during oneshot activation:** The service state write-lock is held for the entire ExecStart execution. Property queries use try_read() with atomic fallbacks for MainPID/ExecMainStatus.
- **NixOS VM test framework:** Tests boot QEMU VMs with rust-systemd as PID 1. Tests run via `machine.execute()` shell commands, NOT as systemd services. This breaks tests that expect to run inside a service context.
- **Transient boot hangs:** Non-deterministic ~30% hang rate in QEMU. Caused by timing-dependent race conditions during early boot. Retrying usually works.
