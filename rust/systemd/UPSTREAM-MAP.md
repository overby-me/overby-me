# UPSTREAM-MAP: rust-systemd vs upstream systemd v258

Synthesis of an 8-subsystem divergence audit (2026-07-15). Upstream references point into the
nixpkgs systemd v258 source (`nix eval --raw nixpkgs#systemd.src`, currently
`/nix/store/kban61mm86a1nhq05rzg771n4l7qfjgw-source`); rust references point into
`rust/systemd/crates/libsystemd/src/` unless another crate is named. Every divergence was
adversarially verified against both sources except those marked `*` (unverified, hit the
verification budget cap); minor-severity rows without a marker were produced by the same analysis
but received lighter verification. Severities reflect post-verification calibration.

**Related documents:** `PLAN.md` (integration-test status: ~403/404 tests passing
pre-nixpkgs-drift, failing tests categorized by root cause) and `docs/plan/status.md`
(port phases + directive coverage, 414/429 = 97%). This map was derived from source
comparison alone; its per-fix "test impact" claims must be reconciled against PLAN.md's
recorded test status before acting on them - several tests named as affected are recorded
there as passing. Where this map documents divergences in areas status.md marks complete,
the map audits and supersedes the status claim.

Subsystem ID prefixes used throughout: ML (manager-loop), JT (jobs-transactions),
ST (service-types), SR (sigchld-reaping), SA (socket-activation), RR (reload-reexec),
DM (device-mount), LM (locking-model).

---

## 1. Architecture divergence summary

### The two models

**Upstream: one thread, one event loop, a job engine.**

- `manager_loop` (manager.c:3271) blocks only inside `sd_event_run`. `job_run_and_invalidate`
  (job.c:906) calls `unit_start`/`unit_stop` (job.c:849), which *initiate* (fork, set state) and
  return immediately. Completion always arrives as an event: SIGCHLD (manager.c:2828, WNOWAIT
  peek, one zombie per iteration), sd_notify (priority NOTIFY = -5, dispatched strictly before
  SIGCHLD = -4), exec-fd EOF, inotify, mountinfo, NameOwnerChanged, timers.
- The run queue dispatches at `SD_EVENT_PRIORITY_IDLE+1` (manager.h:674), so IPC and event intake
  always outrank job launches. `job_finish_and_invalidate` re-queues every After=/Before= neighbor
  on completion (job.c:1090-1113); waiting jobs are parked at zero cost and woken only by events,
  forever, with no polling and no expiry.
- Reload and reexec set `m->objective` and execute at a guaranteed quiescent point after the loop
  unwinds (dbus-manager.c:1577, main.c:2235), with the client reply parked and flushed afterward.
- There are no locks. There is nothing to starve.

**rust-systemd: 32 worker threads plus a global RwLock.**

- A 32-thread activation pool (activate.rs:893) whose workers hold the global RuntimeInfo READ
  guard across the *entire* `activate_unit` call (activate.rs:1114-1129, flagged in-tree by the
  SLOW-ACTIVATE TEMP diagnostic). Per-service state write locks are additionally held across whole
  start/stop sequences (unit.rs:1412-1434, 1591-1614).
