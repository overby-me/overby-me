//! Minimal job objects: the bookkeeping half of upstream's job.c.
//!
//! This is increment 0 of docs/EVENT-LOOP.md. Every start, stop, restart and
//! reload request installs a `Job` in the `JobRegistry`; the existing
//! synchronous control paths complete it inline, and the `--no-block` start
//! path completes its jobs from the activation monitor. Jobs give
//! `systemctl list-jobs` stable monotonically increasing IDs, back the real
//! D-Bus job object paths and the JobNew/JobRemoved signals, and later
//! increments grow them into the actual unit of dispatch (the `run_queue`
//! stays empty until then).
//!
//! Deliberate reductions from upstream, per the design's non-goals: no
//! transaction anchors, and the merge table below is the closed subset of
//! job_type_merge that exists without the compound job types
//! (reload-or-start and friends). A non-mergeable collision replaces the
//! installed job under `JobMode::Replace` (upstream's default) or fails the
//! request under `JobMode::Fail`.

use crate::units::UnitId;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::Sender;

pub type JobId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Start,
    Stop,
    Restart,
    TryRestart,
    Reload,
    VerifyActive,
    Nop,
}

impl JobKind {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::TryRestart => "try-restart",
            Self::Reload => "reload",
            Self::VerifyActive => "verify-active",
            Self::Nop => "nop",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Waiting,
    Running,
}

impl JobState {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Running => "running",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobResult {
    Done,
    Failed,
    Canceled,
    Timeout,
    Dependency,
    Skipped,
}

impl JobResult {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::Timeout => "timeout",
            Self::Dependency => "dependency",
            Self::Skipped => "skipped",
        }
    }
}

/// How a new job request behaves when the unit already has an installed job
/// it cannot merge with. Mirrors the two `JobMode` values the design keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobMode {
    Replace,
    Fail,
}

pub struct Job {
    pub id: JobId,
    pub unit: UnitId,
    pub kind: JobKind,
    pub state: JobState,
    pub result: Option<JobResult>,
    /// Enforced by the owner of the job's completion (the `--no-block`
    /// activation monitor today, the dispatcher's timer wheel from
    /// increment 2 on). `None` means no deadline is armed.
    pub deadline: Option<std::time::Instant>,
    pub source: crate::units::ActivationSource,
    /// Reply channels of clients blocking on this job's completion.
    pub waiters: Vec<Sender<JobResult>>,
}

pub struct JobRegistry {
    next_id: JobId,
    jobs: HashMap<JobId, Job>,
    by_unit: HashMap<UnitId, JobId>,
    /// Ready-to-dispatch jobs. Unused until the dispatcher increments; kept
    /// here so the registry's shape matches the design from the start.
    pub run_queue: VecDeque<JobId>,
}

