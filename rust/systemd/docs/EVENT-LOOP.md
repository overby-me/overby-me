# Event-loop convergence: jobs and a single dispatcher

Design for strategy item 2 in [ROADMAP.md](ROADMAP.md): converge PID 1 on upstream
systemd's execution model deliberately, instead of retiring the
[ARCHITECTURE.md](ARCHITECTURE.md) invariants one at a time. When this series lands,
the I1-I6 table holds by construction rather than being defended by gates, gates'
timeouts, and pollers.

Upstream's model, for reference (manager.c:3271, job.c:906): one thread, one event
loop, a job engine. Starts initiate and return; completion always arrives as an
event (SIGCHLD, sd_notify, exec-fd, inotify, mountinfo, NameOwnerChanged, timers);
the notify event source (priority -5) dispatches strictly before SIGCHLD (-4); the
job run queue dispatches only after all event intake (SD_EVENT_PRIORITY_IDLE+1); a
finished job requeues its After=/Before= neighbours. There are no locks and nothing
to starve.

## Target model

- **One dispatcher thread** owns every mutation of the unit table and unit status.
  It consumes a two-priority event queue and dispatches the job run queue between
  event batches. It never blocks: not on child waits, not on mounts, not on bus
  connects, not on sleeps.
- **Everything else is a producer.** The signal thread, the notification listeners,
  control-socket connections, the D-Bus server, timers, path watchers, the mount
  monitor, udev RPC and socket activation translate their inputs into events and,
  where they need an answer, park on a reply channel.
- **Jobs are the unit of intent and the completion owner.** Every start, stop,
  restart and reload is a Job. The job owns the timeout and is the single place
  that decides success or failure (invariant I2 by construction).
- **Locks shrink to snapshots.** The RuntimeInfo RwLock stays for read-only status
  queries, but the dispatcher is the only writer, and it never holds a guard
  across anything blocking. The try_write spin, the pending-writer gate and the
  10 s yield loops become dead code (I1, I6).

## Today's machinery, inventoried

Each row below is a partial reinvention of a job engine. Each is absorbed by the
new model, not patched.

| Mechanism | Where | Fate |
|-----------|-------|------|
| 32-thread activation pool; workers hold the table read guard across `activate_unit` | activate.rs:975, 1204 | Deleted; jobs run initiate-only on the dispatcher (inc 2, 4) |
| Fixpoint sweep re-walking the graph up to N+2 passes | activate.rs:1004 | Replaced by requeue-on-completion (inc 4) |
| Goal re-drive poller, 400 ticks x 500 ms, then expires | service_manager.rs:564 | Replaced by job requeue; nothing expires (inc 4) |
| Per-unit deferred drivers (notify wait, oneshot exec chain) as spawned threads | activate.rs:1433, 1614 | Become job continuations driven by events (inc 2) |
| A fresh 8-thread pool per deferred completion to dispatch dependents | activate.rs:2044 | Deleted with the drivers (inc 2, 4) |
| A thread spawned per child exit | service_exit_handler.rs:306, signal_handler.rs:151 | Exit handling runs on the dispatcher (inc 1) |
| Upholds= retry threads | activate.rs:1075 | UnitStateChanged retriggers a start job (inc 4) |
| `write_poisoned_nonblocking` unbounded try_write spin | lock_ext.rs:82 | Deleted; the single writer uses plain `write()` (inc 6) |
| WRITER_PENDING gate + 10 s yield loop before activation | activate.rs:897, 1196 | Deleted (inc 6) |
| ACTIVE_GOAL / ACTIVATION_DEPTH statics | activate.rs:852, 870 | Dispatcher serialization; the goal becomes ordinary state (inc 4, 6) |
| Control `start` sleep-polling Restarting units for up to 30 s | control.rs:9571 | Waits on the job's reply channel (inc 2) |
| Oneshot Started->NeverStarted reset dance before re-start | control.rs:9526 | Job semantics: a completed oneshot start job is Done (inc 4) |
| Fictional D-Bus job paths; `n_jobs` hardcoded to 0 | dbus_server.rs:910, 1013 | Real job objects and signals (inc 0) |
| daemon-reload quiescent table swap | control.rs:10756 | Kept; becomes an Objective event on the dispatcher (inc 6) |

## The job engine

