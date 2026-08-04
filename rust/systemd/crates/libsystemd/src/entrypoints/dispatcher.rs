//! The dispatcher: the thread that will own every mutation of the unit
//! table and unit status (docs/EVENT-LOOP.md).
//!
//! Increment 1 scope: the dispatcher exists, consumes a two-priority event
//! queue, applies sd_notify state transitions, and owns the non-blocking
//! head of child-exit processing. The deactivation-bearing tail of the exit
//! handler still runs on a continuation thread, because today's stop path
//! blocks on ExecStop/ExecStopPost waits; increment 3 restructures stops
//! into initiate-plus-event and retires that thread. Later increments move
//! starts, dependency waiting, mounts and reload onto the dispatcher.

use crate::lock_ext::{MutexExt, RwLockExt};
use crate::runtime_info::{ArcMutRuntimeInfo, RuntimeInfo};
use crate::signal_handler::ChildTermination;
use crate::units::UnitId;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

/// What the signal thread's PID-table phase resolved a reaped child to.
pub enum ChildKind {
    /// A service's main process (ServiceExited recorded).
    Service(UnitId),
    /// A helper process (HelperExited recorded); inline waiters may be
    /// polling the PID table for it, so only continuations the dispatcher
    /// itself registered may claim it.
    Helper(UnitId),
    /// No PID-table entry at reap time. Usually a rerooted orphan, but also
    /// a chain child whose exit raced the registering insert; the chain
    /// table match by pid covers that case (the registration always lands
    /// before the dispatcher pops this event, because both happen on the
    /// dispatcher).
    Unknown,
}

/// Events consumed by the dispatcher. The set grows per increment of
/// docs/EVENT-LOOP.md; only variants with a live producer are defined.
pub enum Event {
    /// One sd_notify datagram for a service. The reader has already
    /// enforced NotifyAccess= and appended the text to the service's
    /// notifications_buffer; the dispatcher drains the buffer (state
    /// transitions such as READY=1 and MAINPID=) and then applies
    /// FDSTORE/FDSTOREREMOVE from the raw datagram. Ownership of any
    /// SCM_RIGHTS fds transfers with the event. HIGH priority.
    Notify {
        unit: UnitId,
        datagram: String,
        received_fds: Vec<std::os::fd::RawFd>,
    },
    /// A reaped child of PID 1 with the PID table already updated
    /// (invariant I4). NORMAL priority, so that a doomed process's final
    /// READY=/MAINPID= datagram is always applied before its exit is
    /// processed (upstream: notify at event priority -5, SIGCHLD at -4).
    ChildExit(nix::unistd::Pid, ChildKind, ChildTermination),
    /// Begin a deferred multi-command oneshot start: the dispatcher forks
    /// the first preliminary command and advances the chain on ChildExit
    /// events (docs/EVENT-LOOP.md inc 2).
    StartOneshotChain(crate::units::OneshotChainStart),
    /// Park a deferred service start: the dispatcher re-evaluates it on
    /// Notify and ChildExit events and enforces its timeouts, replacing the
    /// per-start polling thread (docs/EVENT-LOOP.md inc 2).
    StartServiceWait(crate::units::StartWaitParams),
    /// Begin a deferred ExecCondition=/ExecStartPre= helper phase: the
    /// dispatcher runs each helper initiate-only and, when the phase
    /// completes, runs the main ExecStart dispatch and routes its result
    /// (docs/EVENT-LOOP.md inc 2).
    StartServiceChain(crate::units::ServiceStartChain),
    /// A Type=dbus bus-name watcher thread finished its blocking wait.
    DbusNameResult {
        unit: UnitId,
        ok: bool,
        message: String,
    },
    /// Stop a unit and its stop closure in dependency order on the
    /// dispatcher (docs/EVENT-LOOP.md inc 3): dependents' chains finalize
    /// before the unit's own chain starts, PropagatesStopTo= targets stop
    /// afterwards, and the optional control job finishes when the root's
    /// chain completes.
    StopUnitTree {
        root: UnitId,
        job: Option<crate::units::jobs::JobId>,
    },
}

/// A helper-command continuation parked between steps, keyed by the
/// awaited pid: either a preliminary oneshot ExecStart= chain or an
/// ExecCondition=/ExecStartPre= start chain (which keeps the Child handle
/// so its piped output can be drained on exit).
enum PendingHelper {
    Oneshot {
        chain: crate::units::OneshotChainStart,
        cmd: crate::units::Commandline,
        timeout: Option<std::time::Duration>,
    },
    StartPhase {
        chain: crate::units::ServiceStartChain,
        cmd: crate::units::Commandline,
        child: std::process::Child,
        timeout: Option<std::time::Duration>,
    },
    /// An ExecStop=/ExecStopPost= helper of a stop chain.
    StopHelper {
        chain: crate::units::ServiceStopChain,
        cmd: crate::units::Commandline,
        child: std::process::Child,
    },
    /// A stop chain awaiting its main process's exit under TimeoutStopSec.
    StopMain {
        chain: crate::units::ServiceStopChain,
    },
}

struct PendingEntry {
    kind: PendingHelper,
    deadline: Option<std::time::Instant>,
}