impl Default for JobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl JobRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_id: 1,
            jobs: HashMap::new(),
            by_unit: HashMap::new(),
            run_queue: VecDeque::new(),
        }
    }

    /// Install a job for `unit`, merging with or replacing an existing
    /// installed job per upstream's rules. New jobs start out `Waiting`;
    /// the synchronous paths flip them to `Running` via [`Self::set_running`]
    /// immediately.
    ///
    /// Returns the installed job's ID, which is the existing job's ID when
    /// the request merged into it.
    pub fn create(
        &mut self,
        unit: UnitId,
        kind: JobKind,
        source: crate::units::ActivationSource,
        mode: JobMode,
    ) -> Result<JobId, String> {
        if let Some(&existing_id) = self.by_unit.get(&unit) {
            let existing_kind = self.jobs[&existing_id].kind;
            if let Some(merged) = merge_kinds(existing_kind, kind) {
                if merged != existing_kind
                    && let Some(job) = self.jobs.get_mut(&existing_id)
                {
                    job.kind = merged;
                }
                return Ok(existing_id);
            }
            if mode == JobMode::Fail {
                return Err(format!(
                    "Transaction for {} is destructive ({} job pending).",
                    unit.name,
                    existing_kind.as_str()
                ));
            }
            self.finish(existing_id, JobResult::Canceled);
        }

        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let job = Job {
            id,
            unit: unit.clone(),
            kind,
            state: JobState::Waiting,
            result: None,
            deadline: None,
            source,
            waiters: Vec::new(),
        };
        crate::dbus_server::notify_job_created(&job);
        self.jobs.insert(id, job);
        self.by_unit.insert(unit, id);
        Ok(id)
    }

    pub fn set_running(&mut self, id: JobId) {
        if let Some(job) = self.jobs.get_mut(&id) {
            job.state = JobState::Running;
        }
    }

    pub fn set_waiting(&mut self, id: JobId) {
        if let Some(job) = self.jobs.get_mut(&id) {
            job.state = JobState::Waiting;
        }
    }

    /// Subscribe a reply channel to the job's completion. The result is sent
    /// on [`Self::finish`].
    pub fn add_waiter(&mut self, id: JobId, waiter: Sender<JobResult>) {
        if let Some(job) = self.jobs.get_mut(&id) {
            job.waiters.push(waiter);
        }
    }

    /// Mark an installed `Waiting` job ready to dispatch by appending it to
    /// the run queue. The dispatcher enqueues a job once its ordering
    /// prerequisites are met (`unstarted_deps` empty); increment 4's drain
    /// step pops it with [`Self::pop_ready`]. Idempotent: a job already in the
    /// queue is not duplicated, and a job that is not installed or no longer
    /// `Waiting` is ignored (finished jobs are already retained out of the
    /// queue by [`Self::finish`]).
    pub fn enqueue(&mut self, id: JobId) {
        if self
            .jobs
            .get(&id)
            .is_some_and(|job| job.state == JobState::Waiting)
            && !self.run_queue.contains(&id)
        {
            self.run_queue.push_back(id);
        }
    }

    /// Pop the next ready job from the run queue and flip it to `Running`,
    /// returning its ID for the dispatcher to run the initiate half of. Skips
    /// stale entries (a job cancelled between enqueue and dispatch, defended
    /// against even though [`Self::finish`] retains them out). Returns `None`
    /// when the queue holds no dispatchable job.
    pub fn pop_ready(&mut self) -> Option<JobId> {
        while let Some(id) = self.run_queue.pop_front() {
            if let Some(job) = self.jobs.get_mut(&id)
                && job.state == JobState::Waiting
            {
                job.state = JobState::Running;
                return Some(id);
            }
        }
        None
    }

    /// Whether the run queue currently holds any job. The dispatcher uses this
    /// to decide whether a drain pass is worth taking.
    #[must_use]
    pub fn run_queue_is_empty(&self) -> bool {
        self.run_queue.is_empty()
    }

    /// Complete and uninstall a job. Idempotent: finishing an ID that is no
    /// longer installed returns `None` and has no effect, so every owner may
    /// call it defensively.
    pub fn finish(&mut self, id: JobId, result: JobResult) -> Option<Job> {
        let mut job = self.jobs.remove(&id)?;
        if self.by_unit.get(&job.unit) == Some(&id) {
            self.by_unit.remove(&job.unit);
        }
        self.run_queue.retain(|queued| *queued != id);
        job.result = Some(result);
        for waiter in &job.waiters {
            let _ = waiter.send(result);
        }
        crate::dbus_server::notify_job_removed(&job);
        Some(job)
    }

    /// Wake the Waiting jobs whose readiness may have changed because
    /// `finished` just completed. This is upstream's `job_finish_and_invalidate`
    /// (job.c) reduced to its one load-bearing rule: finishing a job
    /// re-evaluates the Waiting jobs of the units ordered against it, and
    /// enqueues the ones that are now ready. That single rule is what lets a
    /// Waiting job park at zero cost and be woken only by events — it replaces
    /// the fixpoint sweep, the goal re-drive and the upholds retry threads
    /// (docs/EVENT-LOOP.md, "Inc 4").
    ///
    /// `ordered_after(u)` yields the units ordered *after* `u` — its `Before=`
    /// neighbours, i.e. the units that were waiting on `u`. `is_ready(v)`
    /// re-evaluates whether `v`'s ordering prerequisites are now all met (the
    /// live caller wraps [`unstarted_deps`](crate::units::unitset_manipulation::activate::unstarted_deps),
    /// checking it is empty). Both relations are injected so the rule is pure
    /// over the graph and exercisable without a live unit table: PID 1 passes
    /// the real `Before=` walk and predicate; a test passes fixtures.
    ///
    /// Only a neighbour that still holds an installed `Waiting` job and is now
    /// ready is enqueued (a Running or already-queued job is left alone by
    /// [`Self::enqueue`]'s guards). Returns the job IDs newly moved onto the run
    /// queue, for the dispatcher to log and for tests to assert on.
    pub fn requeue_after_finish(
        &mut self,
        finished: &UnitId,
        ordered_after: impl Fn(&UnitId) -> Vec<UnitId>,
        is_ready: impl Fn(&UnitId) -> bool,
    ) -> Vec<JobId> {
        let mut woken = Vec::new();
        for neighbour in ordered_after(finished) {
            let Some(&id) = self.by_unit.get(&neighbour) else {
                // No installed job for this neighbour: nothing to wake.
                continue;
            };
            let waiting = self
                .jobs
                .get(&id)
                .is_some_and(|job| job.state == JobState::Waiting);
            if waiting && is_ready(&neighbour) {
                let already_queued = self.run_queue.contains(&id);
                self.enqueue(id);
                if !already_queued && !woken.contains(&id) {
                    woken.push(id);
                }
            }
        }
        woken
    }

    /// Prime the run queue when a start closure is first installed: enqueue
    /// every installed `Waiting` job that already has all its ordering
    /// prerequisites met, returning the IDs queued in job-creation order. This
    /// is the one-time pump-priming pass upstream's transaction does when it
    /// activates a closure — the jobs with no unstarted deps go straight onto
    /// the queue. Steady-state readiness afterwards is event-driven via
    /// [`Self::requeue_after_finish`], never a repeated fixpoint sweep. Like the
    /// other run-queue methods, `is_ready(u)` is injected (live: `u`'s
    /// `unstarted_deps` is empty) so the rule stays pure over the graph.
    pub fn enqueue_ready(&mut self, is_ready: impl Fn(&UnitId) -> bool) -> Vec<JobId> {
        // Snapshot the Waiting candidates first (in id order for a deterministic
        // result): `enqueue` needs `&mut self`, so we cannot hold an iterator
        // over `self.jobs` across the calls.
        let mut candidates: Vec<(JobId, UnitId)> = self
            .jobs
            .iter()
            .filter(|(_, job)| job.state == JobState::Waiting)
            .map(|(id, job)| (*id, job.unit.clone()))
            .collect();
        candidates.sort_by_key(|&(id, _)| id);

        let mut queued = Vec::new();
        for (id, unit) in candidates {
            if !self.run_queue.contains(&id) && is_ready(&unit) {
                self.enqueue(id);
                queued.push(id);
            }
        }
        queued
    }

    /// Set (or refresh) a job's timeout deadline. The dispatcher assigns one
    /// when it begins running a job (the unit's start or stop timeout, or
    /// `JobTimeoutSec=` when set); [`Self::next_deadline`] then bounds the
    /// dispatcher's block and [`Self::pop_expired`] fires the timeouts
    /// (docs/EVENT-LOOP.md, "Timeouts"). A refresh must be monotone at the call
    /// site (EXTEND_TIMEOUT_USEC only ever pushes the deadline later), matching
    /// the rule the deferred start waits already hold. A no-op for an unknown
    /// id.
    pub fn set_deadline(&mut self, id: JobId, deadline: std::time::Instant) {
        if let Some(job) = self.jobs.get_mut(&id) {
            job.deadline = Some(deadline);
        }
    }

    /// The earliest deadline among installed jobs, or `None` when no job carries
    /// one. The dispatcher blocks on its event channel until the next event or
    /// this instant, whichever is sooner (dispatcher loop step 5).
    #[must_use]
    pub fn next_deadline(&self) -> Option<std::time::Instant> {
        self.jobs.values().filter_map(|job| job.deadline).min()
    }

    /// Report the jobs whose deadline is at or before `now`, clearing it so each
    /// fires exactly once even if the caller defers the finish. The dispatcher
    /// raises JobTimeout for every returned id, routing it to the timeout
    /// cleanup (upstream `job_timeout`, absorbing `deferred_start_fail_cleanup`)
    /// and finishing the job as `Timeout`. Returned in job-creation order so a
    /// batch of simultaneous timeouts fires deterministically.
    pub fn pop_expired(&mut self, now: std::time::Instant) -> Vec<JobId> {
        let mut expired: Vec<JobId> = self
            .jobs
            .iter()
            .filter(|(_, job)| job.deadline.is_some_and(|d| d <= now))
            .map(|(id, _)| *id)
            .collect();
        expired.sort_unstable();
        for id in &expired {
            if let Some(job) = self.jobs.get_mut(id) {
                job.deadline = None;
            }
        }
        expired
    }

    #[must_use]
    pub fn get(&self, id: JobId) -> Option<&Job> {
        self.jobs.get(&id)
    }

    #[must_use]
    pub fn job_for_unit(&self, unit: &UnitId) -> Option<&Job> {
        self.by_unit.get(unit).and_then(|id| self.jobs.get(id))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Job> {
        self.jobs.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }
}

