# Architecture: the concurrency model

oxidized-systemd's PID 1 does not share upstream systemd's execution model. This is the
single largest source of behavioural divergence and of every hang the project has hit.
Read this before debugging a wedge.

Derived from an 8-subsystem audit against systemd v258 (2026-07-15), re-checked against
the tree on 2026-07-26. Only still-open items are kept here; the fixes that landed are
in `CHANGELOG.md`.

## The two models

**Upstream: one thread, one event loop, a job engine.**

`manager_loop` (manager.c:3271) blocks only inside `sd_event_run`.
`job_run_and_invalidate` (job.c:906) calls `unit_start`/`unit_stop`, which initiate
(fork, set state) and return immediately. Completion always arrives as an event:
SIGCHLD, sd_notify, exec-fd EOF, inotify, mountinfo, NameOwnerChanged, timers. The
notify source has priority -5 and is dispatched strictly before SIGCHLD at -4. The run
queue dispatches at `SD_EVENT_PRIORITY_IDLE+1`, so IPC and event intake always outrank
job launches. `job_finish_and_invalidate` re-queues every `After=`/`Before=` neighbour
on completion, so waiting jobs park at zero cost and are woken only by events. Reload
and reexec set `m->objective` and run at a guaranteed quiescent point after the loop
unwinds, with the client reply parked and flushed afterwards.

There are no locks. There is nothing to starve.

**oxidized-systemd: a worker pool plus a global RwLock.**

A 32-thread activation pool whose workers hold the global `RuntimeInfo` **read** guard
across the whole `activate_unit` call. Per-service state write locks are additionally
held across entire start and stop sequences. Writers deliberately never block-wait, to
dodge glibc's writer-preferring rwlock: they spin in `try_write`
(`lock_ext.rs:82-97`). Event-driven completion exists only for part of the space;
the rest waits inline under the guard.

## The no-wedge contract

Six invariants the implementation is converging on.

| | Invariant | Status |
|-|-----------|--------|
| **I1** | No thread holds the `RuntimeInfo` lock or a per-service state write lock across a blocking operation: process waits, sleep-polls, `mount`/`umount2`/`swapon`, bus connects, bus-name waits, `accept()`, blocking sends. | **Open.** `units/unitset_manipulation/activate.rs:1151` still holds the read guard across `activate_unit`, which waits on exec internally. |
| **I2** | Every unit put into Starting or Stopping has exactly one event-driven completion owner that also enforces the timeout. Nothing is initiated and left ownerless. | **Partial.** The deferred-wait machinery covers `Type=notify`, `notify-reload` and oneshot from pool, socket and trigger sources. Forking/dbus/exec starts, `ExecCondition`/`StartPre`/`StartPost`, `ExecStop`/`StopPost`, restarts, and mount syscalls still wait inline. |
| **I3** | Completion re-dispatches waiters through a central `unit_state_changed` hook feeding one persistent queue. Pollers may remain as safety nets but never expire and are never the sole wake-up path. | **Partial.** `spawn_active_goal_redrive` (`entrypoints/service_manager.rs:564`) now follows the *active* goal rather than the static boot target and yields to pending writers, but still expires after 400 ticks (about 200 s). |
| **I4** | The SIGCHLD thread is the only `wait()`er in the manager process. Every spawned child is registered in `pid_table` before its exit can be processed, or its exit is parked as an unclaimed record and later claimed, never discarded as a rerooted orphan. | Largely held. |
| **I5** | Table-wide mutation (daemon-reload, reexec serialization) runs only at an engineered quiescent point: set an objective flag, refuse new job dispatch, drain in-flight activation, then mutate under one brief write hold. The client reply is parked and flushed afterwards. | **Partial.** `ACTIVATION_DEPTH` (`activate.rs:817`) and a quiescent-point table swap exist; there is no single objective flag as in upstream. |
| **I6** | No unbounded spin in PID 1. Writer acquisition gets a deadline plus escalation; a pending-writer gate at reader choke points lets writers converge. | **Partial.** A 10 s `writer_pending()` gate was added at `activate.rs:1142`, but `write_poisoned_nonblocking` itself still loops on `try_write` with a 5 ms sleep and **no deadline**. |

I1 and I2 together kill the writer-starvation hang class. I3 kills the silent-stall
class (expired redrives, units parked forever). I5 makes reload and reexec safe against
mid-transition mutation. I4 kills the exit-race class (ECHILD, lost helper exits).

## Device units arrive over RPC, not netlink

Upstream's PID 1 speaks to udev directly. oxidized-systemd splits it: udevd processes the
event and then sends PID 1 a fire-and-forget `udev-event` JSON-RPC notification on the
control socket (`/run/systemd/oxidized-systemd-notify/control.socket`), carrying action,
sysfs path, devname, subsystem, the event environment, tags and symlinks
(`control/control.rs:235-242`, handled at `control.rs:1214`).

Two consequences worth knowing:

- Device units exist only as a side effect of those notifications, so after a fresh boot
  or a `daemon-reexec` they must be rebuilt by walking `/run/udev/data/*`. That rebuild
  consults the reexec status file so units that were active before the reexec come back
  active rather than being demoted to placeholders.
- `ID_PROCESSING=1`, `SYSTEMD_READY=0` and `ID_RENAMING=1` create the unit as an inactive
  placeholder, so `BindsTo=` dependents do not fire early.

## Debugging a suspected wedge

1. **Grep the VM log for `panic` and `unwrap` first.** A spawned thread that panics dies
   silently and looks exactly like a deadlock. This has cost multiple VM cycles.
2. Check whether the stuck operation is on the inline-wait list under I2. If so, the
   read guard is held and any concurrent `daemon-reload` will spin forever.
3. Check whether a redrive poller has expired (I3). Symptom: a unit sits in Starting
   with no further activity and no error.

## Remaining structural work

Ordered by how much it retires. The convergence design that implements this list
as one deliberate series (jobs plus a single dispatcher) is
[EVENT-LOOP.md](EVENT-LOOP.md); prefer it over item-by-item fixes here.

1. **Universal deferred start completion.** Extend the deferred handler to every service
   type and activation source, so nothing waits for a child while holding a lock. Closes
   I1 and I2 together.
2. **Take helper-process and stop-path waits off the global lock.** `ExecStartPre`,
   `ExecStartPost`, `ExecStop`, `ExecStopPost`, and restart paths currently block under
   the guard.
3. **A bounded writer acquisition.** Give `write_poisoned_nonblocking` a deadline and an
   escalation path instead of an unbounded spin.
4. **Minimal job objects.** oxidized-systemd resolves dependencies inline and has no job
   objects, so nothing is ever queued and `systemctl list-jobs` is always empty. This is
   the direct blocker for TEST-63-PATH's issue-24577 assertions and is a prerequisite
   for modelling upstream's requeue-on-completion behaviour.
5. **A central state-changed hook** feeding one persistent dispatcher queue, replacing
   the expiring pollers.
6. **Mount and swap operations become non-blocking** with an enforced `TimeoutSec`.
7. **Event-source rate limiting** for the mountinfo watcher, with mount start-jobs
   delayed while it is throttled. Blocks TEST-60-MOUNT-RATELIMIT.