Minimal but upstream-shaped (job.c). New module `units/jobs.rs`.

    struct Job {
        id: JobId,                       // u32, monotonically increasing
        unit: UnitId,
        kind: JobKind,                   // Start, Stop, Restart, TryRestart,
                                         // Reload, VerifyActive, Nop
        state: JobState,                 // Waiting, Running
        result: Option<JobResult>,       // Done, Failed, Canceled, Timeout,
                                         // Dependency, Skipped
        deadline: Option<Instant>,       // from the unit's timeouts
        source: ActivationSource,
        waiters: Vec<Sender<JobResult>>, // control/D-Bus reply channels
    }

    struct JobRegistry {
        jobs: HashMap<JobId, Job>,
        by_unit: HashMap<UnitId, JobId>, // at most one installed job per unit
        run_queue: VecDeque<JobId>,      // ready-to-dispatch jobs
    }

Rules, in upstream's terms:

- **One installed job per unit.** A new request against a unit that has a job
  merges (start plus start is start; restart absorbs start) or replaces under
  `JobMode=replace` / fails under `JobMode=fail`. Implement the small closed
  merge table from job_type_merge; transaction anchors and the full merge matrix
  are non-goals.
- **Readiness.** A Waiting job becomes dispatchable when its ordering
  prerequisites are met. Reuse `unstarted_deps` (called at activate.rs:1112) as
  the predicate initially; it already encodes After= against the activation set
  and the device-unit exception.
- **Requeue on completion.** Finishing a job re-evaluates the Waiting jobs of
  units ordered against it (job_finish_and_invalidate). This one rule replaces
  the fixpoint sweep, the goal re-drive and the upholds retry threads: waiters
  park at zero cost and are woken only by events.
- **Timeouts.** Every Running job carries a deadline (start jobs: the unit's
  start timeout; stop jobs: TimeoutStopSec; JobTimeoutSec= when set). The
  dispatcher's timer wheel fires JobTimeout(id); its handling absorbs
  `deferred_start_fail_cleanup` (activate.rs:1388).
- **Client surface.** Real object paths `/org/freedesktop/systemd1/job/<id>`,
  ListJobs, GetJob, JobNew/JobRemoved signals, the unit `Job` property,
  `systemctl list-jobs`. StartUnit returns the job path immediately; `systemctl
  start` without `--no-block` waits for the job result, which retires the
  poll-for-Stopped class (the oneshot `--wait` bug).

## The dispatcher

New module `entrypoints/dispatcher.rs`, spawned via `spawn_critical_thread`
(service_manager.rs:1301).

    enum Event {
        Notify(UnitId, Vec<NotifyMsg>),   // HIGH queue; everything below NORMAL
        ChildExit(Pid, ChildTermination),
        JobQueued,
        JobTimeout(JobId),
        UnitStateChanged(UnitId, UnitStatus, UnitStatus),
        Device(UdevEvent),
        MountsChanged,
        Timer(UnitId),
        PathTriggered(UnitId),
        SocketReady(UnitId),
        Control(ControlRequest, Sender<ControlReply>),
        Objective(Objective),             // Reload, Reexec, Shutdown, Exit
    }

Loop contract, in order, every iteration:

1. Drain the HIGH queue (Notify) completely.
2. Drain the NORMAL queue.
3. If an Objective is pending and nothing is mid-initiation, execute it.
4. Dispatch the run queue: pop ready jobs, run the initiate half of start/stop,
   set deadlines. Initiation may fork and exec (the existing double-fork helper
   is fast) but must never wait.
5. Block on the channel until the next event or the nearest deadline.

The two-queue split preserves upstream's load-bearing ordering: MAINPID= and
READY= from a process that immediately died must be processed before its
ChildExit (upstream: notify at priority -5, SIGCHLD at -4). Run-queue dispatch
strictly after event intake preserves "IPC and event intake outrank job
launches" (run queue at SD_EVENT_PRIORITY_IDLE+1), which is exactly the
starvation `write_poisoned_nonblocking` was invented to dodge.