/// The closed subset of upstream's job_type_merge table that exists without
/// the compound job types. Returns the merged kind when `new` collapses into
/// the installed job, `None` when the two conflict.
fn merge_kinds(existing: JobKind, new: JobKind) -> Option<JobKind> {
    use JobKind::{Nop, Reload, Restart, Start, Stop, TryRestart, VerifyActive};
    // A request never conflicts with itself.
    if existing == new {
        return Some(existing);
    }
    match (existing, new) {
        // Nop merges into anything, in both directions.
        (e, Nop) => Some(e),
        (Nop, n) => Some(n),
        // Restart absorbs starts, verifications and try-restarts; a restart
        // request upgrades any of those in place.
        (Restart, Start | VerifyActive | TryRestart)
        | (Start | VerifyActive | TryRestart, Restart) => Some(Restart),
        // Start absorbs verify-active, and upgrades an installed one.
        (Start, VerifyActive) | (VerifyActive, Start) => Some(Start),
        // Try-restart and verify-active combine to try-restart.
        (TryRestart, VerifyActive) | (VerifyActive, TryRestart) => Some(TryRestart),
        // Start and try-restart only combine through the compound
        // try-restart-or-start semantics, which are out of scope.
        (Start, TryRestart) | (TryRestart, Start) => None,
        // Anything against Stop, and Reload against the start family, needs
        // the compound types upstream merges into, which are also out of
        // scope. The caller replaces or fails per its JobMode.
        (_, Stop) | (Stop, _) | (Reload, _) | (_, Reload) => None,
        // Identical pairs were handled by the early return above.
        (Start, Start)
        | (Restart, Restart)
        | (TryRestart, TryRestart)
        | (VerifyActive, VerifyActive) => Some(existing),
    }
}