- Writers deliberately never block-wait (to dodge glibc's writer-preferring rwlock): they spin in
  `try_write` via `write_poisoned_nonblocking` (lock_ext.rs:82-97) with **no deadline**, so they
  succeed only in a moment of zero readers. Long reader holds therefore starve writers
  indefinitely; conversely a queued blocking writer would block all new readers. This trade is the
  mechanical root of the empirically observed daemon-reload wedges.
- Event-driven completion exists only partially: the DeferNotifyWait machinery
  (services.rs:997-1027, activate.rs:1271-1534) covers Type=notify/notify-reload/oneshot for
  pool/socket/trigger sources in stage 2. Everything else waits inline under the guard:
  Forking/Dbus/Exec starts, ExecCondition/StartPre/StartPost, ExecStop/StopPost, all restarts
  (hardcoded `ActivationSource::Regular`), mount(2)/umount2/swapon, and the dbus name wait.
- Wake-up on asynchronous completion is a patchwork of bounded pollers that permanently expire
  (goal redrive: 400 x 500ms then thread exit, service_manager.rs:320-355; coldplug retry:
  120 x 500ms, service_manager.rs:242-253), plus ad-hoc re-drives that target only the *static*
  boot goal.

### The invariant set rust-systemd must adopt (the "no-wedge contract")

- **I1 (lock hygiene).** No thread may hold the RuntimeInfo lock (read or write) or a per-service
  state write lock across any blocking operation: process waits (`wait_for_service`,
  `wait_for_helper_child`), sleep-polls, mount/umount/swapon syscalls, bus connects or bus-name
  waits, `accept()`, or blocking socket sends. Locks are short slices for state mutation only.
- **I2 (completion ownership).** Every unit put into Starting or Stopping has exactly one
  event-driven completion owner (deferred handler, reaper aftermath, notify handler,
  mountinfo/udev watcher) that also enforces the applicable timeout. Nothing may be initiated and
  left ownerless (the current timer/path trigger paths violate this, LM-5).
- **I3 (event-driven progression).** Completion of any unit re-dispatches its waiters (Before=
  chain, required_by/bound_by, the *current* goal, not the static boot target) through a central
  `unit_state_changed(id)` hook feeding one persistent dispatcher queue. Pollers may remain as
  safety nets but never expire and are never the sole wake-up path.
- **I4 (single reaper).** The SIGCHLD thread is the only `wait()`er in the manager process. Every
  spawned child is either registered in pid_table before its exit can be processed, or its exit is
  parked as an unclaimed record and later claimed, never discarded as a "rerooted orphan".
- **I5 (quiescent mutation).** Table-wide mutation (daemon-reload, reexec serialization) runs only
  at an engineered quiescent point: set an objective flag, refuse new job dispatch, drain
  in-flight activation (ACTIVATION_DEPTH == 0, deferred waits parked), then mutate under one brief
  write hold. The client reply is parked and flushed afterward, mirroring upstream's
  `pending_reload_message`.
- **I6 (bounded acquisition, graceful storms).** No unbounded spin in PID 1: writer acquisition
  gets a deadline plus escalation; a pending-writer gate at reader choke points lets writers
  converge; thread-per-event patterns (per exit, per connection, per deferred wait) become small
  fixed pools with bounded queues.

I1 + I2 mechanically kill the writer-starvation hang class. I3 kills the silent-stall class
(expired redrives, parked-forever units). I5 makes reload/reexec safe against mid-transition
mutation. I4 kills the exit-race class (ECHILD, lost helper exits).

---

## 2. Prioritized fix plan

Ordered by impact on the known daemon-reload hang and on test unblocking. Fix 1-3 retire the hang
class; 4 is a trivial deterministic-failure fix; the rest are correctness parity.

## 1. Universal deferred start completion (all service types, all activation sources)

- Addresses: ML-1, JT-1, ST-1, ST-2, ST-3, ST-6, ST-9*, SR-1 (start half), SA-1, LM-1.
- Approach: extend the `defer_wait` predicate (services.rs:997-1027) to Forking, Dbus and Exec,
  and to every source including Regular/restart (the pool already promotes Regular to
  DeferNotifyWait at activate.rs:1105-1108; make `reactivate_unit` (deactivate.rs:134) and the
  control paths pass DeferNotifyWait too, and have all such callers invoke the existing
  `spawn_deferred_service_wait`, activate.rs:1545, exactly as socket_activation.rs:378 does).
  Move each type's completion signal into `deferred_notify_wait_and_dispatch`: Forking = pid_table
  ServiceExited plus PIDFile pickup; Dbus = a `bus_name_good` flag maintained by one persistent
  manager bus connection subscribed to NameOwnerChanged; Exec = the exec-fd CLOEXEC-pipe protocol
  (byte-then-EOF from exec_helper), removing the fixed 500ms poll. Run ExecCondition/ExecStartPre
  before taking, or after dropping, the RuntimeInfo guard. Enforce as a lint-style invariant:
  `wait_for_service`/helper waits are never called while a RuntimeInfo guard is held.
- Test impact: 07-pid1-daemon-reload, 07-pid1-forking-pidfile, 07-pid1-concurrency,
  07-pid1-exec-start-pre-post*, 07-pid1-type-exec-parallel (can restore upstream's -P 0),
  03-jobs restart sections, 59-reloading-restart, 01-basic boot flakes. Directly retires the
  residual instances of the known hang.
- Risk: medium. Must land with or after fix 7's ExecStartPost repair (the deferred path currently
  skips it, ST-4) or the bug widens; initrd switch-root ordering needs an explicit gate on
  Before=initrd-switch-root.target oneshots before deleting the initrd exception.

## 2. Take helper-process and stop-path waits off the global lock

- Addresses: ML-4, JT-2, SR-1 (stop half), LM-3, LM-6.
- Approach: helper waits need only pid_table, not the unit table, so `run_cmd`/`run_all_cmds`
  callers drop the RuntimeInfo guard first. `service_exit_handler` returns a `PendingStopPost`
  (mirroring the existing PendingRestart/PendingTrigger pattern, service_exit_handler.rs:298-357)
  so ExecStopPost runs after the guard is released. `Command::Stop`/`Restart`/isolate collect
  unit ids under a short read hold, then drop and reacquire per unit and between the kill and wait
  phases (the lock-pulse idiom already exists at control.rs:8887-8940); the shutdown kill loop
  reacquires per iteration.
- Test impact: 07-pid1-exec-stop-post(-failure), 07-pid1-kill-mode, 23-unit-file-execstoppost,
  daemon-reload concurrent with stop jobs, udev-event/socket-activation writer stalls.
- Risk: low-medium; preserve stop-post-before-final-state ordering.

## 3. Quiescent-point daemon-reload (model upstream's m->objective)

- Addresses: ML-2, ML-8*, RR-1, RR-5, LM-2; enables RR-8.
- Approach: set a global RELOAD_REQUESTED flag; activation dispatch stops submitting new work
  while set; wait until ACTIVATION_DEPTH == 0 (activate.rs:817-835, already exists) and deferred
  waits are parked; hoist the `load_all_units_no_prune` disk rescan *out* of the write critical
  section (control.rs:9705-9711); take the write lock once for the in-memory merge; re-run
  generators into fresh output dirs and refresh `unit_dirs` (upstream re-runs generators on every
  reload, manager.c:3593); re-instantiate loaded template instances from current fragments; park
  the reply (per-connection threads make a condvar trivial) and send it only after completion.
  Give `write_poisoned_nonblocking` a deadline plus kmsg escalation everywhere; add a small
  reload/reexec token bucket.
- Test impact: 07-pid1-daemon-reload, 04-journal-reload, TEST-15 dropin reload cases, fstab-edit
  reload flows, NixOS switch-to-configuration; eliminates mid-transition config swaps
  (control.rs:9842-9846 currently mutates units whose start is in flight).
- Risk: medium; the drain needs a timeout fallback so a stuck activation cannot deadlock reload.

## 4. Timer/path triggers must spawn the deferred completion handler (quick win)

- Addresses: LM-5.
- Approach: after `activate_unit` returns in `fire_timer_target`/`fire_path_target`
  (timer_scheduler.rs:552-624, path_watcher.rs:938-1015), if status == Starting call
  `spawn_deferred_service_wait`, exactly as socket_activation.rs:377-382. Two call sites (plus
  udev_event.rs:1151).
- Test impact: 63-path (test63-glob is deterministically stuck in `activating` today); any
  timer-started Type=notify daemon; restores TimeoutStartSec enforcement on trigger paths.
- Risk: minimal.

## 5. Mount/swap operations become non-blocking with enforced TimeoutSec

- Addresses: DM-1, DM-7*.
- Approach: Mount/Swap `activate` sets Starting under a short slice and hands mount(2)/umount2/
  swapon to a worker holding no locks; better, spawn util-linux mount(8)/umount(8) as a child
  completing via the SIGCHLD path, which simultaneously fixes Type=auto and helper filesystems
  (nfs/cifs), enables the umount retry loop (upstream retries 32x) and the
  clean-exit-but-not-mounted protocol check. Arm the existing timer_scheduler with the parsed but
  currently dead `TimeoutSec` (default 90s) and escalate SIGTERM then SIGKILL on expiry
  (mount.c:1647-1719 model).
- Test impact: removes the unbounded D-state wedge (NFS/hung device under the read lock, the worst
  remaining member of the hang class since the syscall cannot even be killed); prerequisite for
  un-stubbing 10-mount and 60-mount-ratelimit; TEST-07/08 mount-adjacent subtests.
- Risk: medium-high; initrd mount path must remain synchronously safe.

## 6. Single-reaper discipline and exit-race hardening

- Addresses: ML-5, SR-2, SR-3, SR-11.
- Approach: route every helper child through fork + `PidEntry::Helper` registration instead of
  `Command::status()` (ExecReload at control.rs:8126-8138, systemd-sleep, systemd-shutdown,
  reboot/poweroff helpers); the crate already implements the safe pattern at services.rs:1393-1399
  plus `wait_for_helper_child`. When the reaper finds no pid_table entry, park an
  `UnclaimedExited(code)` record for later claim instead of discarding (signal_handler.rs:134-140).
  In the notify wait, on observing ServiceExited first drain the notification socket non-blocking
  and only then return ExitBeforeNotify; make the exit handler do a one-shot drain for notify
  units still in Starting. Replace thread-per-exit (signal_handler.rs:149-154) with a small fixed
  worker pool over a queue; garbage-collect ServiceExited/unclaimed entries after a bounded age.
- Test impact: 07-pid1-exec-reload(-failure), sleep verbs, flaky ExecStartPre=/bin/true timeout
  failures, notify races in 80-notifyaccess/74-aux-utils areas; stops unbounded thread growth in
  PID 1 during restart storms.
- Risk: low.

## 7. Deferred-path semantics repair (ExecStartPost, notify protocol failure, timeout escalation)

- Addresses: ST-4, ST-5, ST-8*.
- Approach: in `deferred_notify_wait_and_dispatch`, after readiness and before flipping Started,
  take the state write lock briefly and run `run_poststart` (failure = run_poststop +
  StoppedUnexpected), and re-read PIDFile where applicable. In the exit handler, force a protocol
  failure (StoppedUnexpected, restart-policy treats as failure) when a Type=notify/notify-reload
  main exits in Starting with `signaled_ready == false`, with upstream's RemainAfterExit +
  NotifyAccess!=main carve-out (service.c:4374-4384). Start timeouts send `KillSignal` (default
  SIGTERM) first, SIGKILL after TimeoutStopSec, and latch result = timeout; honor
  TimeoutStartFailureMode= if parsed.
- Test impact: re-enable the patched-out TEST-23 ExecStopPost block (`(! systemd-run --wait
  -p Type=notify ... true)`), exec-order assertions, 16-extend-timeout family, PIDFile-by-StartPost
  pattern.
- Risk: low.

## 8. Exit-status classification parity

- Addresses: SR-4, SR-8, SR-10, ST-13, ST-10*.
- Approach: non-oneshot main-exit classification computes
  `is_success || is_clean_signal` (EXIT_CLEAN_DAEMON: SIGHUP/SIGINT/SIGTERM/SIGPIPE), removing the
  current inconsistency where restart logic and OnSuccess treat these as clean but the unit shows
  failed. Consult the `-` prefix in the exit handler for main-process exits. Pass
  SuccessExitStatus into `run_cmd` for user-configured control commands. Fire SuccessAction= for
  exec-less oneshots (upstream's systemd-poweroff.service shape, service.c:2553-2569).
- Test impact: `systemctl kill` / `kill -TERM $MAINPID` leaves inactive not failed (remove the
  74-aux-utils-kill-signal reset-failed workaround), 23-unit-file-success-failure edges,
  TEST-82-family poweroff/reboot unit flows.
- Risk: low.

## 9. Central state-changed hook: event-driven requeue, dependency-failure propagation, retroactive deps

- Addresses: ML-6, JT-3, JT-5, JT-12, DM-3 partially (late-device dependents), ML-9* (indirect).
- Approach: add `unit_state_changed(id)` called from every transition site (exit handler, READY=1
  in the notify handler, deferred dispatch, udev plug, mount completion). It looks up reverse
  After= peers plus required_by/bound_by waiters *restricted to the current activation set* and
  enqueues them on one persistent Mutex<VecDeque>+Condvar dispatcher drained by a long-lived
  worker calling `activate_units_recursive`. Delete the 400-tick redrive expiry and the fixpoint
  sweep; keep the redrive as an uncapped safety net targeting the *current* goal. On transition
  to StoppedUnexpected, walk required_by/bound_by transitively and fail Starting/NeverStarted
  members of the activation set with a DependencyError, trigger their OnFailure=, then invoke the
  hook (replaces the 30s Restarting-wait heuristics in Command::Start, control.rs:8713-8795).
  Retroactive start/stop deps (unit.c:2301-2334) fold into the same hook.
- Test impact: post-200s stalls (isolate after boot settles, late devices, slow CI), TEST-07
  on-failure dependent cases, fast dependency-failure errors instead of 30s waits, boot-target
  determinism.
- Risk: medium; the activation-set restriction must be kept to avoid pulling in bystanders.

## 10. Minimal job objects

- Addresses: ML-3, JT-4, JT-8*, ML-10, JT-6.
- Approach: one `Option<Job>` slot per unit in CommonState (global-counter id, JobType
  {Start,Stop,Restart,Reload,VerifyActive,Nop}, JobState {Waiting,Running}): set on enqueue,
  cleared on completion, consulted for merging (start-into-start no-op, restart absorbs start,
  stop cancels start) and for restart's in-place STOP-then-START patch. Control Start/Stop reply
  with the job id/path immediately; completion paths emit JobRemoved over the existing zbus
  server; the systemctl shim waits client-side (upstream's split). Back list-jobs and a real
  cancel off the slot; replace the three deactivation atomics. Arm JobTimeoutSec on the existing
  timer wheel with `execute_unit_action(job_timeout_action)` on expiry, and map
  RunCmdError/UnitOperationErrorReason onto distinct result codes.
- Test impact: 03-jobs unpatched, D-Bus clients with the 25s sd-bus timeout, JobRemoved waiters,
  07-pid1-concurrency; rescue/emergency JobTimeoutAction flows.
- Risk: medium; large surface but each piece is additive.

## 11. Device wait timeout (DefaultDeviceTimeoutSec analog)

- Addresses: DM-3, remainder of JT-6.
- Approach: when `unstarted_deps` parks a unit on a Device dependency, register
  (unit, device, deadline = now + JobRunningTimeoutSec-or-90s) with timer_scheduler; on expiry, if
  the device is still not Started, set Stopped(StoppedUnexpected, DependencyError) with "Timed out
  waiting for device", letting OnFailure/emergency wiring take over. Wire the fstab-generator's
  already-emitted JobRunningTimeoutSec drop-ins (currently parsed and ignored end-to-end, making
  x-systemd.device-timeout a no-op).
- Test impact: missing-root/missing-device scenarios fail into emergency.target after 90s instead
  of hanging forever; TEST-24-adjacent device-timeout semantics.
- Risk: low.

## 12. Reexec fidelity: socket fds, quiescence, alias migration

- Addresses: RR-2, RR-3, RR-6, RR-7.
- Approach: extend `serialize_reexec_state` to dump `fd_store` globals as
  (socket_unit, fdname, fd) with FD_CLOEXEC cleared and reinsert them before the
  socket-activation thread starts (restore already runs early enough); fall back to re-running
  SocketState.activate for Started sockets whose fds did not survive. Before execve: reject new
  activations, drain workers bounded, serialize Starting as Starting plus the current goal, and
  in the new image re-drive via set_active_goal + one activate_needed_units pass instead of
  skipping activation. During reload merge and reexec restore, resolve names through the fresh
  alias map and migrate status/pid/timestamps to the new canonical unit (covers
  TEST-07-PID1.alias-rename for both reload and reexec). Add timestamps/n_restarts columns to the
  state file.
- Test impact: socket activation surviving daemon-reexec (latent today, real-system breakage for
  sshd.socket-style units), alias-rename subtests once wired, reexec racing NixOS activation.
- Risk: medium.

## 13. PID 1 signal contract

- Addresses: ML-7*, RR-4.
- Approach: register SIGHUP and route to the deferred-reload objective; SIGTERM =
  `daemon_reexec` (sysvinit compat; today it powers the machine off); SIGINT = start
  ctrl-alt-del.target via switch_target on a thread; SIGQUIT = crash handling. Renumber the
  RTMIN dispatch to upstream's table (+13..16 immediate halt/poweroff/reboot/kexec, +24 exit,
  +25 reexec; today +13 is reexec and +25 is missing).
- Test impact: TEST-01 reexec-by-signal checks, ctrl-alt-del behavior, container supervisors and
  systemd-shutdown fallbacks that send these signals.
- Risk: low but behavior-visible; coordinate with the test harness.

## 14. Socket-activation correctness batch

- Addresses: SA-2, SA-3, SA-4, SA-5, SA-6, SA-10.
- Approach: decrement `active_accept_connections` when instances die (or derive the count from
  live instances, self-healing); set O_NONBLOCK on all listen fds, treat EAGAIN as spurious, and
  have `wait_for_socket` report (UnitId, fd) so accept targets the fd that fired (today only
  `fds.first()` is ever accepted, and a dual-stack Accept=yes unit wedges the whole
  socket-activation thread); replace select() with poll() (FD_SETSIZE panic risk); implement the
  SOCKET_DEFERRED state machine for DeferTrigger= plus the `listening` substate (07-pid1-socket-defer
  fails today at its first assertion); fix trigger/poll-limit defaults (20/2s non-accept, poll
  150/15 per 2s); close fds on trigger-limit failure and add the missing SocketResult variants;
  execute socket Exec* command lists around open/close.
- Test impact: 07-pid1-socket-defer (currently failing in-suite), poll-limit defaults,
  long-running Accept=yes workloads (cumulative-64 lockout), dual-stack sockets, large
  deployments (>1024 fds).
- Risk: medium; several independent small fixes.

## 15. Mount/device model completion

- Addresses: DM-2, DM-4, DM-5, DM-6, DM-8*, DM-9..12.
- Approach: add a mountinfo watcher thread (POLLPRI on /proc/self/mountinfo, full-table diff,
  synthesize/flip .mount units, brief write slices only) to un-stub 10-mount/60-mount-ratelimit;
  in `apply_device_inactive` propagate stops to `bound_by` dependents via the existing background
  deactivation thread (device unplug currently never stops BindsTo= mounts/services);
  serialize PID1's udev intake through one bounded channel and make udevd queue-and-retry instead
  of dropping on its 2s timeout, plus a periodic /run/udev/data catchup; gate the stat()-based
  device-plugging/mknod/symlink fallbacks to the initrd and to targeted uevent writes; native
  fstab units gain Requires=+After= on their What= device, fsck instances, correct
  local-fs-pre direction and x-systemd.automount handling; scope ID_PROCESSING deferral to
  devlink units; perpetual -.mount and RequiresMountsFor= re-resolution on mount insertion.
- Test impact: TEST-10, TEST-60 (both stubbed exit-77 today), storage hotplug, TEST-17 timing
  edges, stage-2 fstab boots on slow disks.
- Risk: medium-high (largest new-code surface); mountinfo watcher first, it unblocks the most.

## 16. Hygiene and storm-proofing batch

- Addresses: LM-7, LM-8, ML-11, SR-9, ST-11, ST-12, SA-7, SA-8, SA-9, RR-8, RR-9, RR-10, JT-9,
  JT-10, JT-11.
- Approach: delete the four TEMP diagnostics; sweep raw `.read()/.unwrap()` to the poison-recovering
  lock_ext variants (add a CI grep); cap control-connection and worker threads with fixed pools;
  notification handler uses try_write with re-queue instead of a blocking per-service state write
  (LM-4, and move hot notify fields to the existing lock-free Common mirrors); OOM-kill detection
  via memory.events baselines; MAINPID cgroup validation and STOPPING=1 handling; idle-pipe for
  Type=idle once the dispatcher exists; stop-ordering by Before=/After= within stop sets;
  per-transaction cycle handling; instance naming/per-source keying; FDSTORE preserve/dedup;
  Reloading signals and reload rate limit; close fds of removed socket units on reload.
- Risk: low, incremental.

---

## 3. Subsystem-by-subsystem mapping

Severity: **C** = critical, **M** = major, **m** = minor. `*` = claim not adversarially verified.
Refs are abbreviated; upstream files are under src/core/ unless noted.

### 3.1 manager-loop

**Matches upstream (verified confirmations):** dedicated signal thread updates pid_table without
the RuntimeInfo lock (fixed the documented 3-way deadlock); shutdown runs on its own thread with
Before/After ordering and hands off to systemd-shutdown; SIGRTMIN+N mapping mostly correct
(+0..+6, +13..+16, +22..+24); daemon-reexec serializes PIDs/statuses/stored-fds/freezer and skips
re-activation; startup ordering (mount API fs, generators, enumerate/coldplug, default target)
matches upstream's shape; stage-2 DeferNotifyWait mitigations cover notify/oneshot pool/socket/
trigger starts; --no-block variants reply immediately; start rate limiting (5/10s) enforced;
isolate closure + IgnoreOnIsolate honored; all writers use try_write polling (conscious
workaround, incomplete).

| ID | Divergence | Sev | Upstream | rust-systemd | Fix |
|----|-----------|-----|----------|--------------|-----|
| ML-1 | Job starts block pool threads up to TimeoutStartSec holding the RuntimeInfo read lock | C | unit_start returns immediately; run queue at IDLE+1 never blocks (job.c:849,906; manager.c:2542) | Workers hold read guard across activate_unit (activate.rs:1114-1129); Forking/Dbus/Exec and all Regular-source starts wait inline (services.rs:997-1042, fork_parent.rs:240-498); restarts always Regular (deactivate.rs:134) | Plan fix 1 |
| ML-2 | daemon-reload inline with unbounded try_write spin; no quiescent point | C | method_reload parks reply, sets objective, reload runs post-loop (dbus-manager.c:1577, main.c:2193, manager.c:3593) | LoadAllNew spins try_write forever (control.rs:9705, lock_ext.rs:82-97); can mutate config of mid-start units (control.rs:9842-9846); no Reloading signals, rate limit, or reply parking | Plan fix 3 |
| ML-3 | Start/Stop reply only after completion; no job objects, JobNew/JobRemoved, modes | M | bus_unit_queue_job replies with job path before running; JobRemoved async (dbus-unit.c:1955, dbus-job.c:302) | execute_command runs whole activation synchronously (control.rs:8584-9074); D-Bus returns fake job "/", n_jobs()=0 (dbus_server.rs:815-935); modes partially honored on the socket path only | Plan fix 10 |
| ML-4 | Stop/Restart/isolate hold the read lock across recursive deactivation incl. ExecStop | M | unit_stop returns immediately; completion via SIGCHLD/cgroup (job.c:849) | One guard across the whole multi-unit stop loop (control.rs:9326-9411); Restart holds guards across deactivate + reactivate (control.rs:8181-8217); default stop timeout is 1s (note: upstream 90s), so worst case needs configured timeouts | Plan fix 2 |
| ML-5 | waitpid(-1) drain races in-process Command::status() waits; reap-before-inspect | M | Single reaper, WNOWAIT peek, one zombie/iteration (manager.c:2828) | Signal thread reaps all; ExecReload/sleep/shutdown/reboot use Command::status() with no pid_table entry: ECHILD race, exits discarded as orphans (signal_handler.rs:85-160, control.rs:8126-8138); thread-per-exit unbounded | Plan fix 6 |
| ML-6 | No event-driven requeue; bounded sweeps + expiring redrive pollers | M | job_finish_and_invalidate re-queues all After/Before neighbors forever (job.c:1004) | Fixpoint sweep (activate.rs:897-952); goal redrive 400x500ms then permanent exit (service_manager.rs:320-355); ad-hoc re-drives target only the static boot goal; post-expiry isolate goals stall forever | Plan fix 9 |
| ML-7* | SIGTERM/SIGINT power off instead of reexec/ctrl-alt-del | M | SIGTERM=reexec, SIGINT=ctrl-alt-del.target (manager.c:2942) | All three map to shutdown_sequence(Poweroff) (signal_handler.rs:162-170) | Plan fix 13 |
| ML-8* | daemon-reload does not re-run generators | M | manager_reload flushes and re-runs generators (manager.c:3593) | Generators run once at boot (service_manager.rs:130-131); reload re-parses stale dirs | Plan fix 3 |
| ML-9* | On-demand loading needs write lock; hard-fails after 30s | M | Load queue drained per loop iteration; can never fail on contention (manager.c:2338) | find_or_load_unit try_write 30s then error (control.rs:1567-1583) | Healed by fix 1; optionally load queue / sharded map |
| ML-10 | JobTimeoutSec/Action parsed, never enforced; no job-result taxonomy | m | job timers + errno-coded results (job.c:1118-1189) | Config fields dead (unit.rs:2421-2427); failures collapse to Stopped + strings | Plan fixes 10, 11 |
| ML-11 | No fairness budgets or storm protection | m | Per-iteration budgets, loop ratelimit (manager.c:111-114, 3271) | 0666 socket, thread per connection/exit/wait, no bounds (control.rs:69-76, 10186-10201) | Plan fix 16 |

### 3.2 jobs-transactions

**Matches upstream:** pure After= without pull neither activates nor blocks unless the peer is in
the activation set (incl. the rescue.target case); condition failure = success with before-chain
dispatch; assertion failure = failed start; already-active early-return prevents double-fork;
ignore-dependencies bypasses the graph; isolate approximates transaction_apply with IgnoreOnIsolate;
TEST-03 irreversible-job scenario emulated; Upholds= retry loop; OnFailure split job-walk vs exit
handler; deferred poller enforces TimeoutStartSec+EXTEND_TIMEOUT with try_read hygiene; start rate
limiting; TEST-03 list-jobs/transaction-cycle assertions satisfied by emulations.

| ID | Divergence | Sev | Upstream | rust-systemd | Fix |
|----|-----------|-----|----------|--------------|-----|
| JT-1 | Activation threads park inside starts holding the read lock (inline wait residuals) | C | unit_start only initiates (job.c:906, unit.c:1933) | DeferNotifyWait covers only notify/oneshot for some sources; Forking/Dbus always inline; restart + ignore-deps use Regular under a guard (services.rs:997-1027, control.rs:8699-8708); racing caller also blocks on the per-service write lock (unit.rs:1412-1434) | Plan fix 1 |
| JT-2 | systemctl stop: whole recursive deactivation under the read lock | M | Async JOB_STOP; PID1 never blocks (job.c:849, unit.c:2725) | One guard across the loop; ExecStop waits inline (control.rs:9326-9411, services.rs:1748-1809); starves writers (reload spins, find_or_load 30s failures), other readers proceed | Plan fix 2 |
| JT-3 | Event completion replaced by expiring pollers; blocked units can stall forever | C | Waiting jobs woken only by events, no expiry (job.c:1090-1113, unit.c:2624-2633) | Redrive `for _ in 0..400` then permanent exit; deferred pollers dispatch only own before-chain; re-drives target the static boot goal; post-expiry isolate stalls confirmed | Plan fix 9 |
| JT-4 | No job objects: no slot, merging, conflict cancel, cancel verb | M | One installed job/unit; merge tables; JOB_CANCELED (job.c:232, 408-474) | Three atomics cover only StartNoBlock-during-Stop; list-jobs synthesized, racy pa.clear(); ids regenerated per query (control.rs:7132-7156, 9164-9409) | Plan fix 10 |
| JT-5 | No JOB_DEPENDENCY propagation: dependents of failed Requires= park forever | M | job_fail_dependencies + dependent OnFailure (job.c:987, 1044-1086) | Dependents stay NeverStarted, no error, no OnFailure; Command::Start papers over with 30s heuristics (control.rs:8713-8795); 07-pid1-on-failure replaced with weaker custom subtest | Plan fix 9 |
| JT-6 | JobTimeoutSec/JobRunningTimeoutSec/Action parsed, never enforced; boot hangs vs emergency action | M | job_start_timer/job_dispatch_timer + emergency_action; device 90s default (job.c:1118-1180, device.c:129) | Zero consumers; fstab-generator emits JobRunningTimeoutSec drop-ins that are parsed and ignored (x-systemd.device-timeout is a no-op end to end) | Plan fixes 10, 11 |
| JT-7* | No transaction: immediate side effects, inline Conflicts=, Requisite folded into Requires | M | Prospective graph, 10-step reduction, atomic apply (transaction.c:711, 941) | Flood-fill + immediate execution; Conflicts stopped inline under the guard (activate.rs:472-504); Requisite= wrongly pulls the dep up | Split Requisite verify-only; conflict set computed up front, job-mode=fail refusal |
| JT-8* | Restart is hand-rolled stop+start, blocking under the guard | M | JOB_RESTART patched in place to START; late merges (job.c:1024-1035) | Manual stop of reverse deps + inline reactivate + heuristic re-activation (control.rs:8170-8290) | Plan fixes 1, 10 |
| JT-9 | Stop propagation ignores Before=/After= edges | m | Stops ordered inverse to starts (job.c:1707) | Requirement-edge recursion only (deactivate.rs:42-69) | Plan fix 16 |
| JT-10 | Cycles broken permanently at load time by arbitrary edge | m | Per-transaction anchor-mattering breaks (transaction.c:340-493) | break_dependency_cycles mutates the table; TEST-03 passes via journal emulation | Store deleted edges; re-apply per subgraph |
| JT-11 | No run-queue priority or idle deferral | m | Prioq by CPU weight; idle-pipe (manager.c:868) | Unordered pool dispatch; idle deferral removed to dodge a deadlock (activate.rs:869-884) | Heap-keyed dispatcher after fix 9 |
| JT-12 | Retroactive dep transactions only as scattered special cases | m | retroactively_start/stop_dependencies (unit.c:2301-2334) | Fragments in udev/exit-handler paths only | Fold into fix 9's hook |

### 3.3 service-types

**Matches upstream:** simple/idle readiness semantics incl. child-side EXIT_GROUP/EXIT_USER;
oneshot serial multi-ExecStart with reload-aware re-read; EXIT_CLEAN_COMMAND for oneshot; only
oneshot may lack ExecStart; inline notify ExitBeforeNotify failure; NotifyAccess enforced in both
delivery paths incl. NOTIFYACCESS= override; ExecCondition exit-code semantics; TimeoutStartSec
90s default + infinity + per-command re-arm; EXTEND_TIMEOUT_USEC in all three waits;
RemainAfterExit; RuntimeMaxSec; ExitType=cgroup; `-` prefix on helper paths; forking MAINPID=
adoption; full Restart= decision table with RestartSteps ramp, run outside the lock; the deferred
handler itself is lock-hygienic (the template for extension).

| ID | Divergence | Sev | Upstream | rust-systemd | Fix |
|----|-----------|-----|----------|--------------|-----|
| ST-1 | Entire start chain synchronous under RuntimeInfo read + state write locks | C | Chain of non-blocking state entries (service.c:3129,2659,2623,2526,2457) | run_exec_condition/prestart/fork/wait/poststart on one thread, both locks held (services.rs:731-1072; activate.rs:1114-1129; unit.rs:1412-1414) | Plan fixes 1, 2 |
| ST-2 | Forking readiness always inline, polls to TimeoutStartSec | M | Control-pid SIGCHLD event (service.c:4504-4533) | fork_parent.rs:320-463 sleep-polls under both locks; never in defer set | Plan fix 1 |
| ST-3 | Dbus: blocking bus-name wait with locks; no NameOwnerChanged after start (name loss never stops the unit) | C | NameOwnerChanged drives start_post and service_good() teardown (service.c:5427, 2400) | New blocking zbus conn + 50ms poll to 90s under locks; hard-fails if bus not up; no post-start subscription anywhere; no bus-based MainPID pickup (fork_parent.rs:464-498, dbus_wait.rs:55-123) | Plan fix 1 + persistent bus conn + name-loss deactivation |
| ST-4 | Deferred path (stage-2 default) skips ExecStartPost entirely | M | All readiness funnels through service_enter_start_post (service.c:2457-2483) | DeferredNotifyWait returns before run_poststart; deferred dispatch never runs it or re-reads PIDFile (services.rs:1025-1062, activate.rs:1450-1503) | Plan fix 7 |
| ST-5 | Notify clean exit before READY=1 = success on the deferred path | M | SERVICE_FAILURE_PROTOCOL (service.c:4374-4384) | Exit handler judges by exit code only; exit 0 becomes cleanly inactive, OnSuccess even fires; TEST-23 block patched out to hide it | Plan fix 7 |
| ST-6 | Type=exec: fixed 500ms poll for exit 203 instead of exec-fd | M | Byte-then-CLOEXEC-EOF, zero latency (service.c:4070-4116) | Every healthy exec start pays 500ms under both locks; misclassifies legit 203 and slow exec failures (fork_parent.rs:192-239); 07-pid1-type-exec-parallel already downgraded from -P 0 to -P 3 | Plan fix 1 (exec-fd) |
| ST-7* | PIDFile: 1s retry then untracked pid=None; no inotify; GuessMainPID dead; no cgroup validation | M | inotify wait in START until timeout; suitable-pid checks; cgroup scan (service.c:3974-4068, 1203-1309) | read_pid_file 20x50ms then Started untracked (fork_parent.rs:15-30, 385-442); parsed pid trusted blindly | inotify via path_watcher; cgroup validation; GuessMainPID scan |
| ST-8* | Start timeout = immediate SIGKILL; no TERM-first, no failure mode, result not "timeout" | M | TimeoutStartFailureMode + SIGTERM then SIGKILL, result latched (service.c:4642) | RunCmdError::Timeout goes straight to kill_all_remaining_processes (services.rs:1075-1160) | Plan fix 7 |
| ST-9* | Restart path re-enters inline waits: reactivate hardcodes Regular | C | Restart is just the normal event-driven chain | Every restart of notify/oneshot/forking/dbus blocks under a guard (deactivate.rs:118-135, service_exit_handler.rs:497-530); crash-looping Restart=always reopens the 90s starvation window each cycle | Plan fix 1 |
| ST-10* | Exec-less oneshot never fires SuccessAction= | M | Fake START transition fires it (service.c:2553-2569); systemd-poweroff.service shape | Empty exec list = Started; no exit handler ever runs (services.rs:1044-1048) | Plan fix 8 |
| ST-11 | Type=idle = pure alias of simple (no idle pipe, no state mapping) | m | Idle-pipe dance in child, ACTIVE state table (service.c:98-130) | Simple | Idle identical (fork_parent.rs:189-191) |
| ST-12 | sd_notify gaps: MAINPID unvalidated, STOPPING=1 inert, no tag precedence | m | Cgroup/uid gating; STOPPING>READY>RELOADING (service.c:4927-5133) | Any live pid accepted; stopping flag never consumed (notification_handler.rs:881-918) | Plan fix 16 |
| ST-13 | Clean-signal death marked failed; no pidfile re-read on main death | m | EXIT_CLEAN_DAEMON; live-pid handoff cancels death (service.c:4261-4290) | is_success excludes clean signals in status path only (service_exit_handler.rs:1452-1467) | Plan fix 8 |

### 3.4 sigchld-reaping

**Matches upstream:** all children always reaped incl. orphans; reaping never blocks and does no
async-signal-context work; ExecCondition exit semantics; `-` prefix for helpers and last-ExecStart;
Restart= decision table; manual-stop suppression; RestartSec + reactivation outside the lock;
oneshot chain-abort; unconditional ExecStopPost; ExitType=cgroup drain; MAINPID adoption during
forking wait; ExecMainStatus/timestamps recorded; helper exits cause no state change; RestartSteps
ramp; OnSuccess/OnFailure with MONITOR_* env after the guard drops.

| ID | Divergence | Sev | Upstream | rust-systemd | Fix |
|----|-----------|-----|----------|--------------|-----|
| SR-1 | Exit/start pipeline blocks under the read lock (StopPost in exit handler, helper waits, forking/dbus starts) | C | Command chaining is exit-driven; sigchld dispatch never blocks (service.c:4326-4334, 4471-4479) | ExecStopPost runs under read guard + state write lock (service_exit_handler.rs:1274-1291, 840-856); helper waits park under the guard; INLINE-WAIT diagnostic in-tree | Plan fixes 1, 2 |
| SR-2 | READY=1/exec confirmation not consumed before the same pid's exit | M | NOTIFY(-5)/EXEC_FD(-6) dispatch before SIGCHLD(-4) (manager.h:658-674) | Notify wait checks ServiceExited before recv, guaranteeing the wrong order in the race (fork_parent.rs:62-118); exit handler races the notify thread (service_exit_handler.rs:1244-1263) | Plan fix 6 |
| SR-3 | Reap-before-register race silently discards helper exits | M | Watch registered before reaper can run (single thread) | PidEntry::Helper inserted after spawn (services.rs:1393-1399); reaper drops unknown pids (signal_handler.rs:134-140); full-timeout hang + stray kill of recycled pid | Plan fix 6 |
| SR-4 | Clean daemon signals (HUP/INT/TERM/PIPE) mark the unit failed | M | EXIT_CLEAN_DAEMON = inactive (service.c:4252-4278) | Status path counts only configured signals; restart/OnSuccess disagree with the shown state; 74-aux-utils carries a reset-failed workaround | Plan fix 8 |
| SR-5 | Forking PIDFile 1s retry then untracked (dup of ST-7 evidence) | M | demand_pid_file inotify, stays in START (service.c:4012-4068) | Warn + pid=None + Started; daemon exit never noticed (fork_parent.rs:15-30, 385-442); breaks the mainpid3 must-fail case of 07-pid1-main-PID-change | With ST-7 fix |
| SR-6* | No pidfile re-read on main exit: daemon re-exec treated as death | M | New valid pid adopted, no state transition (service.c:4280-4293) | No pid_file logic in the exit handler at all | Re-read + cgroup-validated adoption at top of non-oneshot path |
| SR-7* | Default ExitType=main deactivates on clean main exit with live cgroup | M | service_good() keeps RUNNING while cgroup non-empty, for all services (service.c:2400-2455) | Only explicit ExitType=cgroup gets the drain-poll; default kills remaining children (service_exit_handler.rs:637-732, 1412-1439) | Check cgroup_has_processes on clean exit; reuse drain-poll |
| SR-8 | SuccessExitStatus= not consulted for control commands / intermediate ExecStarts | m | Consulted for user-configured control commands (service.c:4252-4278) | run_cmd is exit-0-only (services.rs:1409-1423) | Plan fix 8 |
| SR-9 | No WNOWAIT peek, no OOM-kill detection, OOMPolicy unenforced | m | Peek keeps /proc readable; unit_check_oom (manager.c:2835-2888) | Immediate reap; nothing reads memory.events (signal_handler.rs:885-917) | memory.events baseline diff (fix 16) |
| SR-10 | `-` prefix not applied to main-exit classification (simple/notify/exec) | m | IGNORE_FAILURE forces success (service.c:4295-4324) | Honored at spawn/oneshot/forking but not in the exit handler | Plan fix 8 |
| SR-11 | Exclusive pid map, thread-per-exit, never-purged ServiceExited entries | m | Non-exclusive watches, one child per iteration | pid_table 1:1; unbounded threads; stale-entry growth (runtime_info.rs:96, signal_handler.rs:106-154) | Plan fix 6 |

### 3.5 socket-activation

**Matches upstream:** Accept=no dedup + RUNNING approximation with level-triggered re-check; polling
gated on state with fds kept open; FD handover order (socket, FDSTORE, OpenFile) with
LISTEN_FDS/FDNAMES; trigger_notify-equivalent re-arm on every deactivation path; FlushPending=yes;
TriggerLimit when configured (issue-2467 assertions); PollLimit when configured; Accept=yes basics
(MaxConnections=64, per-uid source cap, fd transfer + close); stopped-guard bypass for on-demand
restart; deferred handler lock-safety; OnFailure for failed socket starts (#35635); FDSTORE core
protocol incl. reexec survival.

| ID | Divergence | Sev | Upstream | rust-systemd | Fix |
|----|-----------|-----|----------|--------------|-----|
| SA-1 | Residual inline wait for socket-activated Forking/Dbus (all types in initrd) under the read lock | M | Fire-and-forget: job queued, SOCKET_RUNNING immediately (socket.c:2466-2499) | Single socket thread holds the guard through wait_for_service for Forking/Dbus; initrd defers nothing for SocketActivation (services.rs:997-1026, socket_activation.rs:315-425) | Plan fix 1 |
| SA-2 | Accept=yes connection counter never decremented on instance exit | M | socket_connection_unref from service_release_socket_fd (socket.c:3458, service.c:284) | Only incremented + failure-path decrement; after 64 cumulative accepts the socket refuses forever; NConnections permanently wrong (socket_activation.rs:495-594) | Plan fix 14 (or derive from live instances) |
| SA-3 | Blocking accept() on blocking fds; only the first port fd ever accepted | M | Per-port nonblocking io sources, EAGAIN = spurious (socket.c:3102-3210) | No O_NONBLOCK anywhere (sockets/mod.rs:710-717); wait_for_socket loses the fd identity; dual-stack Accept=yes wedges the whole socket thread on the wrong fd (socket_activation.rs:135-139, 453-462) | Plan fix 14 |
| SA-4 | DeferTrigger= parsed, unimplemented; also no `listening` substate | M | SOCKET_DEFERRED + JOB_LENIENT + DeferTriggerMaxSec (socket.c:2384-2493) | No runtime reader of defer_trigger; DeferTriggerMaxSec not even parsed; conflicts killed instead of deferred; 07-pid1-socket-defer fails in-suite today (first assertion: expected `listening`, got `running`) | Plan fix 14 |
| SA-5 | Trigger/poll limit defaults diverge (200 for Accept=no; poll disabled) | m | 20/2s non-accept; poll 150/15 per 2s (socket.c:327-336) | 200 for all; poll 0=off, comments wrongly claim parity (socket_activation.rs:124-132) | Plan fix 14 |
| SA-6 | Failed socket keeps fds open; SocketResult lacks variants | m | stop_pre to dead closes fds; SERVICE_START_LIMIT_HIT (socket.c:3472+) | Clients hang in backlog; only Success | TriggerLimitHit (socket_activation.rs:230-258, unit.rs:982-986) |
| SA-7 | Instance naming / per-source keying / NAccepted approximations | m | n_accepted+peer name; IP-or-uid keying; default per-source off (socket.c:1443-1465) | Counter-only names; uid-only keying (TCP never limited); per-source defaults to 64; NAccepted bumped for Accept=no too; live-instance fd stapling leak | Plan fix 16 |
| SA-8 | No stop-pending suppression before activation | m | Refuse/flush when a stop is queued (socket.c:2444-2453) | Race can leave a socket activated=true with no re-armer (socket_activation.rs:318-345, unit.rs:519-537) | Plan fix 16 |
| SA-9 | FDSTORE preserve/dedup/FDPOLL/pinned-substate gaps | m | same_fd dedup, EPOLLHUP auto-remove, Preserve lifecycle (service.c:577-681) | Effectively always preserve=yes; no dedup; dead fds passed to services | Plan fix 16 |
| SA-10 | select() FD_SETSIZE cap; socket Exec* never executed; ratelimit not serialized over reexec | m | epoll; control-command chain with timeouts (socket.c:2612-3331) | fd >= 1024 panics the socket thread; ExecStartPre/Post parsed but skipped | Plan fix 14 |

### 3.6 reload-reexec

**Matches upstream:** transient units survive reload (and reexec via /run/systemd/transient in the
search path); running units with removed files survive (issue-3171); Reexecute never replies,
disconnect = completion, restore runs before the control socket reopens; reload reply only after
completion; FDSTORE fds survive reexec; freezer state survives reexec (TEST-38); set-environment
survives both; deserialized state overlays only pre-existing units; restored PIDs validated;
reexec re-runs generators via the full startup path; NeedDaemonReload is a real mtime check;
write_poisoned_nonblocking genuinely prevents the queued-writer reader-blockade half of the hang
class; device unit state preserved across reexec (ID_PROCESSING demotion guard).

| ID | Divergence | Sev | Upstream | rust-systemd | Fix |
|----|-----------|-----|----------|--------------|-----|
| RR-1 | Reload inline in a request thread requiring total lock quiescence; disk rescan inside the write section | C | Objective + post-loop quiescent reload + parked reply (dbus-manager.c:1577, main.c:2235) | Unbounded try_write spin (control.rs:9705); load_all_units_no_prune inside the critical section; live-lock risk with 32 overlapping readers; D-Bus variant blocks the zbus executor | Plan fix 3 |
| RR-2 | Reexec closes all .socket listening fds while restoring sockets as active | M | FDSet serialization + manager_distribute_fds; seamless listening (main.c:1215-1261, manager.c:1853) | fd_store fds are CLOEXEC and never serialized; sockets restored Started with no listener; nothing re-runs open_all (signal_handler.rs:595-618, 729-736). Latent for the current suite; real-system breakage | Plan fix 12 |
| RR-3 | Reexec has no quiescent point; mid-start units fabricated active; pending starts dropped | M | Reexec after loop unwinds; jobs serialized and resumed with absolute deadlines (job.c:1233, 1356-1400) | execve from control/signal thread mid-activation; Starting mapped to "Started", Stopping resurrected; new image skips activation/goal/redrive (signal_handler.rs:332-393, 571-578; service_manager.rs:286-300) | Plan fix 12 |
| RR-4 | PID 1 signal contract inverted (SIGHUP absent, SIGTERM=poweroff, RT table shifted, +25 missing) | M | SIGHUP=reload, SIGTERM=reexec, +13..16=immediate halt/poweroff/reboot/kexec, +25=reexec (manager.c:555-569, 2986, 3038) | SIGHUP unregistered (kernel discards for PID1); TERM/INT/QUIT all poweroff; +13=reexec, +25 absent (service_manager.rs:173-179, signal_handler.rs:162-263) | Plan fix 13 |
| RR-5 | Reload skips manager-config re-parse, external generator re-run, unreferenced-instance re-parse | m | Full re-parse + generator re-run + re-enumerate (main.c:2243-2260, manager.c:3620-3648) | Only ManagerEnvironment= re-read; symlink-referenced instances DO get re-instantiated (claimed TEST-15 breakage refuted); real residue: external generators never re-run, ad-hoc instances stale, DefaultTimeoutStartSec not parsed at all | Plan fix 3 |
| RR-6 | No state migration when a name becomes an alias of a new canonical unit; no orphan synthesis | M | State follows current symlink resolution; orphaned-r<id> units adopt cgroups (manager-serialize.c:243-397) | Old object kept under old name, new canonical NeverStarted; reexec restore matches exact names only; fails TEST-07-PID1.alias-rename when wired (control.rs:9750-9763, signal_handler.rs:679-701) | Plan fix 12 |
| RR-7 | Reexec fidelity: timestamps, ratelimit counters, results, main-vs-control pid lost | m | Full unit/service serialization with absolute timer re-arm (unit-serialize.c:63-150) | Only name/pid/type/tri-state/freezer/stored-fds (signal_handler.rs:539-621); reexec resets StartLimitBurst | Plan fix 12 |
| RR-8 | No reload/reexec rate limit | m | reload_reexec_ratelimit, serialized across reload (dbus-manager.c:1596-1645) | None (control.rs:9699, 5391) | Plan fix 3 |
| RR-9 | No Reloading signals, no RELOADING=1/READY=1 notify | m | manager_send_reloading + deferred Reloading(false) (manager.c:3928) | zbus server emits no signals (dbus_server.rs:1049-1060) | Plan fix 16 |
| RR-10 | Removed socket units leak bound fds after reload | m | Freed with the unit (manager.c:1630) | fd_store untouched; select loop skips them; ports stay bound (socket_activation.rs:692-696) | Plan fix 16 |

### 3.7 device-mount

**Matches upstream:** devices never started by the job machinery (park-don't-block matches
wait_only); udev event path is the sole device activator; SYSTEMD_READY=0 both directions +
ID_RENAMING; MOVE/DEVPATH_OLD handling (rename-netif passes); unit-name fan-out (syspath, subsystem
alias, devnode, symlinks, SYSTEMD_ALIAS) with stale-alias retirement; SYSTEMD_WANTS re-derived per
event with reverse edges + on-demand template instantiation; startup enumeration from
/run/udev/data with reexec preservation; StopWhenUnneeded gc on disappearance; udev handler itself
never blocks on activation; fstab basics (swap units, noauto/nofail, _netdev, x-*/comment
filtering; initrd uses the real systemd-fstab-generator so it is upstream-faithful by
construction); already-mounted short-circuit; the mountinfo-before-SIGCHLD ordering hazard cannot
trigger today because no helper processes exist (must be re-established when they are added).

| ID | Divergence | Sev | Upstream | rust-systemd | Fix |
|----|-----------|-----|----------|--------------|-----|
| DM-1 | Mount/swap blocking syscalls inline under the read lock, no helper, TimeoutSec dead config | C | /bin/mount spawned, timer-armed state machine, umount retried 32x (mount.c:1163-1273, 1647-1719) | nix mount(2)/umount2/swapon on the pool thread with the guard held (unit.rs:1855-2262); an NFS/D-state mount wedges PID1 with no possible recovery (syscall in the manager's own thread) | Plan fix 5 |
| DM-2 | udev-to-PID1 transport: lossy fire-and-forget RPC, no post-boot catchup | M | Netlink monitor on the loop, kernel SEQNUM order, enumerate+found-mask catchup, re-enumeration on reload (device.c:1029-1234) | Per-event UnixStream with 2s timeout, silent drop on failure (udevd lib.rs:5498-5570); per-device ordering held by busy_devpaths in normal operation; catchup only at startup + 60s; a dropped event leaves the table stale until reboot | Plan fix 15 |
| DM-3 | No device wait timeout: units park forever on missing devices | M | job_running_timeout = 90s, then fail + emergency path (device.c:119-134, job.c:874) | No deadline anywhere; JobRunningTimeoutSec parsed but not even copied to runtime; x-systemd.device-timeout a no-op; boot hangs silently | Plan fix 11 |
| DM-4 | PID1 substitutes udev: stat()-plugging, mknod, by-label/uuid symlinks, mass uevent re-trigger | M | Only udev events/enumeration plug devices; PID1 never creates nodes/symlinks (device.c:752-784, 183-201) | 60s fallback loop plugs referenced units on path existence with fabricated tags, mknods block nodes, writes symlinks udev cannot clean, floods `add` into all of /sys (udev_event.rs:314-637); runs in stage 2 too | Plan fix 15 (initrd gate + targeted triggers + TENTATIVE) |
| DM-5 | Device removal does not stop BindsTo= dependents | M | DEVICE_DEAD -> unit_notify -> retroactive stop of BoundBy (device.c:163-224) | apply_device_inactive never touches bound_by; the module doc claims otherwise; device-bound mounts and NIC-bound services outlive their device (udev_event.rs:1243-1309) | Plan fix 15 |
| DM-6 | No /proc/self/mountinfo monitor: external (un)mounts invisible; no synthesis; no TENTATIVE; no ratelimit | M | libmount monitor + full-table diff + device_found_node + 5/1s ratelimit with -EAGAIN job requeue (mount.c:2071-2309) | Nothing; point-in-time /proc/mounts scan at explicit (de)activation only; TEST-10 and TEST-60 stubbed exit-77; requires-mounts-for weakened | Plan fix 15 |
| DM-7* | mount(2) direct cannot do Type=auto or helper filesystems; success = syscall return; no umount retry | M | mount(8) probes blkid, invokes mount.<type> helpers; clean-exit-without-entry = PROTOCOL failure (mount.c:1513-1645) | fs_type None gives kernel EINVAL for auto; nfs/cifs impossible; single umount2 (unit.rs:1984-2089) | Plan fix 5 |
| DM-8* | Native fstab units: no device deps, no fsck, inverted local-fs-pre for /, x-systemd.automount eager-mounted | M | Requires/After on What= device, fsck@ instances, correct ordering, .automount generation (mount.c:313-465) | generate_fstab_mount_units wires only target edges (directory_deps.rs:1848-2227); stage-2 mounts race their devices | Plan fix 15 |
| DM-9 | ID_PROCESSING deferral applied to all unit names, not just devlink units | m | Only devlink setup skips while processing (device.c:786-811) | Everything deferred (udev_event.rs:84-162); shipped test still passes | Plan fix 15 |
| DM-10 | Readiness uses merged sticky tags; any symlink is unit-worthy | m | Requires CURRENT `systemd` tag; refuses /dev/block-char (device.c:752-784) | Once tagged always tagged (udevd lib.rs:5509-5531); untagged symlinked devices get units | Plan fix 16 |
| DM-11 | SYSTEMD_WANTS: no auto-instancing of bare templates; whole-target redrive per event | m | Escaped syspath as instance; targeted JOB_STARTs (device.c:543-627) | Bare `foo@.service` silently starts nothing; initrd redrive walks the whole graph per event | Plan fix 15 |
| DM-12 | No perpetual -.mount; RequiresMountsFor= expanded to hard Requires at parse | m | Perpetual -.mount; mount-side re-resolution (mount.c:2019-2042, 243-311) | Phantom deps ignored at runtime; late mounts uncovered; upstream subtest replaced with weaker script | Plan fix 15 |

### 3.8 locking-model

**Matches upstream (structural confirmations):** journal lifecycle logging is non-blocking under
locks (fixed the journal-send deadlock); SIGCHLD reaping decoupled from RuntimeInfo; the stage-2
deferral machinery is structurally the right convergence template and is itself lock-hygienic;
no writer ever block-waits (the documented 3-way deadlock is eliminated, traded for the starvation
issue); multi-unit status locks taken in a global sorted order; ExecReload drops the lock; poll
loops in Command::Start pulse the lock; background loops consistently yield via try_read; control
socket accepts each connection on its own thread, limiting blast radius.

| ID | Divergence | Sev | Upstream | rust-systemd | Fix |
|----|-----------|-----|----------|--------------|-----|
| LM-1 | Activation pool holds the read lock across blocking service starts | C | No locks; initiate-and-return (manager.c:3271, job.c:906) | ri_guard held across ExecStartPre + inline per-type waits; OnFailure triggers via Regular also pin readers (activate.rs:1114, 1623-1628; services.rs:997-1034; unit.rs:1412) | Plan fixes 1, 2 |
| LM-2 | Reload's global write via unbounded try_write spin: no reader gate, no deadline, no quiescence | C | Objective at a quiescent point (dbus-manager.c:1577-1615) | Zero-reader requirement vs 32 overlapping workers = livelock; activation_in_flight exists but only gates the redrive thread (control.rs:9705, lock_ext.rs:82, service_manager.rs:331) | Plan fix 3 + pending-writer gate (I6) |
| LM-3 | Control commands run whole stop+start inline under the read lock instead of enqueueing jobs | M | bus_unit_queue_job + JobRemoved (dbus-unit.c:2048, job.c:174) | Stop/Restart/isolate/StopNoBlock/exit-handler restart all hold guards across ExecStop and start halves (control.rs:9326, 8181-8216; deactivate.rs:134; service_exit_handler.rs:509-526) | Plan fixes 2, 10 |
| LM-4 | Notification thread blocks on per-service state write locks held across entire start/stop | M | Single loop, no per-unit locks (manager_dispatch_notify_fd) | Blocking write_poisoned per ready fd (notification_handler.rs:177) while unit.rs:1412/1591 hold the lock for whole operations; one slow stop freezes manager-wide readiness and pins a read guard | try_write + re-queue; lock-free hot fields (fix 16) |
| LM-5 | Timer/path triggers defer the wait but never spawn the completion handler: stuck-in-Starting | M | Every start is a job with a guaranteed owner (job.c:1140, 961) | fire_timer/path_target leave DeferredNotifyWait ownerless: no READY poll, no timeout, no before-chain; 63-path test63-glob deterministically fails (timer_scheduler.rs:588, path_watcher.rs:966) | Plan fix 4 |
| LM-6 | Shutdown holds one blocking read lock across stopping every unit + exec | m | Job-engine-driven shutdown transaction | Raw .read() held through the whole kill loop (shutdown.rs:309-361) | Plan fix 2 |
| LM-7 | Committed "TEMP diagnostic (remove before commit)" instrumentation on hot paths | m | n/a | SLOW-ACTIVATE, INLINE-WAIT, spawn_starting_probe, per-reap REAP kmsg | Plan fix 16 |
| LM-8 | Raw .read()/.unwrap() sites bypass poison recovery in PID1 paths | m | n/a (crate's own invariant, lock_ext.rs:1-16) | varlink.rs:220, udev_event.rs:1675/1686, shutdown.rs:305/309, assorted status locks: one panic cascades to manager death | Plan fix 16 + CI grep |

---

## 4. Coverage gaps

Subsystems and areas **not yet analyzed** against upstream (each is a candidate for the same
divergence-audit treatment before its tests are trusted):

- **cgroup management**: hierarchy setup, controller delegation, cgroup-empty event handling
  (upstream's cgroup inotify/PSI paths), cgroup attribute application (CPUWeight, MemoryMax, ...),
  Delegate=, systemd-oomd integration. Only ExitType=cgroup and the missing OOM-kill detection
  were touched. Test areas: TEST-19, TEST-32, TEST-55, TEST-56.
- **exec context / sandboxing**: exec_helper vs exec-invoke.c (namespaces, mount propagation,
  User/Group/DynamicUser, capabilities, seccomp, RootDirectory/RootImage, LoadCredential/
  SetCredential, PrivateTmp and friends). Test areas: TEST-07 exec-context, TEST-23 large parts,
  TEST-54 credentials.
- **timer units**: OnCalendar math, Persistent=, AccuracySec/RandomizedDelaySec, WakeSystem,
  OnClockChange. Only trigger-dispatch locking was reviewed. Test area: TEST-24-like timers.
- **path units**: inotify semantics, glob handling, PathChanged vs PathModified fidelity. Only
  trigger locking reviewed. Test area: TEST-63 beyond the stuck-glob case.
- **automount units**: the autofs protocol is entirely unexamined.
- **swap details**: priority handling, /proc/swaps monitoring, deactivation ordering (only
  swapon-under-lock covered).
- **scope/slice units and transient units** (StartTransientUnit surface, logind session scopes,
  systemd-run parity).
- **journald and stream handling**: journald itself, stdout/stderr stream lifecycle, rate
  limiting, forwarding (only the socket-send-under-lock deadlock and 04-journal-reload
  interaction covered).
- **D-Bus API completeness**: property coverage, UnitNew/UnitRemoved/PropertiesChanged signals,
  polkit authorization, Subscribe semantics (noted as no-ops), TEST-21 dfuzzer surface.
- **user manager instance** (`systemd --user`), PAM sessions, logind interplay.
- **watchdog**: hardware watchdog and WatchdogSec full semantics (RuntimeMaxSec confirmed only).
- **condition/assert matrix**: full Condition*/Assert* evaluation fidelity (only
  outcome-on-failure semantics verified).
- **install/enable/preset/mask**: unit-file state machine, symlink management, TEST-15 alias
  loading beyond what reload analysis touched.
- **resource limits application** (rlimits, ulimit inheritance) beyond DefaultLimitNOFILE
  readback.
- **environment generators** (noted absent; never analyzed as a subsystem).
- **initrd flow details**: switch-root sequencing, soft-reboot (TEST-82), initrd-cleanup beyond
  the no-block confirmations.
- **emergency_action full semantics** (execute_unit_action exists; only partially examined via
  the missing job-timeout wiring).
- **security posture of the control plane**: the 0666 control socket has no peer-credential
  checks for state-mutating commands (flagged under ML-11 but not audited as a subsystem).
- **sd_notify extensions**: BARRIER=1, WATCHDOG_USEC runtime updates, MONOTONIC_USEC handling.

**Verification debt**: 13 divergences remain unverified (marked `*` above: ML-7/8/9, JT-7/8,
ST-7/8/9/10, SR-6/7, DM-7/8) and most minor rows received lighter scrutiny; re-verify each against
both sources before implementing its fix, since several verified claims required correction
(severity recalibrations, refuted impact claims, stale in-code comments).

---

## Not yet mapped (coverage gaps)

- **cgroup lifecycle and cgroup-empty tracking**: upstream treats cgroup.events "populated→0" as the authoritative unit-death/stop-completion signal (and prunes cgroups on GC); rust-systemd has no populated watcher in platform/cgroups/cgroup2.rs and infers death from pid_table alone, so straggler children under KillMode=process/mixed or forking daemons leave state wrong - affects 07-pid1-kill-mode, stop-completion and restart correctness suite-wide.
- **Scope units do not exist**: the `Specific` enum (units/unit.rs:132) has Service/Socket/Target/Slice/Mount/Swap/Timer/Path/Device but no Scope, yet `systemd-run --scope`, logind session scopes via pam_systemd, and TEST-19-family cgroup tests all require them; the map never mentions the missing type.
- **Transient-unit property surface (systemd-run depth)**: StartTransientUnit exists (dbus_server.rs:1112) via a variant→text conversion, but its property coverage, `--wait`/`--pty` (StandardInput socket), ExitType, aux-unit array, and CollectMode GC are unaudited - nearly every upstream TEST-*.sh drives itself with systemd-run, so gaps here fail tests attributed to other subsystems.
- **Exec context / sandboxing has no audit subsystem**: exec_helper.rs implements ProtectSystem/PrivateTmp/mount-ns but User/Group/DynamicUser, capabilities, seccomp, RestrictNamespaces, RuntimeDirectory/StateDirectory and rlimits are uncovered by the 8 prefixes - 07-pid1-exec-context.nix alone is 50K of assertions, plus dedicated private-network/-users/-pids, protect-hostname/-control-groups, delegate-namespaces and 05-rlimits tests.
- **Dependency-type semantics beyond failure propagation**: PartOf= stop/restart propagation, Conflicts= enforcement (one lone reference in activate.rs:476), Requisite=, and PropagatesReloadTo= are unmapped, and StopWhenUnneeded= is parsed but referenced nowhere in activation/deactivation code (silently dead) - 07-pid1-service-dependencies, TEST-03 and isolate correctness depend on these.
- **Timer unit semantics**: timer_scheduler.rs + calendar_spec.rs (~100K) are unaudited except the trigger-completion fix - OnCalendar= correctness, Persistent= stamp files, AccuracySec/RandomizedDelaySec, OnUnitActive rebasing, and timer state across reload/reexec (absent from the RR serialization list) all matter for timer tests and Persistent-across-boot cases.
- **Path unit semantics beyond the 63-glob bug**: path_watcher.rs (70K) is otherwise unmapped - PathChanged vs PathModified event mapping, DirectoryNotEmpty re-trigger, MakeDirectory=, trigger rate limiting, and inotify re-arm after the triggered unit stops drive the rest of TEST-63.
- **Journal integration is entirely unaudited**: the in-tree journald (journal/, ~130K), stdout/stderr stream attribution (_SYSTEMD_UNIT, _SYSTEMD_INVOCATION_ID), invocation-ID lifecycle, syslog forwarding and LogFilterPatterns back 15 separate 04-journal-* integration tests, none covered by any subsystem prefix.
- **User manager instances**: no `systemd --user` support exists (no user-instance code paths in service_manager.rs), yet user@.service, session bus, and `systemctl --user` appear throughout upstream test scripts (TEST-23 user cases, TEST-43 unprivileged-user tests).
- **D-Bus surface beyond jobs**: Subscribe/UnitNew/UnitRemoved/PropertiesChanged signal emission, GetUnitProcesses, SetEnvironment/UnsetAndSetEnvironment and property Get/Set fidelity are unmapped - upstream test scripts assert via busctl and `systemctl show`, and 07-pid1-set-environment plus sd-bus monitor flows fail without them.
- **Unit-file install and load-state machinery**: enable/disable/preset/mask/revert, [Install] Alias=/Also=, UnitFileState/LoadState reporting and drop-in precedence have dedicated tests (07-pid1-enable-disable, -is-enabled, -mask, -drop-in-override) and the whole TEST-15 loading matrix, but the map touches unit loading only in the reload context.
- **Automount units missing entirely**: no Automount variant in the type enum, so x-systemd.automount (mentioned in fix 15 as mere "handling") and the automount portions of TEST-10/mount flows cannot work at all; the map should state the type must be created, not adjusted.
- **Conditions/asserts breadth**: the map verifies only the condition-failure-equals-success job semantic, not the ~40 Condition*/Assert* predicate types (Virtualization, KernelVersion, Memory, Security, NeedsUpdate) or negation syntax - 07-pid1-condition-negation and -condition-virt test exactly this surface.
- **Unit GC / CollectMode / failed-unit retention**: no unit_gc analog or CollectMode=inactive-or-failed means transient units and failed remnants accumulate in the table, breaking `systemctl status`/list-units assertions after systemd-run-heavy scripts and interacting badly with reset-failed workarounds already noted in fix 8.
- **Emergency and limit actions**: StartLimitAction=, unit- and manager-scope FailureAction=/SuccessAction= (beyond fix 8's exec-less-oneshot case) and CtrlAltDelBurstAction are unmapped - TEST-82-family reboot/poweroff flows and rescue-escalation paths rely on them.