Panic policy: a panic on the dispatcher is the manager's brain dying and must
not be recovered by lock-poison recovery or die silently (the "silent thread
panic looks like a deadlock" class). Catch at the top of the loop, log to kmsg,
route to `unrecoverable_error` (emergency shell).

## Producer conversions

| Producer | Today | Becomes |
|----------|-------|---------|
| Signal thread | Spawns a thread per child exit | Sends ChildExit; pid_table registration unchanged (I4 already holds) |
| Notification handler | Sets flags that deferred drivers poll | Sends Notify per batch; socket collection unchanged |
| Control connections | Call activate/deactivate synchronously under read guards | Translate verbs into jobs, park on the reply channel with a client-side timeout |
| D-Bus server | Fake job paths, blocking calls into activation | Enqueues jobs, returns real job paths; property reads stay snapshot reads |
| Timer scheduler, path watcher | Call activation directly | Send Timer/PathTriggered |
| Mount monitor | Mutates mount units directly | Sends MountsChanged; reconciliation runs on the dispatcher |
| udev RPC | Control-socket handler mutates device units | Sends Device events; the RPC transport stays (see ARCHITECTURE.md) |
| Socket activation | Own thread starts services | Sends SocketReady; the accept loop stays |
| Watchdog | Own thread | Unchanged (talks to the kernel, not the table) |

## Migration plan

Seven increments. Each ends with `cargo test --workspace` clean, the named VM
gates green, and a commit. The standing gate set for every increment is
01-basic, 03-jobs, 15-dropin and 26-systemctl; increments add more on top.
01-basic boot time is measured before the series and after each increment and
must stay within 10% of the baseline.

Baseline, measured 2026-08-02 on tree 5e28b2a4 before increment 0:
`multi-user.target` activation at 10.09 s on the guest kernel clock
(01-basic VM log), 15.05 s test-driver boot wait.

### Inc 0: job bookkeeping and a real D-Bus job surface

JobRegistry lands in RuntimeInfo. Every control/D-Bus start, stop, restart and
reload creates a job that the existing synchronous path completes inline
(created, Running, finished in one call). ListJobs/GetJob/JobNew/JobRemoved,
the unit Job property and `systemctl list-jobs` become real; the fictional
job paths and the hardcoded `n_jobs = 0` are retired.

Gates: standing set. Expected flips: none yet (63-PATH needs Waiting jobs,
inc 4). Value: the D-Bus surface stops lying, and every later increment has its
bookkeeping in place.

### Inc 1: the dispatcher exists and owns child exits

Dispatcher thread and queues land. The signal thread stops spawning a thread
per exit and sends ChildExit; `service_exit_handler` logic runs on the
dispatcher. The notification handler forwards Notify events for the state
transitions it currently applies itself. Mutation for exits and notifies moves
to the dispatcher; activation is untouched.

Gates: standing set + 05-rlimits, 16-extend-timeout (EXTEND_TIMEOUT_USEC runs
through notify ordering), 59-reloading-restart at its current baseline. Risk:
today's exit handler assumes it may block (per-service state write locks held
while running stop followups); anything blocking found there moves behind an
event, never onto the dispatcher.

### Inc 2: starts initiate-only; deferred drivers become continuations

Job execution calls the initiate half of `Service::start`. The deferred oneshot
exec chain, notify waits, forking-parent waits, dbus-name waits and
exec-confirmation waits become per-job continuation state machines advanced by
ChildExit/Notify events; start timeouts become JobTimeout. `activate_unit` no
longer needs the table read guard held across it, and the 32-thread pool stops
being the thing that waits.

Gates: standing set + the 07-pid1 exec family, 20-mainpidgames,
07-pid1-exec-deserialization (reload during a multi-command oneshot: the
scenario today's drivers were built for). Risk: highest code-motion volume;
land per service type (oneshot, notify, simple/exec, forking, dbus) in separate
commits, each VM-gated.

Progress note, 2026-08-02: increments 0 and 1 are landed, and inc 2 has
landed its deferred half in two slices. The oneshot exec chain runs as
dispatcher continuations advanced by ChildExit events with per-command
deadlines on the dispatcher's wheel; every deferred start wait (notify,
oneshot and forking completion, the exec confirmation window, dbus names
via a per-start watcher thread reporting back as an event) is parked as a
StartWait continuation with EXTEND_TIMEOUT_USEC-aware monotonic deadlines
and SIGTERM-then-SIGKILL escalation, and the per-start polling threads
are deleted. Completion (ExecStartPost plus the Started bookkeeping) runs
on a finisher thread, one per completing start. Two rules proved
load-bearing: deadline refreshes must be monotone, and every dispatcher
acquisition of the RuntimeInfo lock must yield to writers (dispatcher_read),
or a queued writer starves the dispatcher into the old wedge shape.
The inline helper half followed as a third slice: for pool-path
activations, ExecCondition= and ExecStartPre= run as a phased dispatcher
chain (condition, prestart, then the extracted main phase, with
ExecStopPost= as the error phase and the run_cmd exit rules per phase),
so Service::start defers before its first helper fork and the main-phase
result routes to the oneshot chain, a start wait, a finisher, or the
WaitingForSocket transition. Still open in inc 2: starts from non-pool
sources (socket activation, triggers, restarts) run cond/prestart and
the per-type fork_parent waits inline, and stop-side helpers are inc 3's
business. 20-mainpidgames exists here as 07-pid1-main-PID-change, which
is red for harness reasons on the C oracle too (see its wrapper).