/// RAII wrapper for the synchronous control paths: installs a `Running` job
/// on creation and guarantees it never leaks. An early `return Err(...)?`
/// anywhere in a control handler drops the handle, which finishes the job as
/// `Failed`; the happy path calls [`Self::finish`] explicitly.
pub struct JobHandle {
    jobs: crate::runtime_info::Jobs,
    id: JobId,
    finished: bool,
}

impl JobHandle {
    pub fn create(
        jobs: &crate::runtime_info::Jobs,
        unit: UnitId,
        kind: JobKind,
        mode: JobMode,
    ) -> Result<Self, String> {
        let mut registry = jobs.lock().unwrap();
        let id = registry.create(unit, kind, crate::units::ActivationSource::Regular, mode)?;
        registry.set_running(id);
        drop(registry);
        Ok(Self {
            jobs: jobs.clone(),
            id,
            finished: false,
        })
    }

    #[must_use]
    pub const fn id(&self) -> JobId {
        self.id
    }

    pub fn finish(mut self, result: JobResult) {
        self.finished = true;
        self.jobs.lock().unwrap().finish(self.id, result);
    }
}

impl Drop for JobHandle {
    fn drop(&mut self) {
        if !self.finished {
            self.jobs.lock().unwrap().finish(self.id, JobResult::Failed);
        }
    }
}

/// Whether the increment-4 job-graph dispatch path is enabled. While the
/// increment is developed it is opt-in via `SYSTEMD_RS_JOB_GRAPH=1` so the
/// default boot stays on the fixpoint-sweep activation path for A/B
/// comparison; the flag (and the old path) are deleted when the increment
/// merges (docs/EVENT-LOOP.md, "Inc 4"). Matches the `SYSTEMD_RS_REEXEC`
/// idiom read in service_manager.rs / signal_handler.rs.
#[must_use]
pub fn job_graph_enabled() -> bool {
    std::env::var("SYSTEMD_RS_JOB_GRAPH").map_or(true, |v| v != "0")
}

#[cfg(test)]
mod tests {
    use super::{JobKind, JobMode, JobRegistry, JobResult, JobState};
    use crate::units::{ActivationSource, UnitId, UnitIdKind};

    fn uid(name: &str) -> UnitId {
        UnitId {
            kind: UnitIdKind::Service,
            name: name.to_owned(),
        }
    }

    fn tid(name: &str) -> UnitId {
        UnitId {
            kind: UnitIdKind::Target,
            name: name.to_owned(),
        }
    }

    fn create(reg: &mut JobRegistry, name: &str, kind: JobKind, mode: JobMode) -> Result<u32, String> {
        reg.create(uid(name), kind, ActivationSource::Regular, mode)
    }