/// A deferred service start parked on the dispatcher, keyed by unit.
struct StartWait {
    id: UnitId,
    svc_type: crate::units::ServiceType,
    next_services_ids: Vec<UnitId>,
    filter_ids: Arc<Vec<crate::units::UnitId>>,
    errors: Arc<Mutex<Vec<crate::units::UnitOperationError>>>,
    source: crate::units::ActivationSource,
    timeout: Option<std::time::Duration>,
    /// Static base start deadline; the armed deadline is the effective one
    /// (base pushed by EXTEND_TIMEOUT_USEC), refreshed on notify events and
    /// re-checked when the wheel fires.
    base_deadline: Option<std::time::Instant>,
    armed_deadline: Option<std::time::Instant>,
    stop_grace: std::time::Duration,
    term_sent_at: Option<std::time::Instant>,
    /// Type=exec confirmation window; expiring COMPLETES the start.
    exec_confirm_at: Option<std::time::Instant>,
    dbus_done: bool,
}

type StartWaits = std::collections::HashMap<UnitId, StartWait>;

/// Dependency-ordered stop scheduling state (docs/EVENT-LOOP.md inc 3):
/// dependents' chains must finalize before their unit's chain starts.
#[derive(Default)]
struct StopGraph {
    /// Unit -> number of its dependents in the closure not yet finalized.
    pending_counts: std::collections::HashMap<UnitId, usize>,
    /// Finalizing child -> parents whose counts it decrements.
    parents: std::collections::HashMap<UnitId, Vec<UnitId>>,
    /// Unit -> PropagatesStopTo= targets, each started as its own tree
    /// after the unit finalizes.
    propagate_after: std::collections::HashMap<UnitId, Vec<UnitId>>,
    /// Root unit -> control job finished when that root's chain completes.
    root_jobs: std::collections::HashMap<UnitId, crate::units::jobs::JobId>,
    /// Units with a live chain or queued in the graph, for dedup.
    active: std::collections::HashSet<UnitId>,
}

/// Global producer handle, for senders that may hold a RuntimeInfo read
/// guard (a second same-thread read acquisition deadlocks against a queued
/// writer, so they must not read the handle out of RuntimeInfo).
static GLOBAL_DISPATCHER: std::sync::OnceLock<DispatcherHandle> = std::sync::OnceLock::new();

#[must_use]
pub fn global() -> Option<&'static DispatcherHandle> {
    GLOBAL_DISPATCHER.get()
}

struct Queues {
    high: VecDeque<Event>,
    normal: VecDeque<Event>,
}

/// Cloneable producer handle to the dispatcher's two-priority queue. The
/// HIGH queue is drained completely before NORMAL is considered.
#[derive(Clone)]
pub struct DispatcherHandle {
    queue: Arc<(Mutex<Queues>, Condvar)>,
}

impl Default for DispatcherHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl DispatcherHandle {
    #[must_use]
    pub fn new() -> Self {
        Self {
            queue: Arc::new((
                Mutex::new(Queues {
                    high: VecDeque::new(),
                    normal: VecDeque::new(),
                }),
                Condvar::new(),
            )),
        }
    }

    /// Handle for unit tests and tools that construct a `RuntimeInfo`
    /// without running a dispatcher: events queue up and are never
    /// consumed.
    #[must_use]
    pub fn detached() -> Self {
        Self::new()
    }

    pub fn send_high(&self, event: Event) {
        let (lock, cv) = &*self.queue;
        lock.lock().unwrap().high.push_back(event);
        cv.notify_one();
    }

    pub fn send_normal(&self, event: Event) {
        let (lock, cv) = &*self.queue;
        lock.lock().unwrap().normal.push_back(event);
        cv.notify_one();
    }

    /// Pop the next event, draining HIGH before NORMAL, or return None once
    /// `deadline` passes with no event: the dispatcher's timer wheel.
    fn pop_blocking_until(&self, deadline: Option<std::time::Instant>) -> Option<Event> {
        let (lock, cv) = &*self.queue;
        let mut queues = lock.lock().unwrap();
        loop {
            if let Some(event) = queues.high.pop_front() {
                return Some(event);
            }
            if let Some(event) = queues.normal.pop_front() {
                return Some(event);
            }
            match deadline {
                None => queues = cv.wait(queues).unwrap(),
                Some(deadline) => {
                    let now = std::time::Instant::now();
                    if now >= deadline {
                        return None;
                    }
                    let (guard, _timeout) = cv.wait_timeout(queues, deadline - now).unwrap();
                    queues = guard;
                }
            }
        }
    }
}

