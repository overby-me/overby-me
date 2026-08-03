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
    std::env::var("SYSTEMD_RS_JOB_GRAPH").is_ok_and(|v| v == "1")
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
}