    #[test]
    fn ids_are_stable_and_monotonic() {
        let mut reg = JobRegistry::new();
        let a = create(&mut reg, "a.service", JobKind::Start, JobMode::Replace).unwrap();
        let b = create(&mut reg, "b.service", JobKind::Start, JobMode::Replace).unwrap();
        assert_eq!(a, 1);
        assert_eq!(b, 2);
        assert_eq!(reg.len(), 2);
        // Finishing does not recycle IDs.
        reg.finish(a, JobResult::Done);
        let c = create(&mut reg, "c.service", JobKind::Start, JobMode::Replace).unwrap();
        assert_eq!(c, 3);
    }

    #[test]
    fn same_kind_merges_into_existing() {
        let mut reg = JobRegistry::new();
        let a = create(&mut reg, "a.service", JobKind::Start, JobMode::Replace).unwrap();
        let b = create(&mut reg, "a.service", JobKind::Start, JobMode::Replace).unwrap();
        assert_eq!(a, b);
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn restart_absorbs_installed_start() {
        let mut reg = JobRegistry::new();
        let a = create(&mut reg, "a.service", JobKind::Start, JobMode::Replace).unwrap();
        let b = create(&mut reg, "a.service", JobKind::Restart, JobMode::Replace).unwrap();
        assert_eq!(a, b);
        assert_eq!(reg.get(a).unwrap().kind, JobKind::Restart);
    }

    #[test]
    fn conflicting_kind_fails_under_mode_fail() {
        let mut reg = JobRegistry::new();
        create(&mut reg, "a.service", JobKind::Start, JobMode::Replace).unwrap();
        let err = create(&mut reg, "a.service", JobKind::Stop, JobMode::Fail).unwrap_err();
        assert!(err.contains("destructive"), "unexpected error: {err}");
        assert_eq!(reg.job_for_unit(&uid("a.service")).unwrap().kind, JobKind::Start);
    }

    #[test]
    fn conflicting_kind_replaces_and_cancels() {
        let mut reg = JobRegistry::new();
        let a = create(&mut reg, "a.service", JobKind::Start, JobMode::Replace).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        reg.add_waiter(a, tx);
        let b = create(&mut reg, "a.service", JobKind::Stop, JobMode::Replace).unwrap();
        assert_ne!(a, b);
        assert_eq!(rx.try_recv().unwrap(), JobResult::Canceled);
        assert!(reg.get(a).is_none());
        assert_eq!(reg.job_for_unit(&uid("a.service")).unwrap().id, b);
    }

    #[test]
    fn finish_notifies_waiters_and_is_idempotent() {
        let mut reg = JobRegistry::new();
        let a = create(&mut reg, "a.service", JobKind::Start, JobMode::Replace).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        reg.add_waiter(a, tx);
        let finished = reg.finish(a, JobResult::Done).unwrap();
        assert_eq!(finished.result, Some(JobResult::Done));
        assert_eq!(rx.try_recv().unwrap(), JobResult::Done);
        assert!(reg.finish(a, JobResult::Done).is_none());
        assert!(reg.is_empty());
    }

    #[test]
    fn handle_drop_finishes_as_failed() {
        let jobs: crate::runtime_info::Jobs =
            std::sync::Arc::new(std::sync::Mutex::new(JobRegistry::new()));
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let handle =
                super::JobHandle::create(&jobs, uid("a.service"), JobKind::Start, JobMode::Replace)
                    .unwrap();
            assert_eq!(
                jobs.lock().unwrap().get(handle.id()).unwrap().state,
                JobState::Running
            );
            jobs.lock().unwrap().add_waiter(handle.id(), tx);
            // Dropped without an explicit finish, as on an early error return.
        }
        assert_eq!(rx.try_recv().unwrap(), JobResult::Failed);
        assert!(jobs.lock().unwrap().is_empty());
    }

    #[test]
    fn source_is_recorded_for_activation_details() {
        let mut reg = JobRegistry::new();
        let a = reg
            .create(
                uid("a.service"),
                JobKind::Start,
                ActivationSource::NonBlocking,
                JobMode::Replace,
            )
            .unwrap();
        assert_eq!(reg.get(a).unwrap().source, ActivationSource::NonBlocking);
    }

    // --- increment 4: run-queue dispatch primitives ---