/// Spawn the dispatcher thread. A panic while handling an event is the
/// manager's brain dying: it must neither be swallowed by lock-poison
/// recovery nor die silently (a dead spawned thread looks exactly like a
/// deadlock), so it is caught at the top of the loop, reported to kmsg and
/// routed to the emergency shell.
pub fn spawn_dispatcher(run_info: ArcMutRuntimeInfo) {
    let handle = run_info.read_poisoned().dispatcher.clone();
    let _ = GLOBAL_DISPATCHER.set(handle.clone());
    super::service_manager::spawn_critical_thread("dispatcher", move || {
        // Dispatcher-private continuation state: oneshot exec chains keyed
        // by awaited pid, and deferred start waits keyed by unit. Nothing
        // else may mutate them.
        let mut chains: std::collections::HashMap<i32, PendingEntry> =
            std::collections::HashMap::new();
        let mut start_waits: StartWaits = StartWaits::new();
        let mut stop_graph = StopGraph::default();
        loop {
            // Increment 4: fold the job-graph timeout wheel into the block
            // deadline so a stuck job wakes the drive. `None` (and thus no
            // change to the min) whenever nothing is enqueued, i.e. always with
            // the flag off.
            let job_deadline = if crate::units::jobs::job_graph_enabled() {
                run_info.read_poisoned().jobs.lock().unwrap().next_deadline()
            } else {
                None
            };
            let next_deadline = chains
                .values()
                .filter_map(|entry| entry.deadline)
                .chain(start_waits.values().filter_map(|wait| wait.armed_deadline))
                .chain(start_waits.values().filter_map(|wait| wait.exec_confirm_at))
                .chain(job_deadline)
                .min();
            let event = handle.pop_blocking_until(next_deadline);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                match event {
                    Some(event) => dispatch_one(
                        event,
                        &run_info,
                        &mut chains,
                        &mut start_waits,
                        &mut stop_graph,
                    ),
                    // The deadline wheel fired with no event.
                    None => {
                        expire_due_chains(
                            &run_info,
                            &mut chains,
                            &mut start_waits,
                            &mut stop_graph,
                        );
                        expire_due_start_waits(&run_info, &mut chains, &mut start_waits);
                    }
                }
                // Increment 4: drive the job run queue after event intake (step 4
                // of the loop contract). Behind the flag and a no-op when nothing
                // is enqueued, so the flag-off loop is unchanged. Inside the
                // catch_unwind, so a panic here routes to the emergency shell like
                // any other dispatcher fault.
                if crate::units::jobs::job_graph_enabled() {
                    crate::units::drive_run_queue(&run_info);
                }
            }));
            if let Err(panic) = result {
                let msg = panic_message(&panic);
                crate::entrypoints::kmsg(&format!("DISPATCHER PANIC: {msg}"));
                super::service_manager::unrecoverable_error(format!(
                    "dispatcher panicked while handling an event: {msg}"
                ));
            }
        }
    });
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

fn dispatch_one(
    event: Event,
    run_info: &ArcMutRuntimeInfo,
    chains: &mut std::collections::HashMap<i32, PendingEntry>,
    start_waits: &mut StartWaits,
    stop_graph: &mut StopGraph,
) {
    match event {
        Event::Notify {
            unit,
            datagram,
            received_fds,
        } => {
            crate::notification_handler::apply_notify(&unit, &datagram, received_fds, run_info);
            // A parked start for this unit may just have become ready, and
            // an EXTEND_TIMEOUT_USEC in the datagram moves its deadline.
            // Once SIGTERM escalation has begun, the armed deadline is the
            // stop grace period and must not be clobbered with the (already
            // elapsed) start deadline.
            if let Some(mut wait) = start_waits.remove(&unit) {
                if wait.term_sent_at.is_none() {
                    let refreshed = crate::units::effective_start_deadline(
                        &unit,
                        wait.base_deadline,
                        run_info,
                    );
                    // An extension only ever pushes the deadline out.
                    wait.armed_deadline = match (wait.armed_deadline, refreshed) {
                        (Some(current), Some(new)) => Some(current.max(new)),
                        (current, new) => new.or(current),
                    };
                }
                progress_start_wait(wait, false, run_info, chains, start_waits);
            }
        }
        Event::ChildExit(pid, kind, code) => {
            if let Some(pending) = chains.remove(&pid.as_raw()) {
                // A parked chain owns this pid. Drop its PID-table record
                // the way wait_for_helper_child does for inline waits; the
                // registration is guaranteed to have landed because both it
                // and this pop happen on the dispatcher.
                {
                    let ri = crate::units::dispatcher_read(run_info);
                    ri.pid_table.lock_poisoned().remove(&pid);
                }
                match pending.kind {
                    PendingHelper::Oneshot { chain, cmd, .. } => {
                        let outcome =
                            crate::units::oneshot_chain_child_exited(&chain, &cmd, code, run_info);
                        park_chain(outcome, chain, chains, start_waits, run_info);
                    }
                    PendingHelper::StartPhase {
                        mut chain,
                        cmd,
                        child,
                        ..
                    } => {
                        let outcome = crate::units::service_start_chain_child_exited(
                            &mut chain, &cmd, child, code, run_info,
                        );
                        route_start_chain_outcome(outcome, chain, chains, start_waits, run_info);
                    }
                    PendingHelper::StopHelper {
                        mut chain,
                        cmd,
                        child,
                    } => {
                        let outcome = crate::units::service_stop_chain_helper_exited(
                            &mut chain, &cmd, child, code, run_info,
                        );
                        route_stop_outcome(outcome, chain, chains, stop_graph, run_info);
                    }
                    PendingHelper::StopMain { mut chain } => {
                        // The chain owns finalization; the exit head's
                        // bookkeeping (ExecMainStatus recording) still runs,
                        // and the blocking tail is deliberately not spawned.
                        {
                            let guard = acquire_read(run_info, &chain.id, pid);
                            let _ = crate::services::service_exit_head(
                                pid, &chain.id, &code, &guard, run_info,
                            );
                        }
                        let outcome =
                            crate::units::service_stop_chain_main_exited(&mut chain, run_info);
                        route_stop_outcome(outcome, chain, chains, stop_graph, run_info);
                    }
                }
                return;
            }
            match kind {
                ChildKind::Service(id) => {
                    let proceed = {
                        let guard = acquire_read(run_info, &id, pid);
                        crate::services::service_exit_head(pid, &id, &code, &guard, run_info)
                    };
                    if proceed {
                        crate::services::service_exit_handler_new_thread(
                            pid,
                            id.clone(),
                            code,
                            run_info.clone(),
                        );
                    }
                    // The head recorded main_exit_status: a parked oneshot,
                    // forking or exec start may now have its verdict.
                    if let Some(wait) = start_waits.remove(&id) {
                        progress_start_wait(wait, false, run_info, chains, start_waits);
                    }
                }
                ChildKind::Helper(_) | ChildKind::Unknown => {
                    // Inline waiters poll the PID table for helper exits;
                    // orphan reaps have nothing to do here.
                }
            }
        }
        Event::StartOneshotChain(chain) => {
            let outcome = crate::units::oneshot_chain_step(&chain, run_info);
            park_chain(outcome, chain, chains, start_waits, run_info);
        }
        Event::StartServiceChain(mut chain) => {
            let outcome = crate::units::service_start_chain_step(&mut chain, run_info);
            route_start_chain_outcome(outcome, chain, chains, start_waits, run_info);
        }
        Event::StartServiceWait(params) => {
            register_start_wait(params, run_info, chains, start_waits);
        }
        Event::DbusNameResult { unit, ok, message } => {
            if let Some(mut wait) = start_waits.remove(&unit) {
                if ok {
                    wait.dbus_done = true;
                    progress_start_wait(wait, false, run_info, chains, start_waits);
                } else {
                    fail_start_with_poststop(&unit, message, run_info, chains, start_waits);
                }
            }
        }
        Event::StopUnitTree { root, job } => {
            begin_stop_tree(root, job, run_info, chains, stop_graph);
        }
    }
}