### Inc 3: stops and restarts under jobs

ExecStop/ExecStopPost/restart waits become events; deactivation initiates and
returns. The stop path stops holding guards across process waits, invariant
I1's last big holdout.

Gates: standing set + 23-unit-file, 59, 16 stop-timeout extension. Expected
flip: 23-unit-file ExecStopPost-on-failed-start, currently honestly red and
blocked exactly on this (`deferred_start_fail_cleanup` cannot run poststop
under its locks today).

### Inc 4: dependency waiting moves into Waiting jobs

Enqueue jobs for the whole start closure (reuse `collect_unit_start_subgraph`,
activate.rs:807). Jobs sit Waiting until ready and requeue on completion.
Delete: the fixpoint sweep, the goal re-drive poller, the upholds retry
threads, the oneshot status-reset dance, ACTIVE_GOAL and ACTIVATION_DEPTH.
`systemctl isolate` becomes "cancel conflicting jobs, enqueue the new closure".

Gates: full standing set + a cold-boot 01-basic timing run, 03-jobs in depth,
35-login, 45-timedate (D-Bus activation ordering). Expected flip: 63-PATH
issue-24577 (a Waiting job visible in list-jobs). Risk: the highest behavioural
risk in the series; develop behind `SYSTEMD_RS_JOB_GRAPH=1` for A/B comparison,
and delete the flag before the increment merges (a permanent toggle would
double the test surface).

### Inc 5: mounts, swaps and devices as jobs

Mount and swap operations leave the callers' threads: initiation hands the
syscall to a small helper pool that reports completion as an event, and the job
owns an enforced timeout (invariant I2 for mounts). With reconciliation on the
dispatcher, event-source rate limiting for the mountinfo watcher becomes
natural.

Gates: standing set + 10-mount, 22-tmpfiles, then a 60-mount-ratelimit attempt.
Flip candidate: 60-MOUNT-RATELIMIT once the throttle exists.

### Inc 6: single-writer collapse

Every remaining mutation path routes through the dispatcher. Delete
`write_poisoned_nonblocking`, WRITER_PENDING and the yield loops; daemon-reload
and daemon-reexec become Objective events executed between run-queue
dispatches, with the client reply parked (upstream's m->objective shape,
invariant I5). Reexec serializes pending jobs (unit, kind) into the status file
and re-enqueues them after restore.

Gates: standing set + 01-basic daemon-reload/reexec paths, 36-numapolicy.
Afterwards the ARCHITECTURE.md invariant table is rewritten: I1, I2, I3, I5
and I6 hold by construction.

## Load-bearing details ported exactly

- Notify before ChildExit (upstream priorities -5/-4): a doomed process's
  MAINPID=/READY= must land before its exit.
- Run-queue dispatch strictly after event intake (IDLE+1): udev notifications
  and control replies must never sit behind job launches.
- Requeue neighbours on job finish, never on a timer.
- The lock/fork rules stand: fork only from contexts holding no lock another
  thread can observe. Single-writer shrinks this hazard; it does not remove it
  (the inc 5 helper pool must not fork).