    #[test]
    fn enqueue_then_pop_ready_marks_running_in_fifo_order() {
        let mut reg = JobRegistry::new();
        let a = create(&mut reg, "a.service", JobKind::Start, JobMode::Replace).unwrap();
        let b = create(&mut reg, "b.service", JobKind::Start, JobMode::Replace).unwrap();
        assert!(reg.run_queue_is_empty());
        reg.enqueue(a);
        reg.enqueue(b);
        assert!(!reg.run_queue_is_empty());
        // FIFO: a before b, each flipped Waiting -> Running on pop.
        assert_eq!(reg.pop_ready(), Some(a));
        assert_eq!(reg.get(a).unwrap().state, JobState::Running);
        assert_eq!(reg.get(b).unwrap().state, JobState::Waiting);
        assert_eq!(reg.pop_ready(), Some(b));
        assert_eq!(reg.pop_ready(), None);
        assert!(reg.run_queue_is_empty());
    }

    #[test]
    fn enqueue_is_idempotent_and_ignores_non_waiting() {
        let mut reg = JobRegistry::new();
        let a = create(&mut reg, "a.service", JobKind::Start, JobMode::Replace).unwrap();
        // Queuing the same Waiting job twice does not duplicate it.
        reg.enqueue(a);
        reg.enqueue(a);
        assert_eq!(reg.pop_ready(), Some(a));
        assert_eq!(reg.pop_ready(), None);
        // Now Running: re-enqueue is a no-op (only Waiting jobs are queued).
        reg.enqueue(a);
        assert!(reg.run_queue_is_empty());
        // An unknown id is ignored too.
        reg.enqueue(9999);
        assert!(reg.run_queue_is_empty());
    }

    #[test]
    fn pop_ready_skips_finished_stale_entries() {
        let mut reg = JobRegistry::new();
        let a = create(&mut reg, "a.service", JobKind::Start, JobMode::Replace).unwrap();
        let b = create(&mut reg, "b.service", JobKind::Start, JobMode::Replace).unwrap();
        reg.enqueue(a);
        reg.enqueue(b);
        // finish() retains a out of the run queue, so it is gone cleanly.
        reg.finish(a, JobResult::Canceled);
        // Force a stale entry (an id no longer installed) to prove pop_ready
        // defends against one regardless, and still returns the live job.
        reg.run_queue.push_front(a);
        assert_eq!(reg.pop_ready(), Some(b));
        assert_eq!(reg.pop_ready(), None);
    }

    // --- increment 4: requeue-on-finish (job_finish_and_invalidate) ---

    #[test]
    fn requeue_only_wakes_ready_installed_waiting_neighbours() {
        let mut reg = JobRegistry::new();
        // a: Waiting + ready -> woken. b: ready but already Running -> skipped
        // by enqueue's guard. c: a neighbour with no installed job -> skipped.
        let a = create(&mut reg, "a.service", JobKind::Start, JobMode::Replace).unwrap();
        let b = create(&mut reg, "b.service", JobKind::Start, JobMode::Replace).unwrap();
        reg.set_running(b);
        let neighbours = |_: &UnitId| vec![uid("a.service"), uid("b.service"), uid("c.service")];
        // Report a and b as ready; only a is Waiting, and c has no job at all.
        let ready = |v: &UnitId| matches!(v.name.as_str(), "a.service" | "b.service");
        let woken = reg.requeue_after_finish(&uid("x.service"), neighbours, ready);
        assert_eq!(woken, vec![a]);
        assert_eq!(reg.pop_ready(), Some(a));
        assert_eq!(reg.pop_ready(), None); // b (Running) and c (no job) never queued
    }

    #[test]
    fn requeue_is_idempotent() {
        let mut reg = JobRegistry::new();
        let a = create(&mut reg, "a.service", JobKind::Start, JobMode::Replace).unwrap();
        let neighbours = |_: &UnitId| vec![uid("a.service")];
        let first = reg.requeue_after_finish(&uid("x.service"), &neighbours, |_| true);
        assert_eq!(first, vec![a]);
        // Already queued: a second finish of the same dep must not re-report it.
        let second = reg.requeue_after_finish(&uid("x.service"), &neighbours, |_| true);
        assert!(second.is_empty(), "an already-queued job is not woken again");
        assert_eq!(reg.pop_ready(), Some(a));
        assert_eq!(reg.pop_ready(), None);
    }