/// Handle Event::StartServiceWait: read the type-relevant setup under one
/// brief guard, spawn the Type=dbus name watcher when needed, then evaluate
/// once immediately (the readiness signal may already be there) and park.
fn register_start_wait(
    params: crate::units::StartWaitParams,
    run_info: &ArcMutRuntimeInfo,
    chains: &mut std::collections::HashMap<i32, PendingEntry>,
    start_waits: &mut StartWaits,
) {
    let Some(setup) = crate::units::start_wait_setup(&params, run_info) else {
        return;
    };
    let now = std::time::Instant::now();
    let base_deadline = setup.timeout.map(|t| now + t);
    if setup.svc_type == crate::units::ServiceType::Dbus {
        match setup.dbus_name.clone() {
            Some(bus_name) => {
                spawn_dbus_name_watcher(params.id.clone(), bus_name, setup.timeout);
            }
            None => {
                fail_start_with_poststop(
                    &params.id,
                    "No BusName= configured for Type=dbus service".to_owned(),
                    run_info,
                    chains,
                    start_waits,
                );
                return;
            }
        }
    }
    let wait = StartWait {
        id: params.id,
        svc_type: setup.svc_type,
        next_services_ids: setup.next_services_ids,
        filter_ids: params.filter_ids,
        errors: params.errors,
        source: params.source,
        timeout: setup.timeout,
        base_deadline,
        armed_deadline: base_deadline,
        stop_grace: setup
            .stop_timeout
            .unwrap_or(std::time::Duration::from_secs(90)),
        term_sent_at: None,
        exec_confirm_at: (setup.svc_type == crate::units::ServiceType::Exec)
            .then(|| now + std::time::Duration::from_millis(500)),
        dbus_done: false,
    };
    progress_start_wait(wait, false, run_info, chains, start_waits);
}

/// Evaluate a start wait and act on the verdict: park it again, complete it
/// on a finisher thread, or fail it. The finisher thread is where the
/// blocking ExecStartPost= runs, never the dispatcher.
fn progress_start_wait(
    wait: StartWait,
    exec_confirm_elapsed: bool,
    run_info: &ArcMutRuntimeInfo,
    chains: &mut std::collections::HashMap<i32, PendingEntry>,
    start_waits: &mut StartWaits,
) {
    match crate::units::evaluate_start_wait(
        &wait.id,
        wait.svc_type,
        exec_confirm_elapsed,
        wait.dbus_done,
        run_info,
    ) {
        crate::units::StartWaitVerdict::Pending => {
            start_waits.insert(wait.id.clone(), wait);
        }
        crate::units::StartWaitVerdict::Abandoned => {}
        crate::units::StartWaitVerdict::Fail(reason) => {
            fail_start_with_poststop(&wait.id, reason, run_info, chains, start_waits);
        }
        crate::units::StartWaitVerdict::Ready => {
            spawn_start_finisher(wait, run_info.clone());
        }
    }
}

fn spawn_start_finisher(wait: StartWait, run_info: ArcMutRuntimeInfo) {
    spawn_start_finisher_parts(
        wait.id,
        wait.next_services_ids,
        wait.filter_ids,
        wait.errors,
        wait.source,
        run_info,
    );
}

fn spawn_start_finisher_parts(
    id: UnitId,
    next: Vec<UnitId>,
    filter: Arc<Vec<UnitId>>,
    errors: Arc<Mutex<Vec<crate::units::UnitOperationError>>>,
    source: crate::units::ActivationSource,
    run_info: ArcMutRuntimeInfo,
) {
    let name = id.name.clone();
    let log_name = name.clone();
    let thread_name = format!("start-finish-{name}");
    let spawned = std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            crate::units::finish_deferred_start(
                &id, &name, next, filter, run_info, errors, source,
            );
        });
    if let Err(e) = spawned {
        log::error!("failed to spawn start finisher for {log_name}: {e}");
    }
}

