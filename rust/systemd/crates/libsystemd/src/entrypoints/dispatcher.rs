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
}

/// A oneshot exec chain parked between steps, keyed by the awaited pid.
struct PendingChain {
    chain: crate::units::OneshotChainStart,
    cmd: crate::units::Commandline,
    timeout: Option<std::time::Duration>,
    deadline: Option<std::time::Instant>,
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
    super::service_manager::spawn_critical_thread("dispatcher", move || {
        // Oneshot exec chains parked between steps, keyed by awaited pid.
        // Dispatcher-private: nothing else may mutate a parked chain.
        let mut chains: std::collections::HashMap<i32, PendingChain> =
            std::collections::HashMap::new();
        loop {
            let next_deadline = chains.values().filter_map(|chain| chain.deadline).min();
            let event = handle.pop_blocking_until(next_deadline);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                match event {
                    Some(event) => dispatch_one(event, &run_info, &mut chains),
                    // The deadline wheel fired with no event.
                    None => expire_due_chains(&run_info, &mut chains),
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
    chains: &mut std::collections::HashMap<i32, PendingChain>,
) {
    match event {
        Event::Notify {
            unit,
            datagram,
            received_fds,
        } => {
            crate::notification_handler::apply_notify(&unit, &datagram, received_fds, run_info);
        }
        Event::ChildExit(pid, kind, code) => {
            if let Some(pending) = chains.remove(&pid.as_raw()) {
                // A parked chain owns this pid. Drop its PID-table record
                // the way wait_for_helper_child does for inline waits; the
                // registration is guaranteed to have landed because both it
                // and this pop happen on the dispatcher.
                {
                    let ri = run_info.read_poisoned();
                    ri.pid_table.lock_poisoned().remove(&pid);
                }
                let outcome = crate::units::oneshot_chain_child_exited(
                    &pending.chain,
                    &pending.cmd,
                    code,
                    run_info,
                );
                park_chain(outcome, pending.chain, chains);
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
                            id,
                            code,
                            run_info.clone(),
                        );
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
            park_chain(outcome, chain, chains);
        }
    }
}

/// Park a chain that advanced to a new preliminary command, arming its
/// per-command deadline on the wheel. Finished chains need nothing.
fn park_chain(
    outcome: crate::units::OneshotStepOutcome,
    chain: crate::units::OneshotChainStart,
    chains: &mut std::collections::HashMap<i32, PendingChain>,
) {
    if let crate::units::OneshotStepOutcome::Advanced { pid, cmd, timeout } = outcome {
        chains.insert(
            pid.as_raw(),
            PendingChain {
                chain,
                cmd,
                timeout,
                deadline: timeout.map(|t| std::time::Instant::now() + t),
            },
        );
    }
}

fn expire_due_chains(
    run_info: &ArcMutRuntimeInfo,
    chains: &mut std::collections::HashMap<i32, PendingChain>,
) {
    let now = std::time::Instant::now();
    let due: Vec<i32> = chains
        .iter()
        .filter(|(_, pending)| pending.deadline.is_some_and(|deadline| deadline <= now))
        .map(|(raw_pid, _)| *raw_pid)
        .collect();
    for raw_pid in due {
        if let Some(pending) = chains.remove(&raw_pid) {
            crate::units::oneshot_chain_timed_out(
                &pending.chain,
                &pending.cmd,
                nix::unistd::Pid::from_raw(raw_pid),
                pending.timeout,
                run_info,
            );
        }
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