    /// The `local-fs-pre.target` wedge in one pure test: a target ordered after
    /// its members, with a service ordered after the target. Finishing the
    /// members must thread the target through (it holds no process, so nothing
    /// else can), and finishing the target must in turn wake the service. The
    /// reverted inc-4 engine wedged here because a finished member never
    /// re-evaluated the target, so the dispatcher went silent with the target
    /// Waiting forever. This asserts the requeue rule threads the whole chain.
    #[test]
    fn requeue_threads_a_target_through_its_members() {
        use std::collections::{HashMap, HashSet};

        let mut reg = JobRegistry::new();
        let m1 = create(&mut reg, "m1.service", JobKind::Start, JobMode::Replace).unwrap();
        let m2 = create(&mut reg, "m2.service", JobKind::Start, JobMode::Replace).unwrap();
        let t = reg
            .create(
                tid("local-fs-pre.target"),
                JobKind::Start,
                ActivationSource::Regular,
                JobMode::Replace,
            )
            .unwrap();
        let s = create(&mut reg, "s.service", JobKind::Start, JobMode::Replace).unwrap();

        // Before= adjacency (who is ordered *after* each unit, keyed by name).
        let mut before: HashMap<&str, Vec<UnitId>> = HashMap::new();
        before.insert("m1.service", vec![tid("local-fs-pre.target")]);
        before.insert("m2.service", vec![tid("local-fs-pre.target")]);
        before.insert("local-fs-pre.target", vec![uid("s.service")]);
        let ordered_after = |u: &UnitId| before.get(u.name.as_str()).cloned().unwrap_or_default();

        // After= adjacency: a unit is ready once all its After= deps finished.
        let mut after: HashMap<&str, Vec<&str>> = HashMap::new();
        after.insert("local-fs-pre.target", vec!["m1.service", "m2.service"]);
        after.insert("s.service", vec!["local-fs-pre.target"]);
        let ready_with = |finished: &HashSet<String>, v: &UnitId| {
            after
                .get(v.name.as_str())
                .is_none_or(|deps| deps.iter().all(|d| finished.contains(*d)))
        };

        let mut finished: HashSet<String> = HashSet::new();

        // m1 finishes first: the target still waits on m2, so nothing wakes.
        reg.finish(m1, JobResult::Done);
        finished.insert("m1.service".to_owned());
        let woken = reg.requeue_after_finish(&uid("m1.service"), &ordered_after, |v| {
            ready_with(&finished, v)
        });
        assert!(woken.is_empty(), "target must not wake on a partial member set");

        // m2 finishes: the target is now ready and must be enqueued.
        reg.finish(m2, JobResult::Done);
        finished.insert("m2.service".to_owned());
        let woken = reg.requeue_after_finish(&uid("m2.service"), &ordered_after, |v| {
            ready_with(&finished, v)
        });
        assert_eq!(woken, vec![t], "both members done -> target ready");
        assert_eq!(reg.pop_ready(), Some(t));

        // The target finishes (no process of its own): the service must wake.
        reg.finish(t, JobResult::Done);
        finished.insert("local-fs-pre.target".to_owned());
        let woken = reg.requeue_after_finish(&tid("local-fs-pre.target"), &ordered_after, |v| {
            ready_with(&finished, v)
        });
        assert_eq!(woken, vec![s], "target done -> service after it ready");
        assert_eq!(reg.pop_ready(), Some(s));
        assert_eq!(reg.pop_ready(), None);
    }

    #[test]
    fn enqueue_ready_primes_only_the_initially_ready_jobs() {
        let mut reg = JobRegistry::new();
        // m1 has no deps (ready); the target waits on m1 (not ready yet).
        let m1 = create(&mut reg, "m1.service", JobKind::Start, JobMode::Replace).unwrap();
        let _t = reg
            .create(
                tid("local-fs-pre.target"),
                JobKind::Start,
                ActivationSource::Regular,
                JobMode::Replace,
            )
            .unwrap();
        let queued = reg.enqueue_ready(|v| v.name == "m1.service");
        assert_eq!(queued, vec![m1], "only the initially-ready job is primed");
        assert_eq!(reg.pop_ready(), Some(m1));
        assert_eq!(reg.pop_ready(), None); // the target was not primed
    }