/// One watcher thread per Type=dbus start: runs the existing blocking bus
/// name wait and reports the outcome as an event. A NameOwnerChanged
/// subscription can replace this without changing the event contract.
fn spawn_dbus_name_watcher(
    unit: UnitId,
    bus_name: String,
    timeout: Option<std::time::Duration>,
) {
    let thread_name = format!("dbus-wait-{}", unit.name);
    let spawned = std::thread::Builder::new().name(thread_name).spawn(move || {
        let event = match crate::dbus_wait::wait_for_name_system_bus(&bus_name, timeout) {
            Ok(crate::dbus_wait::WaitResult::Ok) => Event::DbusNameResult {
                unit,
                ok: true,
                message: String::new(),
            },
            Ok(crate::dbus_wait::WaitResult::Timedout) => Event::DbusNameResult {
                unit,
                ok: false,
                message: format!("Timed out waiting for bus name {bus_name} ({timeout:?})"),
            },
            Err(e) => Event::DbusNameResult {
                unit,
                ok: false,
                message: format!("Error waiting for bus name {bus_name}: {e}"),
            },
        };
        if let Some(handle) = global() {
            handle.send_normal(event);
        }
    });
    if let Err(e) = spawned {
        log::error!("failed to spawn dbus name watcher: {e}");
    }
}

fn expire_due_start_waits(
    run_info: &ArcMutRuntimeInfo,
    chains: &mut std::collections::HashMap<i32, PendingEntry>,
    start_waits: &mut StartWaits,
) {
    let now = std::time::Instant::now();
    let due: Vec<UnitId> = start_waits
        .iter()
        .filter(|(_, wait)| {
            wait.exec_confirm_at.is_some_and(|deadline| deadline <= now)
                || wait.armed_deadline.is_some_and(|deadline| deadline <= now)
        })
        .map(|(id, _)| id.clone())
        .collect();
    for id in due {
        let Some(mut wait) = start_waits.remove(&id) else {
            continue;
        };
        // Type=exec: the confirmation window elapsing COMPLETES the start.
        if wait.exec_confirm_at.is_some_and(|deadline| deadline <= now) {
            wait.exec_confirm_at = None;
            progress_start_wait(wait, true, run_info, chains, start_waits);
            continue;
        }
        // Start timeout: re-check the effective deadline first, since an
        // EXTEND_TIMEOUT_USEC may have pushed it while armed.
        if let Some(effective) =
            crate::units::effective_start_deadline(&id, wait.base_deadline, run_info)
            && effective > now
        {
            wait.armed_deadline = Some(effective);
            start_waits.insert(id, wait);
            continue;
        }
        match wait.term_sent_at {
            None => {
                // kmsg deliberately: PID 1's log macros do not reach the
                // console, and a start timeout firing is exactly what a
                // wedge investigation needs to see.
                crate::entrypoints::kmsg(&format!(
                    "START-TIMEOUT {} after {:?}; sending SIGTERM",
                    id.name, wait.timeout
                ));
                crate::units::start_wait_send_sigterm(&id, run_info);
                wait.term_sent_at = Some(now);
                wait.armed_deadline = Some(now + wait.stop_grace);
                start_waits.insert(id, wait);
            }
            Some(_) => {
                // Verify the process actually survived the SIGTERM before
                // declaring the start dead: under load the exit event may
                // still be queued, and failing blind here SIGKILLs a healthy
                // shutdown mid-trap (the 16/59 margin flake class). The
                // main_pid atomic is maintained lock-free: set at fork,
                // cleared by the exit head.
                let (still_starting, live_pid) = {
                    let ri = crate::units::dispatcher_read(run_info);
                    match ri.unit_table.get(&id) {
                        Some(unit) => (
                            matches!(
                                &*unit.common.status.read_poisoned(),
                                crate::units::UnitStatus::Starting
                            ),
                            unit.common
                                .main_pid
                                .load(std::sync::atomic::Ordering::Acquire)
                                != 0,
                        ),
                        None => (false, false),
                    }
                };
                if !still_starting {
                    // Someone else owned the outcome during the grace period.
                    continue;
                }
                if !live_pid {
                    // The process is gone; its exit or final datagram is in
                    // flight. Re-evaluate now (a notify main-exit resolves to
                    // the protocol failure immediately) and otherwise give the
                    // events a short window instead of failing blind.
                    wait.armed_deadline =
                        Some(now + std::time::Duration::from_millis(500));
                    progress_start_wait(wait, false, run_info, chains, start_waits);
                    continue;
                }
                crate::entrypoints::kmsg(&format!(
                    "START-TIMEOUT {} did not exit after SIGTERM; failing the start",
                    id.name
                ));
                fail_start_with_poststop(
                    &id,
                    format!("Timed out starting ({:?})", wait.timeout),
                    run_info,
                    chains,
                    start_waits,
                );
            }
        }
    }
}

