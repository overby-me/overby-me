//! The `RuntimeInfo` encapsulates all information rust-systemd needs to do its job. The units, the pid and filedescriptors and the rust-systemd config.
//!
//! In the lifetime of rust-systemd there will only ever be one `RuntimeInfo` which is passed wrapped inside the `ArcMutRuntimeInfo`.
//!
//! The idea here is to make as much as possible concurrently readable while still being able to get exclusive access to e.g. remove units.
//! Note that units themselves contain `RWLocks` so they can be worked on concurrently as long as no `write()` lock is placed on the `RuntimeInfo`.

use crate::fd_store::FDStore;
use crate::platform::EventFd;
use crate::units::{ServiceType, Unit, UnitId};

use nix::unistd::Pid;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

pub type UnitTable = HashMap<UnitId, Unit>;
pub type MutFDStore = RwLock<FDStore>;

/// Shared handle to the PID table, accessible without the RuntimeInfo RwLock.
///
/// The PID table is wrapped in `Arc<Mutex<…>>` so that the signal handler can
/// update entries (e.g. `Service` → `ServiceExited`) **without** acquiring a
/// read lock on `RuntimeInfo`.  This breaks a 3-way deadlock:
///
///   1. Activation threads hold **read locks** on RuntimeInfo while polling
///      `wait_for_service` (which checks the PID table for `ServiceExited`).
///   2. A `systemctl` command (e.g. from a udev rule) tries to acquire a
///      **write lock** — it blocks because readers hold locks, and on glibc's
///      writer-preferring `pthread_rwlock` all *new* readers are also blocked.
///   3. The service-exit handler thread needs a **read lock** to update the
///      PID table — but it is blocked by the pending writer from (2).
///
/// By giving the signal handler a cloned `Arc` it can update the PID table
/// directly, allowing `wait_for_service` to observe `ServiceExited` and
/// release the read lock, which in turn unblocks the writer and the exit
/// handler.
pub type ArcMutPidTable = Arc<Mutex<PidTable>>;

/// Set of unit IDs that are part of a pending activation (started via --no-block).
/// Used by `list-jobs` to determine which units are "waiting" vs "running".
pub type PendingActivations = Arc<Mutex<std::collections::HashSet<UnitId>>>;

/// Manager environment variables, accessible via show-environment/set-environment.
pub type ManagerEnvironment = Arc<Mutex<HashMap<String, String>>>;

/// Per-unit markers (e.g. "needs-restart"), keyed by unit name.
pub type UnitMarkers = Arc<Mutex<HashMap<String, Vec<String>>>>;

/// This will be passed through to all the different threads as a central state struct
pub struct RuntimeInfo {
    pub unit_table: UnitTable,
    pub pid_table: ArcMutPidTable,
    pub fd_store: MutFDStore,
    pub config: crate::config::Config,
    pub stdout_eventfd: EventFd,
    pub stderr_eventfd: EventFd,
    pub notification_eventfd: EventFd,
    pub socket_activation_eventfd: EventFd,
    pub pending_activations: PendingActivations,
    pub manager_environment: ManagerEnvironment,
    pub unit_markers: UnitMarkers,
}

impl RuntimeInfo {
    pub fn notify_eventfds(&self) {
        crate::platform::notify_event_fd(self.stdout_eventfd);
        crate::platform::notify_event_fd(self.stderr_eventfd);
        crate::platform::notify_event_fd(self.notification_eventfd);
        crate::platform::notify_event_fd(self.socket_activation_eventfd);
    }
}

pub type ArcMutRuntimeInfo = Arc<RwLock<RuntimeInfo>>;

/// The `PidTable` holds info about all launched processes
pub type PidTable = HashMap<Pid, PidEntry>;

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
/// A process can be launched for these reasons. How an exit is handled depends
/// on this reason (e.g. oneshot services are supposed to exit. Normal services should not exit.)
pub enum PidEntry {
    Service(UnitId, ServiceType),
    ServiceExited(crate::signal_handler::ChildTermination),
    Helper(UnitId, String),
    HelperExited(crate::signal_handler::ChildTermination),
}