    /// The pure scheduler core, composed and driven to completion with no
    /// threads and no live table: prime with `enqueue_ready`, then loop
    /// `pop_ready` -> complete -> `requeue_after_finish` until the queue drains.
    /// A diamond closure (two members -> a process-less target -> a service)
    /// must fully drain in dependency order. This is the isolation proof that
    /// the scheduler logic threads a target correctly; the reverted engine's
    /// wedge is therefore in the thread-activation half (a target job that never
    /// completes), not here. Doubles as the executable spec the live wiring
    /// must satisfy.
    #[test]
    fn pure_scheduler_drains_a_target_closure_in_dependency_order() {
        use std::collections::{HashMap, HashSet};

        let mut reg = JobRegistry::new();
        create(&mut reg, "m1.service", JobKind::Start, JobMode::Replace).unwrap();
        create(&mut reg, "m2.service", JobKind::Start, JobMode::Replace).unwrap();
        reg.create(
            tid("local-fs-pre.target"),
            JobKind::Start,
            ActivationSource::Regular,
            JobMode::Replace,
        )
        .unwrap();
        create(&mut reg, "s.service", JobKind::Start, JobMode::Replace).unwrap();

        let mut before: HashMap<&str, Vec<UnitId>> = HashMap::new();
        before.insert("m1.service", vec![tid("local-fs-pre.target")]);
        before.insert("m2.service", vec![tid("local-fs-pre.target")]);
        before.insert("local-fs-pre.target", vec![uid("s.service")]);
        let ordered_after = |u: &UnitId| before.get(u.name.as_str()).cloned().unwrap_or_default();

        let mut after: HashMap<&str, Vec<&str>> = HashMap::new();
        after.insert("local-fs-pre.target", vec!["m1.service", "m2.service"]);
        after.insert("s.service", vec!["local-fs-pre.target"]);
        let ready_with = |finished: &HashSet<String>, v: &UnitId| {
            after
                .get(v.name.as_str())
                .is_none_or(|deps| deps.iter().all(|d| finished.contains(*d)))
        };

        let mut finished: HashSet<String> = HashSet::new();
        let mut order: Vec<String> = Vec::new();

        reg.enqueue_ready(|v| ready_with(&finished, v));
        // Every unit in this closure is process-less for the test: a popped job
        // completes immediately and wakes its neighbours. A real dispatcher runs
        // the initiate half here and finishes on the completion event.
        while let Some(id) = reg.pop_ready() {
            let unit = reg.get(id).unwrap().unit.clone();
            reg.finish(id, JobResult::Done);
            finished.insert(unit.name.clone());
            order.push(unit.name.clone());
            reg.requeue_after_finish(&unit, &ordered_after, |v| ready_with(&finished, v));
        }

        // The whole closure drained, in an order that respects every After= edge.
        assert_eq!(order.len(), 4, "all four jobs ran: {order:?}");
        let pos = |n: &str| order.iter().position(|x| x == n).expect("unit ran");
        assert!(pos("m1.service") < pos("local-fs-pre.target"));
        assert!(pos("m2.service") < pos("local-fs-pre.target"));
        assert!(pos("local-fs-pre.target") < pos("s.service"));
        assert!(reg.is_empty(), "no orphaned job left installed");
    }

    // --- increment 4: job-timeout deadlines (the dispatcher timer wheel) ---

    #[test]
    fn next_deadline_is_none_until_set_and_ignores_unknown_ids() {
        let mut reg = JobRegistry::new();
        assert_eq!(reg.next_deadline(), None);
        // Setting a deadline on an id that is not installed is a no-op.
        reg.set_deadline(9999, std::time::Instant::now());
        assert_eq!(reg.next_deadline(), None);
        // An installed job with no deadline yet still yields None.
        let _a = create(&mut reg, "a.service", JobKind::Start, JobMode::Replace).unwrap();
        assert_eq!(reg.next_deadline(), None);
    }

    #[test]
    fn deadlines_report_earliest_and_expire_once_in_order() {
        use std::time::{Duration, Instant};
        let mut reg = JobRegistry::new();
        let a = create(&mut reg, "a.service", JobKind::Start, JobMode::Replace).unwrap();
        let b = create(&mut reg, "b.service", JobKind::Start, JobMode::Replace).unwrap();
        let c = create(&mut reg, "c.service", JobKind::Start, JobMode::Replace).unwrap();
        let base = Instant::now();
        // a and b are near-term (b sooner than a); c is far out.
        reg.set_deadline(a, base + Duration::from_millis(50));
        reg.set_deadline(b, base + Duration::from_millis(10));
        reg.set_deadline(c, base + Duration::from_secs(3600));
        // next_deadline is the earliest across all installed jobs (b's).
        assert_eq!(reg.next_deadline(), Some(base + Duration::from_millis(10)));
        // By base+100ms both a and b have expired, returned in job-id order; c
        // has not.
        let expired = reg.pop_expired(base + Duration::from_millis(100));
        assert_eq!(expired, vec![a, b]);
        // Cleared on report: a second pop at the same instant finds nothing new.
        assert!(reg.pop_expired(base + Duration::from_millis(100)).is_empty());
        // c's deadline stands and is now the earliest.
        assert_eq!(reg.next_deadline(), Some(base + Duration::from_secs(3600)));
    }
}