/// Act on a chain step's outcome: park it against the next pid with its
/// per-command deadline, hand a finished fork off to a start wait for
/// completion tracking, or nothing.
fn park_chain(
    outcome: crate::units::OneshotStepOutcome,
    chain: crate::units::OneshotChainStart,
    chains: &mut std::collections::HashMap<i32, PendingEntry>,
    start_waits: &mut StartWaits,
    run_info: &ArcMutRuntimeInfo,
) {
    match outcome {
        crate::units::OneshotStepOutcome::Advanced { pid, cmd, timeout } => {
            chains.insert(
                pid.as_raw(),
                PendingEntry {
                    deadline: timeout.map(|t| std::time::Instant::now() + t),
                    kind: PendingHelper::Oneshot {
                        chain,
                        cmd,
                        timeout,
                    },
                },
            );
        }
        crate::units::OneshotStepOutcome::PoststopChain(mut post) => {
            let outcome = crate::units::service_start_chain_step(&mut post, run_info);
            route_start_chain_outcome(outcome, post, chains, start_waits, run_info);
        }
        crate::units::OneshotStepOutcome::AwaitCompletion => {
            register_start_wait(
                crate::units::StartWaitParams {
                    id: chain.id,
                    next_services_ids: Some(chain.next_services_ids),
                    filter_ids: chain.filter_ids,
                    errors: chain.errors,
                    source: chain.source,
                    check_starting: false,
                },
                run_info,
                chains,
                start_waits,
            );
        }
        crate::units::OneshotStepOutcome::Finished => {}
    }
}

fn expire_due_chains(
    run_info: &ArcMutRuntimeInfo,
    chains: &mut std::collections::HashMap<i32, PendingEntry>,
    start_waits: &mut StartWaits,
    stop_graph: &mut StopGraph,
) {
    let now = std::time::Instant::now();
    let due: Vec<i32> = chains
        .iter()
        .filter(|(_, pending)| pending.deadline.is_some_and(|deadline| deadline <= now))
        .map(|(raw_pid, _)| *raw_pid)
        .collect();
    for raw_pid in due {
        let Some(pending) = chains.remove(&raw_pid) else {
            continue;
        };
        match pending.kind {
            PendingHelper::Oneshot { chain, cmd, timeout } => {
                let outcome = crate::units::oneshot_chain_timed_out(
                    &chain,
                    &cmd,
                    nix::unistd::Pid::from_raw(raw_pid),
                    timeout,
                    run_info,
                );
                park_chain(outcome, chain, chains, start_waits, run_info);
            }
            PendingHelper::StartPhase {
                mut chain,
                cmd,
                timeout,
                ..
            } => {
                let outcome = crate::units::service_start_chain_timed_out(
                    &mut chain,
                    &cmd,
                    nix::unistd::Pid::from_raw(raw_pid),
                    timeout,
                    run_info,
                );
                route_start_chain_outcome(outcome, chain, chains, start_waits, run_info);
            }
            PendingHelper::StopHelper { mut chain, cmd, .. } => {
                // A stop helper overrunning its window: SIGKILL it, record
                // the error, skip the rest of its phase like a bad exit.
                unsafe {
                    libc::kill(raw_pid, libc::SIGKILL);
                }
                chain.errors.push(format!("{cmd} timed out"));
                let outcome =
                    crate::units::service_stop_chain_helper_timeout_skip(&mut chain, run_info);
                route_stop_outcome(outcome, chain, chains, stop_graph, run_info);
            }
            PendingHelper::StopMain { mut chain } => {
                let outcome = crate::units::service_stop_chain_main_timeout(
                    &mut chain,
                    nix::unistd::Pid::from_raw(raw_pid),
                    run_info,
                );
                route_stop_outcome(outcome, chain, chains, stop_graph, run_info);
            }
        }
    }
}

/// Build a stop transaction's closure and start its leaf chains. The
/// closure follows required_by, part_of_by and bound_by edges (those units
/// stop BEFORE the one they depend on), counts each unit's in-closure
/// dependents, and records PropagatesStopTo= targets for post-finalize
/// trees, mirroring deactivate_unit_recursive's order.
fn begin_stop_tree(
    root: UnitId,
    job: Option<crate::units::jobs::JobId>,
    run_info: &ArcMutRuntimeInfo,
    chains: &mut std::collections::HashMap<i32, PendingEntry>,
    stop_graph: &mut StopGraph,
) {
    if stop_graph.active.contains(&root) {
        // Already stopping in another transaction; a duplicate control job
        // completes now rather than dangling.
        if let Some(job_id) = job {
            let ri = crate::units::dispatcher_read(run_info);
            ri.jobs
                .lock_poisoned()
                .finish(job_id, crate::units::jobs::JobResult::Done);
        }
        return;
    }
    if let Some(job_id) = job {
        stop_graph.root_jobs.insert(root.clone(), job_id);
    }
    let mut members: Vec<UnitId> = Vec::new();
    let mut edges: Vec<(UnitId, UnitId)> = Vec::new();
    {
        let ri = crate::units::dispatcher_read(run_info);
        let mut queue = std::collections::VecDeque::from([root.clone()]);
        let mut visited: std::collections::HashSet<UnitId> =
            std::collections::HashSet::new();
        while let Some(id) = queue.pop_front() {
            if !visited.insert(id.clone()) {
                continue;
            }
            let Some(unit) = ri.unit_table.get(&id) else {
                continue;
            };
            let already_stopped = matches!(
                &*unit.common.status.read_poisoned(),
                crate::units::UnitStatus::Stopped(..)
                    | crate::units::UnitStatus::NeverStarted
            );
            if already_stopped {
                continue;
            }
            members.push(id.clone());
            let deps = &unit.common.dependencies;
            for child in deps
                .required_by
                .iter()
                .chain(deps.part_of_by.iter())
                .chain(deps.bound_by.iter())
            {
                edges.push((child.clone(), id.clone()));
                queue.push_back(child.clone());
            }
            let propagate: Vec<UnitId> = deps.propagates_stop_to.clone();
            if !propagate.is_empty() {
                stop_graph.propagate_after.insert(id.clone(), propagate);
            }
        }
    }
    if members.is_empty() {
        finish_stop_root(&root, run_info, stop_graph);
        return;
    }
    for id in &members {
        stop_graph.active.insert(id.clone());
        stop_graph.pending_counts.entry(id.clone()).or_insert(0);
    }
    for (child, parent) in edges {
        if stop_graph.active.contains(&child) && stop_graph.active.contains(&parent) {
            *stop_graph.pending_counts.entry(parent.clone()).or_insert(0) += 1;
            stop_graph.parents.entry(child).or_default().push(parent);
        }
    }
    let leaves: Vec<UnitId> = members
        .iter()
        .filter(|id| stop_graph.pending_counts.get(*id).copied().unwrap_or(0) == 0)
        .cloned()
        .collect();
    for id in leaves {
        start_stop_chain(id, run_info, chains, stop_graph);
    }
}