- Device units stay event-sourced from udev (RPC transport unchanged); jobs
  never force-start a `.device` (find_startable_units's existing exception).

## Non-goals

The full transaction engine (anchors, complete merge matrix), cgroup controller
masks, exec_helper changes, the .automount subsystem (jobs are its
prerequisite, not its implementation), performance work beyond the 10% boot
budget, and user-manager feature growth (`run_user_manager` shares the engine
automatically).

## Falsification checkpoint

Per the ROADMAP strategy: if after inc 4 the wedge class persists (new
deadlocks or stalls in two consecutive outside-in audits) or 01-basic boot time
regresses more than 25% with no recovery path, stop the series, keep jobs plus
the dispatcher for the control plane only, and take the decision back to the
roadmap.

## Inc 4 reconstruction plan, 2026-08-04

The first inc-4 engine attempt (`drain_job_graph` + `spawn_graph_activation` +
`activate_via_job_graph`) wedged and was reverted in the working copy without a
commit, so it is **not recoverable from git** (0 hits across all refs and
dangling objects). This section reconstructs the remaining work from the current
tree so the next attempt starts concrete, not from memory.

### What already exists (do not rebuild)

- **The scheduler core is complete and dormant**, in `units/jobs.rs`, all
  unit-tested with injected graph/readiness relations (no live table needed):
  run-queue readiness (`enqueue_ready`, `pop_ready`, `enqueue`), the
  event-driven requeue rule (`requeue_after_finish`, upstream
  `job_finish_and_invalidate`), and the timeout wheel (`set_deadline`,
  `next_deadline`, `pop_expired`). `pure_scheduler_drains_a_target_closure_in_
  dependency_order` proves the core threads a target closure to completion. None
  of it is wired into activation yet; `job_graph_enabled()` is checked nowhere in
  live code, so flag-on currently equals flag-off.
- **The producer already exists and is live**, not behind the flag: the
  `--no-block` StartUnit path (`control/control.rs:10428`) collects the start
  subgraph (`collect_unit_start_subgraph`), creates a `Start` job per unit, and
  runs a monitor thread that flips each job Waiting→Running and finishes it as
  its unit reaches a terminal status. So `list-jobs` already reflects the real
  closure. What this path does **not** do is let the jobs *drive* activation:
  actual dispatch is still the fixpoint sweep.

### The seam to replace

Dispatch today is `activate_needed_units_with_source` (`activate.rs:919`): a
32-thread pool running a fixpoint loop of `find_startable_units` (the readiness
frontier — `unstarted_deps` empty, the same predicate the job graph injects) →
`activate_units_recursive` (activate the frontier on the pool, propagating each
unit's before-chain as it completes) → re-sweep until a full pass starts nothing
new. The re-sweep is the safety net for fan-in stragglers. The job graph's
`requeue_after_finish` is precisely the event-driven replacement for that
re-sweep: a completing unit wakes exactly the neighbours ordered after it,
instead of a blind full re-walk.

### The flag-on dispatch (service-only first)

Behind `SYSTEMD_RS_JOB_GRAPH=1`, in the StartUnit monitor thread only:

1. Prime: `enqueue_ready` over the closure using the `find_startable_units`
   readiness (the initially-startable units).
2. Dispatch: `pop_ready` each ready job and activate just that unit (the
   existing per-unit activation the pool already calls), arming its deadline
   with `set_deadline` from the unit's start timeout.
3. Requeue: reuse the monitor's existing terminal-status detection as the
   completion event — when a unit reaches Started/Stopped, `finish` its job and
   `requeue_after_finish` (its `Before=` neighbours, `find_startable_units`
   readiness), then dispatch the newly-ready jobs.
4. Terminate: loop until the registry drains; `pop_expired` against
   `next_deadline` fires JobTimeout for anything stuck, so a wedge
   **self-terminates as a failed job instead of hanging the VM** (this is why the
   timeout wheel was built first).

Start **service-only** (validate on 03-jobs, whose closure is services) because
the known wedge is target-specific: a `.target` job dispatched (Running) whose
unit status never flips to Started, so every unit ordered after it waits forever
and the dispatcher goes silent (`local-fs-pre.target[running/never started]`).
Services flip Started/Stopped through the monitor's existing detection, so the
requeue fires and the closure drains; targets need a process-less completion
path (flip status + finish the job) that the reverted engine got wrong. Land the
service closure first, then the process-less completion, then full boot.

### Falsification for the first slice

Add a `jobGraph` variant of 03-jobs (mind that `default.nix` `testArgs` is a
**closed key set** — a new test param is silently dropped unless added there),
run it against the flag, and require the same green as the flag-off run. Keep
every change strictly inside the `job_graph_enabled()` branch so flag-off boot is
untouched; revert the branch on a wedge (the scheduler core and producer stay
regardless).