fn start_stop_chain(
    id: UnitId,
    run_info: &ArcMutRuntimeInfo,
    chains: &mut std::collections::HashMap<i32, PendingEntry>,
    stop_graph: &mut StopGraph,
) {
    let mut chain = crate::units::ServiceStopChain {
        id,
        phase: crate::units::StopPhase::ExecStop(0),
        errors: Vec::new(),
        job: None,
    };
    let outcome = crate::units::service_stop_chain_step(&mut chain, run_info);
    route_stop_outcome(outcome, chain, chains, stop_graph, run_info);
}

fn route_stop_outcome(
    outcome: crate::units::StopStepOutcome,
    chain: crate::units::ServiceStopChain,
    chains: &mut std::collections::HashMap<i32, PendingEntry>,
    stop_graph: &mut StopGraph,
    run_info: &ArcMutRuntimeInfo,
) {
    match outcome {
        crate::units::StopStepOutcome::ForkedHelper {
            pid,
            child,
            cmd,
            timeout,
        } => {
            chains.insert(
                pid.as_raw(),
                PendingEntry {
                    deadline: timeout.map(|t| std::time::Instant::now() + t),
                    kind: PendingHelper::StopHelper { chain, cmd, child },
                },
            );
        }
        crate::units::StopStepOutcome::AwaitingMain { pid, timeout } => {
            chains.insert(
                pid.as_raw(),
                PendingEntry {
                    deadline: timeout.map(|t| std::time::Instant::now() + t),
                    kind: PendingHelper::StopMain { chain },
                },
            );
        }
        crate::units::StopStepOutcome::Finished => {
            on_stop_chain_finished(chain.id, run_info, chains, stop_graph);
        }
    }
}

fn on_stop_chain_finished(
    id: UnitId,
    run_info: &ArcMutRuntimeInfo,
    chains: &mut std::collections::HashMap<i32, PendingEntry>,
    stop_graph: &mut StopGraph,
) {
    stop_graph.active.remove(&id);
    stop_graph.pending_counts.remove(&id);
    finish_stop_root(&id, run_info, stop_graph);
    if let Some(parents) = stop_graph.parents.remove(&id) {
        for parent in parents {
            let ready = match stop_graph.pending_counts.get_mut(&parent) {
                Some(count) => {
                    *count = count.saturating_sub(1);
                    *count == 0
                }
                None => false,
            };
            if ready {
                start_stop_chain(parent, run_info, chains, stop_graph);
            }
        }
    }
    if let Some(targets) = stop_graph.propagate_after.remove(&id) {
        for target in targets {
            begin_stop_tree(target, None, run_info, chains, stop_graph);
        }
    }
}

fn finish_stop_root(id: &UnitId, run_info: &ArcMutRuntimeInfo, stop_graph: &mut StopGraph) {
    if let Some(job_id) = stop_graph.root_jobs.remove(id) {
        let ri = crate::units::dispatcher_read(run_info);
        ri.jobs
            .lock_poisoned()
            .finish(job_id, crate::units::jobs::JobResult::Done);
    }
}

/// Fail a deferred start upstream-style: kill, then ExecStopPost= via a
/// poststop chain when configured, then the failed status (the chain's
/// finalizer). Without ExecStopPost= the failure finalizes immediately.
fn fail_start_with_poststop(
    id: &UnitId,
    reason: String,
    run_info: &ArcMutRuntimeInfo,
    chains: &mut std::collections::HashMap<i32, PendingEntry>,
    start_waits: &mut StartWaits,
) {
    // kmsg deliberately: PID 1's log macros never reach the console, and a
    // failing start's reason is the first thing an investigation needs.
    crate::entrypoints::kmsg(&format!("START-FAIL {}: {reason}", id.name));
    if let Some(mut chain) = crate::units::fail_deferred_start(run_info, id, reason) {
        let outcome = crate::units::service_start_chain_step(&mut chain, run_info);
        route_start_chain_outcome(outcome, chain, chains, start_waits, run_info);
    }
}

/// Route a start chain's step outcome: park it against the next helper pid,
/// hand the completed main phase to the right completion owner, or nothing.
fn route_start_chain_outcome(
    outcome: crate::units::StartChainOutcome,
    chain: crate::units::ServiceStartChain,
    chains: &mut std::collections::HashMap<i32, PendingEntry>,
    start_waits: &mut StartWaits,
    run_info: &ArcMutRuntimeInfo,
) {
    match outcome {
        crate::units::StartChainOutcome::Advanced {
            pid,
            child,
            cmd,
            timeout,
        } => {
            chains.insert(
                pid.as_raw(),
                PendingEntry {
                    deadline: timeout.map(|t| std::time::Instant::now() + t),
                    kind: PendingHelper::StartPhase {
                        chain,
                        cmd,
                        child,
                        timeout,
                    },
                },
            );
        }
        crate::units::StartChainOutcome::MainDispatched(result) => match result {
            crate::services::StartResult::DeferredOneshotExec => {
                let oneshot = crate::units::OneshotChainStart {
                    id: chain.id,
                    next_services_ids: chain.next_services_ids,
                    filter_ids: chain.filter_ids,
                    errors: chain.errors,
                    source: chain.source,
                };
                let outcome = crate::units::oneshot_chain_step(&oneshot, run_info);
                park_chain(outcome, oneshot, chains, start_waits, run_info);
            }
            crate::services::StartResult::DeferredNotifyWait => {
                register_start_wait(
                    crate::units::StartWaitParams {
                        id: chain.id,
                        next_services_ids: Some(chain.next_services_ids),
                        filter_ids: chain.filter_ids,
                        errors: chain.errors,
                        source: chain.source,
                        check_starting: false,
                    },
                    run_info,
                    chains,
                    start_waits,
                );
            }
            crate::services::StartResult::Started => {
                // The main phase completed synchronously (e.g. Type=simple):
                // the finisher runs ExecStartPost, flips Started and
                // dispatches dependents, exactly like a ready start wait.
                spawn_start_finisher_parts(
                    chain.id,
                    chain.next_services_ids,
                    chain.filter_ids,
                    chain.errors,
                    chain.source,
                    run_info.clone(),
                );
            }
            crate::services::StartResult::WaitingForSocket => {
                crate::units::apply_waiting_for_socket(&chain.id, run_info);
            }
            crate::services::StartResult::ConditionSkipped
            | crate::services::StartResult::DeferredPrestart => {
                crate::entrypoints::kmsg(&format!(
                    "START-CHAIN {}: unexpected main-phase result",
                    chain.id.name
                ));
            }
        },
        crate::units::StartChainOutcome::Finished => {}
    }
}

/// Acquire the RuntimeInfo read guard with the same yield-to-writers spin
/// the exit threads use, reporting to kmsg if it takes suspiciously long:
/// a silently stuck dispatcher would stall every notify and exit in the
/// system.
fn acquire_read<'a>(
    run_info: &'a ArcMutRuntimeInfo,
    id: &UnitId,
    pid: nix::unistd::Pid,
) -> std::sync::RwLockReadGuard<'a, RuntimeInfo> {
    let spin_start = std::time::Instant::now();
    let mut warned = false;
    loop {
        match run_info.try_read() {
            Ok(guard) => {
                if warned {
                    crate::entrypoints::kmsg(&format!(
                        "DISPATCHER RECOVERED pid={pid} {} acquired the read lock after {:?}",
                        id.name,
                        spin_start.elapsed()
                    ));
                }
                return guard;
            }
            Err(std::sync::TryLockError::Poisoned(poisoned)) => return poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                if !warned && spin_start.elapsed() >= std::time::Duration::from_secs(10) {
                    warned = true;
                    crate::entrypoints::kmsg(&format!(
                        "DISPATCHER STUCK pid={pid} {} waiting >10s for the RuntimeInfo read \
                         lock; notifies and exits stall until it is released",
                        id.name
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DispatcherHandle, Event};
    use crate::units::{UnitId, UnitIdKind};

    fn uid(name: &str) -> UnitId {
        UnitId {
            kind: UnitIdKind::Service,
            name: name.to_owned(),
        }
    }

    #[test]
    fn high_queue_drains_before_normal() {
        let handle = DispatcherHandle::new();
        handle.send_normal(Event::ChildExit(
            nix::unistd::Pid::from_raw(1),
            super::ChildKind::Service(uid("a.service")),
            crate::signal_handler::ChildTermination::Exit(0),
        ));
        handle.send_high(Event::Notify {
            unit: uid("b.service"),
            datagram: "READY=1\n".to_string(),
            received_fds: Vec::new(),
        });
        // The notify was queued second but must come out first.
        match handle.pop_blocking_until(None).unwrap() {
            Event::Notify { unit, .. } => assert_eq!(unit.name, "b.service"),
            _ => panic!("normal event dispatched before the high queue drained"),
        }
        match handle.pop_blocking_until(None).unwrap() {
            Event::ChildExit(_, super::ChildKind::Service(unit), _) => {
                assert_eq!(unit.name, "a.service");
            }
            _ => panic!("expected the child exit second"),
        }
    }

    #[test]
    fn deadline_pop_returns_none_when_idle() {
        let handle = DispatcherHandle::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(20);
        let popped = handle.pop_blocking_until(Some(deadline));
        assert!(popped.is_none());
        assert!(std::time::Instant::now() >= deadline);
    }
}
