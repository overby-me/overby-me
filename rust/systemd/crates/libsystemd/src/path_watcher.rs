//! Path unit monitoring — watches filesystem paths and triggers associated
//! service units when path conditions are met.
//!
//! Uses Linux inotify for instant notification when watched paths change,
//! with automatic poll-based fallback when inotify is unavailable (e.g.
//! restricted containers or non-Linux systems). For paths that don't exist
//! yet, the parent directory is watched for creation events, and once the
//! target appears, a direct watch is added.
//!
//! ## Inotify watch strategy per condition type
//!
//! | Condition           | Watches                                            |
//! |---------------------|----------------------------------------------------|
//! | `PathExists=`       | Parent dir for `IN_CREATE \| IN_MOVED_TO`          |
//! | `PathExistsGlob=`   | Parent dir for `IN_CREATE \| IN_MOVED_TO`          |
//! | `PathChanged=`      | Path for `IN_ATTRIB \| IN_DELETE_SELF \| IN_MOVE_SELF`, parent for create/move |
//! | `PathModified=`     | Path for `IN_CLOSE_WRITE \| IN_DELETE_SELF \| IN_MOVE_SELF`, parent for create/move |
//! | `DirectoryNotEmpty=`| Dir for `IN_CREATE \| IN_MOVED_TO`                 |

use log::{debug, info, trace, warn};
use std::collections::{HashMap, HashSet};
use std::os::fd::{AsFd, AsRawFd};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::lock_ext::RwLockExt;
use crate::runtime_info::ArcMutRuntimeInfo;
use crate::units::{
    ActivationSource, PathCondition, PathConfig, Specific, StatusStarted, StatusStopped, UnitId,
    UnitStatus,
};

/// How often the path watcher thread does a full poll-based scan as a
/// safety net alongside inotify.  This catches any events that inotify
/// might miss (e.g. on network filesystems, or if a watch was lost).
const POLL_FALLBACK_INTERVAL: Duration = Duration::from_secs(10);

/// Timeout for `poll()` on the inotify fd.  We wake up this often even
/// if no inotify events arrive, to do bookkeeping (check for new path
/// units, refresh watches for paths whose parents have appeared, etc.).
const INOTIFY_POLL_TIMEOUT_MS: i32 = 500;

/// Initial delay before the first check, to let the system finish booting.
const STARTUP_DELAY: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// Inotify watch management
// ---------------------------------------------------------------------------

use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify, WatchDescriptor};

/// Flags for watching a parent directory for the appearance of a child.
const PARENT_WATCH_FLAGS: AddWatchFlags = AddWatchFlags::from_bits_truncate(
    AddWatchFlags::IN_CREATE.bits()
        | AddWatchFlags::IN_MOVED_TO.bits()
        | AddWatchFlags::IN_DONT_FOLLOW.bits(),
);

/// Flags for `PathChanged=` direct watches.
const PATH_CHANGED_FLAGS: AddWatchFlags = AddWatchFlags::from_bits_truncate(
    AddWatchFlags::IN_ATTRIB.bits()
        | AddWatchFlags::IN_CLOSE_WRITE.bits()
        | AddWatchFlags::IN_CREATE.bits()
        | AddWatchFlags::IN_DELETE.bits()
        | AddWatchFlags::IN_MOVED_FROM.bits()
        | AddWatchFlags::IN_MOVED_TO.bits()
        | AddWatchFlags::IN_DELETE_SELF.bits()
        | AddWatchFlags::IN_MOVE_SELF.bits()
        | AddWatchFlags::IN_DONT_FOLLOW.bits(),
);

/// Flags for `PathModified=` direct watches.
const PATH_MODIFIED_FLAGS: AddWatchFlags = AddWatchFlags::from_bits_truncate(
    AddWatchFlags::IN_CLOSE_WRITE.bits()
        | AddWatchFlags::IN_CREATE.bits()
        | AddWatchFlags::IN_MOVED_TO.bits()
        | AddWatchFlags::IN_DELETE_SELF.bits()
        | AddWatchFlags::IN_MOVE_SELF.bits()
        | AddWatchFlags::IN_DONT_FOLLOW.bits(),
);

/// Flags for `DirectoryNotEmpty=` watches.
const DIR_NOT_EMPTY_FLAGS: AddWatchFlags = AddWatchFlags::from_bits_truncate(
    AddWatchFlags::IN_CREATE.bits()
        | AddWatchFlags::IN_MOVED_TO.bits()
        | AddWatchFlags::IN_DONT_FOLLOW.bits(),
);

/// An entry tracking one inotify watch descriptor back to its owning
/// path unit and condition.
#[derive(Debug, Clone)]
struct WatchEntry {
    /// Name of the `.path` unit that owns this watch.
    path_unit_name: String,
    /// Index into the unit's `PathConfig.conditions` vec.
    condition_index: usize,
    /// Whether this watch is on the parent directory (waiting for the
    /// target to appear) or directly on the target path.
    is_parent_watch: bool,
}

/// Manages inotify watches for all active path units.
struct PathInotify {
    inotify: Inotify,
    /// Map from watch descriptor to the watch entry metadata.
    /// Multiple entries can share the same WD when inotify merges
    /// watches on the same path (e.g. parent-dir watch for two
    /// conditions in the same directory).
    watches: HashMap<WatchDescriptor, Vec<WatchEntry>>,
    /// Set of `(unit_name, condition_index)` pairs that currently have an
    /// active watch (direct or parent).  Used to avoid adding duplicate
    /// watches.
    active_conditions: HashMap<(String, usize), WatchDescriptor>,
}

impl PathInotify {
    /// Try to create a new inotify-based path watcher.
    /// Returns `None` if inotify is unavailable.
    fn new() -> Option<Self> {
        let inotify = match Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC) {
            Ok(i) => i,
            Err(e) => {
                debug!("inotify_init1 failed, path watcher will use polling: {}", e);
                return None;
            }
        };
        Some(PathInotify {
            inotify,
            watches: HashMap::new(),
            active_conditions: HashMap::new(),
        })
    }

    /// Ensure watches exist for every condition of the given path unit.
    fn ensure_watches(&mut self, unit_name: &str, conf: &PathConfig) {
        for (ci, condition) in conf.conditions.iter().enumerate() {
            let key = (unit_name.to_owned(), ci);
            if self.active_conditions.contains_key(&key) {
                continue;
            }
            self.add_watch_for_condition(unit_name, ci, condition);
        }
    }

    /// Add the appropriate inotify watch for a single condition.
    fn add_watch_for_condition(
        &mut self,
        unit_name: &str,
        condition_index: usize,
        condition: &PathCondition,
    ) {
        let key = (unit_name.to_owned(), condition_index);
        let path_str = condition.path();
        let path = Path::new(path_str);

        // Determine watch flags based on condition type.
        let (direct_flags, need_direct) = match condition {
            PathCondition::PathExists(_) => (PARENT_WATCH_FLAGS, false),
            PathCondition::PathExistsGlob(_) => (PARENT_WATCH_FLAGS, false),
            PathCondition::PathChanged(_) => (PATH_CHANGED_FLAGS, true),
            PathCondition::PathModified(_) => (PATH_MODIFIED_FLAGS, true),
            PathCondition::DirectoryNotEmpty(_) => (DIR_NOT_EMPTY_FLAGS, true),
        };

        if need_direct && path.exists() {
            // Watch the path itself.
            match self.inotify.add_watch(path, direct_flags) {
                Ok(wd) => {
                    trace!(
                        "inotify: direct watch on {:?} for {}[{}]",
                        path, unit_name, condition_index
                    );
                    self.watches.entry(wd).or_default().push(WatchEntry {
                        path_unit_name: unit_name.to_owned(),
                        condition_index,
                        is_parent_watch: false,
                    });
                    self.active_conditions.insert(key, wd);
                    return;
                }
                Err(e) => {
                    debug!("inotify: failed to watch {:?}: {}", path, e);
                    // Fall through to parent watch.
                }
            }
        }

        // Watch the parent directory for creation events.
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("/"),
        };

        if parent.is_dir() {
            match self.inotify.add_watch(parent, PARENT_WATCH_FLAGS) {
                Ok(wd) => {
                    trace!(
                        "inotify: parent watch on {:?} for {}[{}]",
                        parent, unit_name, condition_index
                    );
                    self.watches.entry(wd).or_default().push(WatchEntry {
                        path_unit_name: unit_name.to_owned(),
                        condition_index,
                        is_parent_watch: true,
                    });
                    self.active_conditions.insert(key, wd);
                }
                Err(e) => {
                    debug!("inotify: failed to watch parent {:?}: {}", parent, e);
                }
            }
        }
    }

    /// Remove all watches for a given path unit (e.g. when it's deactivated).
    fn remove_watches_for_unit(&mut self, unit_name: &str) {
        // Collect WDs that have entries belonging to this unit.
        let affected_wds: Vec<WatchDescriptor> = self
            .watches
            .iter()
            .filter(|(_, entries)| entries.iter().any(|e| e.path_unit_name == unit_name))
            .map(|(wd, _)| *wd)
            .collect();

        for wd in affected_wds {
            if let Some(entries) = self.watches.get_mut(&wd) {
                // Remove active_conditions for entries belonging to this unit.
                for entry in entries.iter().filter(|e| e.path_unit_name == unit_name) {
                    let key = (entry.path_unit_name.clone(), entry.condition_index);
                    self.active_conditions.remove(&key);
                }
                // Remove entries belonging to this unit from the vec.
                entries.retain(|e| e.path_unit_name != unit_name);
                // If no entries remain for this WD, remove the inotify watch.
                if entries.is_empty() {
                    self.watches.remove(&wd);
                    let _ = self.inotify.rm_watch(wd);
                }
            }
        }
    }

    /// Drain pending inotify events and return the set of path unit names
    /// that had relevant events.
    fn drain_events(&mut self) -> HashMap<String, bool> {
        let mut triggered_units: HashMap<String, bool> = HashMap::new();

        match self.inotify.read_events() {
            Ok(events) => {
                for event in &events {
                    // IN_IGNORED means the watch was removed (e.g.
                    // watched file was deleted).  We'll re-add it on
                    // the next ensure_watches pass.
                    if event.mask.contains(AddWatchFlags::IN_IGNORED) {
                        if let Some(entries) = self.watches.remove(&event.wd) {
                            for entry in &entries {
                                let key = (entry.path_unit_name.clone(), entry.condition_index);
                                self.active_conditions.remove(&key);
                                trace!(
                                    "inotify: watch removed (IN_IGNORED) for {}[{}]",
                                    entry.path_unit_name, entry.condition_index
                                );
                            }
                        }
                        continue;
                    }

                    if let Some(entries) = self.watches.get(&event.wd) {
                        for entry in entries {
                            trace!(
                                "inotify: event {:?} for {}[{}] (parent={})",
                                event.mask,
                                entry.path_unit_name,
                                entry.condition_index,
                                entry.is_parent_watch
                            );
                            triggered_units.insert(entry.path_unit_name.clone(), true);
                        }
                    }
                }
            }
            Err(ref e)
                if e.to_string().contains("EAGAIN") || e.to_string().contains("EWOULDBLOCK") =>
            {
                // No events pending — normal for non-blocking mode.
            }
            Err(e) => {
                debug!("inotify read error: {}", e);
            }
        }

        triggered_units
    }

    /// Upgrade parent watches to direct watches when the target path now
    /// exists.  Called after draining events so that a parent-dir create
    /// event for the target causes us to set up the proper direct watch.
    fn refresh_watches(&mut self, unit_name: &str, conf: &PathConfig) {
        for (ci, condition) in conf.conditions.iter().enumerate() {
            let key = (unit_name.to_owned(), ci);
            let is_parent = match self.active_conditions.get(&key) {
                Some(wd) => self
                    .watches
                    .get(wd)
                    .and_then(|entries| {
                        entries
                            .iter()
                            .find(|e| e.path_unit_name == unit_name && e.condition_index == ci)
                    })
                    .is_some_and(|e| e.is_parent_watch),
                None => true, // No watch at all, try to add one.
            };

            if !is_parent {
                continue;
            }

            let needs_direct = matches!(
                condition,
                PathCondition::PathChanged(_)
                    | PathCondition::PathModified(_)
                    | PathCondition::DirectoryNotEmpty(_)
            );

            if needs_direct && Path::new(condition.path()).exists() {
                // Remove the old entry for this condition from the WD's vec.
                if let Some(old_wd) = self.active_conditions.remove(&key)
                    && let Some(entries) = self.watches.get_mut(&old_wd)
                {
                    entries.retain(|e| !(e.path_unit_name == unit_name && e.condition_index == ci));
                    if entries.is_empty() {
                        self.watches.remove(&old_wd);
                        let _ = self.inotify.rm_watch(old_wd);
                    }
                }
                self.add_watch_for_condition(unit_name, ci, condition);
            }
        }
    }

    /// Get the raw fd for use with `poll()`.
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.inotify.as_fd().as_raw_fd()
    }
}

// ---------------------------------------------------------------------------
// Poll-based fallback state (also used as safety-net alongside inotify)
// ---------------------------------------------------------------------------

/// A snapshot of a path's state, used for change detection in poll mode.
#[derive(Debug, Clone, PartialEq)]
struct PathSnapshot {
    exists: bool,
    modified: Option<std::time::SystemTime>,
    len: Option<u64>,
}

impl PathSnapshot {
    fn capture(path: &str) -> Self {
        match std::fs::metadata(path) {
            Ok(meta) => PathSnapshot {
                exists: true,
                modified: meta.modified().ok(),
                len: Some(meta.len()),
            },
            Err(_) => PathSnapshot {
                exists: false,
                modified: None,
                len: None,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Shared baseline snapshots (written by activation code, consumed by watcher)
// ---------------------------------------------------------------------------

use std::sync::{LazyLock, Mutex};

/// Baselines recorded at path-unit activation time on the main thread.
/// The watcher thread drains these into its local `last_state` each iteration.
static ACTIVATION_BASELINES: LazyLock<Mutex<HashMap<String, PathSnapshot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Record baseline snapshots for a path unit's PathChanged/PathModified
/// conditions.  Called synchronously during path-unit activation so the
/// filesystem state is captured *before* `systemctl start` returns.
pub fn record_activation_baselines(unit_name: &str, conf: &crate::units::PathConfig) {
    use crate::units::PathCondition;
    let mut map = ACTIVATION_BASELINES.lock().unwrap();
    for condition in &conf.conditions {
        match condition {
            PathCondition::PathChanged(path) => {
                let key = format!("{unit_name}:changed:{path}");
                map.insert(key, PathSnapshot::capture(path));
            }
            PathCondition::PathModified(path) => {
                let key = format!("{unit_name}:modified:{path}");
                map.insert(key, PathSnapshot::capture(path));
            }
            _ => {}
        }
    }
}

/// Drain activation baselines into the watcher's local state.
/// Activation baselines overwrite existing entries because they represent
/// a fresh snapshot taken at unit-start time (the unit may have been
/// restarted, so old state is stale).
fn drain_activation_baselines(last_state: &mut HashMap<String, PathSnapshot>) {
    let mut map = ACTIVATION_BASELINES.lock().unwrap();
    for (key, snapshot) in map.drain() {
        last_state.insert(key, snapshot);
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Start the background path watcher thread.
///
/// This should be called after the initial unit activation is complete
/// (or at least after all path units have been loaded and started).
pub fn start_path_watcher_thread(run_info: ArcMutRuntimeInfo) {
    std::thread::Builder::new()
        .name("path-watcher".into())
        .spawn(move || {
            info!("Path watcher started");

            // Give the system a moment to finish initial activation.
            std::thread::sleep(STARTUP_DELAY);

            // Try to set up inotify; fall back to pure polling if unavailable.
            let mut inotify_state = PathInotify::new();

            if inotify_state.is_some() {
                info!("Path watcher using inotify with poll fallback");
            } else {
                info!("Path watcher using pure poll mode (inotify unavailable)");
            }

            // Rate limiting history.
            let mut trigger_history: HashMap<String, Vec<Instant>> = HashMap::new();

            // Poll-based change detection state.
            let mut last_state: HashMap<String, PathSnapshot> = HashMap::new();

            // Track when we last did a full poll scan.
            let mut last_full_poll = Instant::now();

            // Track which path units are currently being watched via inotify.
            let mut watched_units: HashMap<String, bool> = HashMap::new();

            // Path units that fired on the previous cycle.  Re-check them
            // immediately so that level-triggered conditions (PathExists,
            // DirectoryNotEmpty) that remain true can re-trigger quickly,
            // enabling proper rate-limit enforcement.
            let mut recheck_units: HashSet<String> = HashSet::new();

            loop {
                let now = Instant::now();
                let mut units_to_check: HashMap<String, bool> = HashMap::new();

                // Merge baselines that were recorded on the main thread when
                // path units were activated.  This is the primary mechanism for
                // avoiding the race where a file changes between `systemctl
                // start <path>` returning and the watcher noticing the unit.
                drain_activation_baselines(&mut last_state);

                // Re-check units that fired on the previous cycle.
                for name in recheck_units.drain() {
                    units_to_check.insert(name, true);
                }

                // --- Phase 1: Ensure inotify watches are up to date ---
                if let Some(ref mut ino) = inotify_state {
                    let newly_watched = sync_inotify_watches(ino, &run_info, &mut watched_units);
                    for name in newly_watched {
                        units_to_check.insert(name, true);
                    }
                }

                // --- Phase 2: Drain inotify events ---
                let inotify_triggered = if let Some(ref mut ino) = inotify_state {
                    // Use poll() to wait for events with a timeout.
                    wait_for_inotify_events(ino.as_raw_fd(), INOTIFY_POLL_TIMEOUT_MS);
                    ino.drain_events()
                } else {
                    // Pure poll mode: sleep and then check everything.
                    std::thread::sleep(Duration::from_secs(2));
                    HashMap::new()
                };

                for unit_name in inotify_triggered.keys() {
                    units_to_check.insert(unit_name.clone(), true);
                }

                // --- Phase 3: Periodic full poll scan as safety net ---
                let do_full_poll = now.duration_since(last_full_poll) >= POLL_FALLBACK_INTERVAL
                    || inotify_state.is_none();

                if do_full_poll {
                    // Add all active path units to the check set.
                    let ri = match run_info.try_read() {
                        Ok(g) => g,
                        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
                        Err(std::sync::TryLockError::WouldBlock) => {
                            // Skip full poll; don't update last_full_poll so we retry next cycle.
                            continue;
                        }
                    };
                    last_full_poll = now;
                    for unit in ri.unit_table.values() {
                        if let Specific::Path(path_specific) = &unit.specific {
                            let status = unit.common.status.read_poisoned().clone();
                            if matches!(status, UnitStatus::Started(StatusStarted::Running)) {
                                units_to_check.insert(unit.id.name.clone(), true);
                                // Refresh direct watches for newly-appeared paths.
                                if let Some(ref mut ino) = inotify_state {
                                    ino.refresh_watches(&unit.id.name, &path_specific.conf);
                                }
                            }
                        }
                    }
                }

                // Also refresh watches for inotify-triggered units (parent
                // watch may need upgrading to direct watch).
                if let Some(ref mut ino) = inotify_state {
                    let ri_opt = match run_info.try_read() {
                        Ok(g) => Some(g),
                        Err(std::sync::TryLockError::Poisoned(p)) => Some(p.into_inner()),
                        Err(std::sync::TryLockError::WouldBlock) => None,
                    };
                    if let Some(ri) = ri_opt {
                        for unit_name in inotify_triggered.keys() {
                            if let Some(unit) =
                                ri.unit_table.values().find(|u| u.id.name == *unit_name)
                                && let Specific::Path(path_specific) = &unit.specific
                            {
                                ino.refresh_watches(unit_name, &path_specific.conf);
                            }
                        }
                    }
                }

                // --- Phase 4: Check conditions and trigger ---
                if !units_to_check.is_empty() {
                    recheck_units = check_and_trigger_paths(
                        &run_info,
                        &units_to_check,
                        &mut trigger_history,
                        &mut last_state,
                    );
                }
            }
        })
        .expect("Failed to spawn path-watcher thread");
}

/// Use `poll(2)` to wait for inotify events or a timeout.
fn wait_for_inotify_events(fd: std::os::fd::RawFd, timeout_ms: i32) {
    use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
    let mut fds = [PollFd::new(
        unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) },
        PollFlags::POLLIN,
    )];
    let timeout = PollTimeout::try_from(timeout_ms).unwrap_or(PollTimeout::ZERO);
    let _ = poll(&mut fds, timeout);
}

/// Synchronize inotify watches with the currently active path units.
/// Returns the names of newly-watched units so the caller can schedule
/// an immediate condition check for them.
fn sync_inotify_watches(
    ino: &mut PathInotify,
    run_info: &ArcMutRuntimeInfo,
    watched_units: &mut HashMap<String, bool>,
) -> Vec<String> {
    let ri = match run_info.try_read() {
        Ok(g) => g,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return Vec::new(),
    };
    let mut current_units: HashMap<String, bool> = HashMap::new();
    let mut newly_watched: Vec<String> = Vec::new();

    for unit in ri.unit_table.values() {
        if let Specific::Path(path_specific) = &unit.specific {
            let status = unit.common.status.read_poisoned().clone();
            if matches!(status, UnitStatus::Started(StatusStarted::Running)) {
                let name = unit.id.name.clone();
                if !watched_units.contains_key(&name) {
                    newly_watched.push(name.clone());
                }
                current_units.insert(name, true);
                ino.ensure_watches(&unit.id.name, &path_specific.conf);
            }
        }
    }

    // Remove watches for path units that are no longer active.
    let stale: Vec<String> = watched_units
        .keys()
        .filter(|name| !current_units.contains_key(*name))
        .cloned()
        .collect();

    for name in &stale {
        ino.remove_watches_for_unit(name);
        watched_units.remove(name);
    }

    *watched_units = current_units;
    newly_watched
}

// ---------------------------------------------------------------------------
// Condition checking and triggering
// ---------------------------------------------------------------------------

/// Check path conditions for the given set of unit names and fire triggers.
/// Returns the set of path unit names that fired a trigger (so the caller
/// can re-check them on the next cycle — the condition may still be true).
fn check_and_trigger_paths(
    run_info: &ArcMutRuntimeInfo,
    units_to_check: &HashMap<String, bool>,
    trigger_history: &mut HashMap<String, Vec<Instant>>,
    last_state: &mut HashMap<String, PathSnapshot>,
) -> HashSet<String> {
    let mut fired = HashSet::new();
    let now = Instant::now();
    // (path_unit_id, target_unit_name, trigger_path)
    let mut paths_to_trigger: Vec<(UnitId, String, String)> = Vec::new();

    {
        let ri = match run_info.try_read() {
            Ok(g) => g,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return fired, // retry next cycle
        };
        for unit in ri.unit_table.values() {
            if !units_to_check.contains_key(&unit.id.name) {
                continue;
            }

            if let Specific::Path(path_specific) = &unit.specific {
                let status = unit.common.status.read_poisoned().clone();
                if !matches!(status, UnitStatus::Started(StatusStarted::Running)) {
                    continue;
                }

                let conf = &path_specific.conf;
                let path_name = &unit.id.name;
                let target_unit = &conf.unit;

                match should_trigger_path(conf, path_name, now, trigger_history, last_state) {
                    TriggerResult::Fire(trigger_path) => {
                        paths_to_trigger.push((unit.id.clone(), target_unit.clone(), trigger_path));
                    }
                    TriggerResult::RateLimited => {
                        // Transition to failed with trigger-limit-hit
                        info!(
                            "Path unit {} hit trigger rate limit, transitioning to failed",
                            path_name
                        );
                        {
                            let mut state = path_specific.state.write_poisoned();
                            state.result = crate::units::PathResult::TriggerLimitHit;
                        }
                        *unit.common.status.write_poisoned() = UnitStatus::Stopped(
                            StatusStopped::StoppedFinal,
                            vec![crate::units::UnitOperationErrorReason::GenericStartError(
                                "trigger-limit-hit".to_string(),
                            )],
                        );
                    }
                    TriggerResult::NoMatch => {}
                }
            }
        }
    }

    for (path_id, target_unit_name, trigger_path) in paths_to_trigger {
        info!(
            "Path unit {} triggered (path={}), activating {}",
            path_id.name, trigger_path, target_unit_name
        );

        trigger_history
            .entry(path_id.name.clone())
            .or_default()
            .push(now);

        fired.insert(path_id.name.clone());
        fire_path_target(run_info, &target_unit_name, &path_id.name, &trigger_path);
    }

    fired
}

/// Result of checking path trigger conditions.
enum TriggerResult {
    /// A condition matched; contains the trigger path.
    Fire(String),
    /// Conditions matched but the unit has been triggered too many times.
    RateLimited,
    /// No conditions matched.
    NoMatch,
}

/// Check if a path unit's conditions are met and it should trigger.
fn should_trigger_path(
    conf: &PathConfig,
    path_name: &str,
    now: Instant,
    trigger_history: &mut HashMap<String, Vec<Instant>>,
    last_state: &mut HashMap<String, PathSnapshot>,
) -> TriggerResult {
    // Check conditions first — we need to know if there's a match
    // before deciding whether to fire or rate-limit.
    let mut matched_path = None;
    for condition in &conf.conditions {
        if let Some(trigger_path) = check_condition(condition, path_name, last_state) {
            matched_path = Some(trigger_path);
            break;
        }
    }

    let Some(trigger_path) = matched_path else {
        return TriggerResult::NoMatch;
    };

    if is_rate_limited(conf, path_name, now, trigger_history) {
        trace!(
            "Path unit {} rate-limited, transitioning to failed",
            path_name
        );
        return TriggerResult::RateLimited;
    }

    TriggerResult::Fire(trigger_path)
}

/// Check if a single path condition is met.
/// Returns the trigger path if the condition matched, or None.
fn check_condition(
    condition: &PathCondition,
    path_name: &str,
    last_state: &mut HashMap<String, PathSnapshot>,
) -> Option<String> {
    match condition {
        PathCondition::PathExists(path) => {
            let exists = Path::new(path).exists();
            if exists {
                trace!("Path unit {}: PathExists={} satisfied", path_name, path);
                Some(path.clone())
            } else {
                None
            }
        }

        PathCondition::PathExistsGlob(pattern) => {
            if let Some(matched) = glob_match_first(pattern) {
                trace!(
                    "Path unit {}: PathExistsGlob={} satisfied (matched {})",
                    path_name, pattern, matched
                );
                Some(matched)
            } else {
                None
            }
        }

        PathCondition::PathChanged(path) => {
            let key = format!("{path_name}:changed:{path}");
            let current = PathSnapshot::capture(path);
            let changed = match last_state.get(&key) {
                Some(prev) => *prev != current,
                None => {
                    // First check — record state but don't trigger.
                    last_state.insert(key, current);
                    return None;
                }
            };
            last_state.insert(key, current);
            if changed {
                trace!("Path unit {}: PathChanged={} satisfied", path_name, path);
                Some(path.clone())
            } else {
                None
            }
        }

        PathCondition::PathModified(path) => {
            let key = format!("{path_name}:modified:{path}");
            let current = PathSnapshot::capture(path);
            let modified = match last_state.get(&key) {
                Some(prev) => *prev != current,
                None => {
                    // First check — record state but don't trigger.
                    last_state.insert(key, current);
                    return None;
                }
            };
            last_state.insert(key, current);
            if modified {
                trace!("Path unit {}: PathModified={} satisfied", path_name, path);
                Some(path.clone())
            } else {
                None
            }
        }

        PathCondition::DirectoryNotEmpty(path) => {
            let not_empty = is_directory_not_empty(path);
            if not_empty {
                trace!(
                    "Path unit {}: DirectoryNotEmpty={} satisfied",
                    path_name, path
                );
                Some(path.clone())
            } else {
                None
            }
        }
    }
}

/// Check if the path unit has been triggered too many times recently.
fn is_rate_limited(
    conf: &PathConfig,
    path_name: &str,
    now: Instant,
    trigger_history: &mut HashMap<String, Vec<Instant>>,
) -> bool {
    let interval = conf.trigger_limit_interval_sec;
    let burst = conf.trigger_limit_burst;

    if burst == 0 || interval.is_zero() {
        return false;
    }

    let history = trigger_history.entry(path_name.to_owned()).or_default();

    // Remove entries older than the interval.
    history.retain(|t| now.duration_since(*t) < interval);

    history.len() >= burst as usize
}

/// Check if a directory exists and is not empty.
fn is_directory_not_empty(path: &str) -> bool {
    match std::fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_some(),
        Err(_) => false,
    }
}

/// Simple glob matching for `PathExistsGlob=`.
/// Returns the first matching filesystem path, or None.
fn glob_match_first(pattern: &str) -> Option<String> {
    let path = Path::new(pattern);
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("/"),
    };
    let file_pattern = match path.file_name() {
        Some(f) => f.to_string_lossy().to_string(),
        None => return None,
    };

    let entries = match std::fs::read_dir(parent) {
        Ok(e) => e,
        Err(_) => return None,
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if simple_glob_match(&file_pattern, &name) {
            return Some(parent.join(&name).to_string_lossy().to_string());
        }
    }
    None
}

/// Simple glob matching supporting `*` and `?` wildcards.
fn simple_glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_match_recursive(&pat, &txt, 0, 0)
}

fn glob_match_recursive(pat: &[char], txt: &[char], pi: usize, ti: usize) -> bool {
    if pi == pat.len() {
        return ti == txt.len();
    }
    if pat[pi] == '*' {
        // Try matching zero or more characters.
        for skip in 0..=(txt.len() - ti) {
            if glob_match_recursive(pat, txt, pi + 1, ti + skip) {
                return true;
            }
        }
        false
    } else if pat[pi] == '?' {
        if ti < txt.len() {
            glob_match_recursive(pat, txt, pi + 1, ti + 1)
        } else {
            false
        }
    } else if ti < txt.len() && pat[pi] == txt[ti] {
        glob_match_recursive(pat, txt, pi + 1, ti + 1)
    } else {
        false
    }
}

/// Set TRIGGER_PATH and TRIGGER_UNIT on the target service's state.
fn set_trigger_info(unit: &crate::units::Unit, trigger_unit_name: &str, trigger_path: &str) {
    if let Specific::Service(specific) = &unit.specific {
        let mut state = specific.state.write_poisoned();
        state.srvc.trigger_path = Some(trigger_path.to_owned());
        state.srvc.trigger_unit = Some(trigger_unit_name.to_owned());
    }
}

/// Activate the target unit for a path trigger.
fn fire_path_target(
    run_info: &ArcMutRuntimeInfo,
    target_unit_name: &str,
    path_unit_name: &str,
    trigger_path: &str,
) {
    let ri = match run_info.try_read() {
        Ok(g) => g,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return, // retry next cycle
    };

    let target_unit = ri
        .unit_table
        .values()
        .find(|u| u.id.name == target_unit_name);

    match target_unit {
        Some(unit) => {
            set_trigger_info(unit, path_unit_name, trigger_path);
            let status = unit.common.status.read_poisoned().clone();
            match status {
                UnitStatus::Started(_) => {
                    // Real systemd stops monitoring once the target is active
                    // and only re-checks when the target deactivates.  Skip
                    // the trigger to avoid unnecessary stop→start cycles.
                    trace!(
                        "Path target {} is already running, skipping trigger",
                        target_unit_name
                    );
                }
                _ => {
                    let id = unit.id.clone();
                    drop(ri);
                    match crate::units::activate_unit(
                        id,
                        &run_info.read_poisoned(),
                        ActivationSource::TriggerActivation,
                    ) {
                        Ok(_) => {
                            info!("Path triggered: started {}", target_unit_name);
                        }
                        Err(e) => {
                            warn!("Path failed to start {}: {}", target_unit_name, e);
                        }
                    }
                }
            }
        }
        None => {
            debug!(
                "Path target {} not found in unit table, attempting on-demand load",
                target_unit_name
            );
            drop(ri);

            let ri = run_info.read_poisoned();
            if let Some(unit) = ri
                .unit_table
                .values()
                .find(|u| u.id.name == target_unit_name)
            {
                set_trigger_info(unit, path_unit_name, trigger_path);
                let id = unit.id.clone();
                drop(ri);
                match crate::units::activate_unit(
                    id,
                    &run_info.read_poisoned(),
                    ActivationSource::TriggerActivation,
                ) {
                    Ok(_) => info!("Path triggered: started {} (on-demand)", target_unit_name),
                    Err(e) => warn!(
                        "Path failed to start {} (on-demand): {}",
                        target_unit_name, e
                    ),
                }
            } else {
                warn!(
                    "Path target {} not found and could not be loaded",
                    target_unit_name
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // -- PathSnapshot tests --------------------------------------------------

    #[test]
    fn test_path_snapshot_capture_nonexistent() {
        let snap = PathSnapshot::capture("/nonexistent/path/unlikely/to/exist");
        assert!(!snap.exists);
        assert!(snap.modified.is_none());
        assert!(snap.len.is_none());
    }

    #[test]
    fn test_path_snapshot_capture_existing() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "hello").unwrap();
        let snap = PathSnapshot::capture(file.to_str().unwrap());
        assert!(snap.exists);
        assert!(snap.modified.is_some());
        assert_eq!(snap.len, Some(5));
    }

    #[test]
    fn test_path_snapshot_equality() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "hello").unwrap();
        let snap1 = PathSnapshot::capture(file.to_str().unwrap());
        let snap2 = PathSnapshot::capture(file.to_str().unwrap());
        assert_eq!(snap1, snap2);

        // After writing different content, the snapshot should differ
        // (at least len changes).
        fs::write(&file, "hello world").unwrap();
        let snap3 = PathSnapshot::capture(file.to_str().unwrap());
        assert_ne!(snap1, snap3);
    }

    // -- Glob matching tests -------------------------------------------------

    #[test]
    fn test_simple_glob_match_star() {
        assert!(simple_glob_match("*.conf", "test.conf"));
        assert!(simple_glob_match("*.conf", ".conf"));
        assert!(!simple_glob_match("*.conf", "test.txt"));
        assert!(simple_glob_match("*", "anything"));
        assert!(simple_glob_match("*", ""));
        assert!(simple_glob_match("test*end", "test_middle_end"));
        assert!(!simple_glob_match("test*end", "test_middle_enx"));
    }

    #[test]
    fn test_simple_glob_match_question() {
        assert!(simple_glob_match("?.conf", "a.conf"));
        assert!(!simple_glob_match("?.conf", "ab.conf"));
        assert!(!simple_glob_match("?.conf", ".conf"));
    }

    #[test]
    fn test_simple_glob_match_exact() {
        assert!(simple_glob_match("test.conf", "test.conf"));
        assert!(!simple_glob_match("test.conf", "test.txt"));
    }

    #[test]
    fn test_simple_glob_match_combined() {
        assert!(simple_glob_match("test?.c*", "test1.conf"));
        assert!(simple_glob_match("test?.c*", "testA.c"));
        assert!(!simple_glob_match("test?.c*", "test12.conf"));
    }

    // -- is_directory_not_empty tests ----------------------------------------

    #[test]
    fn test_is_directory_not_empty_with_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("file.txt"), "data").unwrap();
        assert!(is_directory_not_empty(dir.path().to_str().unwrap()));
    }

    #[test]
    fn test_is_directory_not_empty_empty_dir() {
        let dir = TempDir::new().unwrap();
        assert!(!is_directory_not_empty(dir.path().to_str().unwrap()));
    }

    #[test]
    fn test_is_directory_not_empty_nonexistent() {
        assert!(!is_directory_not_empty("/nonexistent/path/xyz"));
    }

    // -- check_condition tests -----------------------------------------------

    #[test]
    fn test_check_condition_path_exists() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("trigger");
        let mut last_state = HashMap::new();

        // File doesn't exist yet.
        let cond = PathCondition::PathExists(file.to_str().unwrap().to_owned());
        assert!(check_condition(&cond, "test.path", &mut last_state).is_none());

        // Create the file.
        fs::write(&file, "").unwrap();
        assert!(check_condition(&cond, "test.path", &mut last_state).is_some());
    }

    #[test]
    fn test_check_condition_directory_not_empty() {
        let dir = TempDir::new().unwrap();
        let watch_dir = dir.path().join("watched");
        fs::create_dir(&watch_dir).unwrap();
        let mut last_state = HashMap::new();

        let cond = PathCondition::DirectoryNotEmpty(watch_dir.to_str().unwrap().to_owned());

        // Empty directory.
        assert!(check_condition(&cond, "test.path", &mut last_state).is_none());

        // Add a file.
        fs::write(watch_dir.join("file.txt"), "data").unwrap();
        assert!(check_condition(&cond, "test.path", &mut last_state).is_some());
    }

    #[test]
    fn test_check_condition_path_changed() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("watched.txt");
        fs::write(&file, "initial").unwrap();
        let mut last_state = HashMap::new();

        let cond = PathCondition::PathChanged(file.to_str().unwrap().to_owned());

        // First check records state, doesn't trigger.
        assert!(check_condition(&cond, "test.path", &mut last_state).is_none());

        // No change.
        assert!(check_condition(&cond, "test.path", &mut last_state).is_none());

        // Change file content.
        fs::write(&file, "modified content").unwrap();
        assert!(check_condition(&cond, "test.path", &mut last_state).is_some());

        // No further change.
        assert!(check_condition(&cond, "test.path", &mut last_state).is_none());
    }

    #[test]
    fn test_check_condition_path_modified() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("watched.txt");
        fs::write(&file, "initial").unwrap();
        let mut last_state = HashMap::new();

        let cond = PathCondition::PathModified(file.to_str().unwrap().to_owned());

        // First check records state.
        assert!(check_condition(&cond, "test.path", &mut last_state).is_none());

        // Modify.
        fs::write(&file, "changed!").unwrap();
        assert!(check_condition(&cond, "test.path", &mut last_state).is_some());
    }

    #[test]
    fn test_check_condition_path_exists_glob() {
        let dir = TempDir::new().unwrap();
        let pattern = format!("{}/*.conf", dir.path().display());
        let mut last_state = HashMap::new();

        let cond = PathCondition::PathExistsGlob(pattern.clone());

        // No .conf files yet.
        assert!(check_condition(&cond, "test.path", &mut last_state).is_none());

        // Create one.
        fs::write(dir.path().join("test.conf"), "data").unwrap();
        assert!(check_condition(&cond, "test.path", &mut last_state).is_some());
    }

    // -- Rate limiting tests -------------------------------------------------

    #[test]
    fn test_rate_limiting() {
        let conf = PathConfig {
            conditions: vec![],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 3,
            unit: "test.service".to_owned(),
        };

        let mut trigger_history: HashMap<String, Vec<Instant>> = HashMap::new();
        let now = Instant::now();

        // First 3 triggers are allowed.
        assert!(!is_rate_limited(
            &conf,
            "test.path",
            now,
            &mut trigger_history
        ));
        trigger_history
            .entry("test.path".to_owned())
            .or_default()
            .push(now);
        assert!(!is_rate_limited(
            &conf,
            "test.path",
            now,
            &mut trigger_history
        ));
        trigger_history
            .entry("test.path".to_owned())
            .or_default()
            .push(now);
        assert!(!is_rate_limited(
            &conf,
            "test.path",
            now,
            &mut trigger_history
        ));
        trigger_history
            .entry("test.path".to_owned())
            .or_default()
            .push(now);

        // 4th should be rate limited.
        assert!(is_rate_limited(
            &conf,
            "test.path",
            now,
            &mut trigger_history
        ));
    }

    #[test]
    fn test_rate_limiting_disabled() {
        let conf = PathConfig {
            conditions: vec![],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::ZERO,
            trigger_limit_burst: 0,
            unit: "test.service".to_owned(),
        };

        let mut trigger_history: HashMap<String, Vec<Instant>> = HashMap::new();
        let now = Instant::now();

        // With both set to 0, rate limiting is disabled.
        assert!(!is_rate_limited(
            &conf,
            "test.path",
            now,
            &mut trigger_history
        ));
    }

    #[test]
    fn test_rate_limiting_expired_entries() {
        let conf = PathConfig {
            conditions: vec![],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_millis(50),
            trigger_limit_burst: 2,
            unit: "test.service".to_owned(),
        };

        let mut trigger_history: HashMap<String, Vec<Instant>> = HashMap::new();
        let past = Instant::now() - Duration::from_millis(100);

        // Add entries that are already older than the interval.
        trigger_history
            .entry("test.path".to_owned())
            .or_default()
            .extend([past, past]);

        let now = Instant::now();
        // Old entries should be cleaned up, so we are NOT rate limited.
        assert!(!is_rate_limited(
            &conf,
            "test.path",
            now,
            &mut trigger_history
        ));
    }

    // -- PathCondition accessor test -----------------------------------------

    #[test]
    fn test_path_condition_path_accessor() {
        let cases: Vec<(PathCondition, &str)> = vec![
            (PathCondition::PathExists("/a".into()), "/a"),
            (PathCondition::PathExistsGlob("/b/*".into()), "/b/*"),
            (PathCondition::PathChanged("/c".into()), "/c"),
            (PathCondition::PathModified("/d".into()), "/d"),
            (PathCondition::DirectoryNotEmpty("/e".into()), "/e"),
        ];
        for (cond, expected) in cases {
            assert_eq!(cond.path(), expected);
        }
    }

    // -- should_trigger_path tests -------------------------------------------

    #[test]
    fn test_should_trigger_path_no_conditions() {
        let conf = PathConfig {
            conditions: vec![],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "test.service".to_owned(),
        };
        let mut history = HashMap::new();
        let mut state = HashMap::new();
        assert!(matches!(
            should_trigger_path(&conf, "test.path", Instant::now(), &mut history, &mut state),
            TriggerResult::NoMatch
        ));
    }

    #[test]
    fn test_should_trigger_path_exists_satisfied() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("trigger");
        fs::write(&file, "").unwrap();

        let conf = PathConfig {
            conditions: vec![PathCondition::PathExists(file.to_str().unwrap().to_owned())],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "test.service".to_owned(),
        };
        let mut history = HashMap::new();
        let mut state = HashMap::new();
        assert!(matches!(
            should_trigger_path(&conf, "test.path", Instant::now(), &mut history, &mut state),
            TriggerResult::Fire(_)
        ));
    }

    #[test]
    fn test_should_trigger_path_exists_not_satisfied() {
        let conf = PathConfig {
            conditions: vec![PathCondition::PathExists(
                "/nonexistent/path/xyz".to_owned(),
            )],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "test.service".to_owned(),
        };
        let mut history = HashMap::new();
        let mut state = HashMap::new();
        assert!(matches!(
            should_trigger_path(&conf, "test.path", Instant::now(), &mut history, &mut state),
            TriggerResult::NoMatch
        ));
    }

    // -- Inotify-specific tests ----------------------------------------------

    #[test]
    fn test_inotify_new_succeeds() {
        let ino = PathInotify::new();
        assert!(ino.is_some(), "inotify should be available on Linux");
    }

    #[test]
    fn test_inotify_drain_events_empty() {
        let mut ino = PathInotify::new().expect("inotify available");
        let events = ino.drain_events();
        assert!(events.is_empty());
    }

    #[test]
    fn test_inotify_ensure_watches_path_exists() {
        let dir = TempDir::new().unwrap();
        let trigger = dir.path().join("trigger");

        let conf = PathConfig {
            conditions: vec![PathCondition::PathExists(
                trigger.to_str().unwrap().to_owned(),
            )],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "test.service".to_owned(),
        };

        let mut ino = PathInotify::new().expect("inotify available");
        ino.ensure_watches("test.path", &conf);

        // Should have added a parent-directory watch.
        assert_eq!(ino.active_conditions.len(), 1);
        let key = ("test.path".to_owned(), 0);
        assert!(ino.active_conditions.contains_key(&key));

        let wd = ino.active_conditions[&key];
        let entries = &ino.watches[&wd];
        let entry = entries
            .iter()
            .find(|e| e.path_unit_name == "test.path" && e.condition_index == 0)
            .expect("should have entry for condition 0");
        assert!(entry.is_parent_watch);
    }

    #[test]
    fn test_inotify_ensure_watches_path_changed_existing() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("watched.txt");
        fs::write(&file, "content").unwrap();

        let conf = PathConfig {
            conditions: vec![PathCondition::PathChanged(
                file.to_str().unwrap().to_owned(),
            )],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "test.path".to_owned(),
        };

        let mut ino = PathInotify::new().expect("inotify available");
        ino.ensure_watches("test.path", &conf);

        // For PathChanged on an existing file, should have a direct watch.
        let key = ("test.path".to_owned(), 0);
        assert!(ino.active_conditions.contains_key(&key));

        let wd = ino.active_conditions[&key];
        let entries = &ino.watches[&wd];
        let entry = entries
            .iter()
            .find(|e| e.path_unit_name == "test.path" && e.condition_index == 0)
            .expect("should have entry for condition 0");
        assert!(!entry.is_parent_watch);
    }

    #[test]
    fn test_inotify_ensure_watches_path_changed_nonexistent() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("does-not-exist.txt");

        let conf = PathConfig {
            conditions: vec![PathCondition::PathChanged(
                file.to_str().unwrap().to_owned(),
            )],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "test.path".to_owned(),
        };

        let mut ino = PathInotify::new().expect("inotify available");
        ino.ensure_watches("test.path", &conf);

        // PathChanged on non-existing file: should watch parent directory.
        let key = ("test.path".to_owned(), 0);
        assert!(ino.active_conditions.contains_key(&key));

        let wd = ino.active_conditions[&key];
        let entries = &ino.watches[&wd];
        let entry = entries
            .iter()
            .find(|e| e.path_unit_name == "test.path" && e.condition_index == 0)
            .expect("should have entry for condition 0");
        assert!(entry.is_parent_watch);
    }

    #[test]
    fn test_inotify_ensure_watches_directory_not_empty() {
        let dir = TempDir::new().unwrap();
        let watch_dir = dir.path().join("watched");
        fs::create_dir(&watch_dir).unwrap();

        let conf = PathConfig {
            conditions: vec![PathCondition::DirectoryNotEmpty(
                watch_dir.to_str().unwrap().to_owned(),
            )],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "test.path".to_owned(),
        };

        let mut ino = PathInotify::new().expect("inotify available");
        ino.ensure_watches("test.path", &conf);

        // DirectoryNotEmpty on existing dir: should have a direct watch.
        let key = ("test.path".to_owned(), 0);
        assert!(ino.active_conditions.contains_key(&key));

        let wd = ino.active_conditions[&key];
        let entries = &ino.watches[&wd];
        let entry = entries
            .iter()
            .find(|e| e.path_unit_name == "test.path" && e.condition_index == 0)
            .expect("should have entry for condition 0");
        assert!(!entry.is_parent_watch);
    }

    #[test]
    fn test_inotify_ensure_watches_idempotent() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("trigger");

        let conf = PathConfig {
            conditions: vec![PathCondition::PathExists(file.to_str().unwrap().to_owned())],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "test.service".to_owned(),
        };

        let mut ino = PathInotify::new().expect("inotify available");
        ino.ensure_watches("test.path", &conf);
        let total_entries: usize = ino.watches.values().map(|v| v.len()).sum();
        assert_eq!(total_entries, 1);

        // Calling again should NOT add duplicate watches.
        ino.ensure_watches("test.path", &conf);
        let total_entries: usize = ino.watches.values().map(|v| v.len()).sum();
        assert_eq!(total_entries, 1);
    }

    #[test]
    fn test_inotify_remove_watches_for_unit() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("trigger");

        let conf = PathConfig {
            conditions: vec![
                PathCondition::PathExists(file.to_str().unwrap().to_owned()),
                PathCondition::DirectoryNotEmpty(dir.path().to_str().unwrap().to_owned()),
            ],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "test.service".to_owned(),
        };

        let mut ino = PathInotify::new().expect("inotify available");
        ino.ensure_watches("test.path", &conf);
        let total_entries: usize = ino.watches.values().map(|v| v.len()).sum();
        assert!(total_entries >= 1);

        ino.remove_watches_for_unit("test.path");
        let total_entries: usize = ino.watches.values().map(|v| v.len()).sum();
        assert_eq!(total_entries, 0);
        assert!(ino.active_conditions.is_empty());
    }

    #[test]
    fn test_inotify_remove_watches_for_unit_only_removes_target() {
        let dir = TempDir::new().unwrap();
        let file_a = dir.path().join("a");
        let file_b = dir.path().join("b");

        let conf_a = PathConfig {
            conditions: vec![PathCondition::PathExists(
                file_a.to_str().unwrap().to_owned(),
            )],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "a.service".to_owned(),
        };
        let conf_b = PathConfig {
            conditions: vec![PathCondition::PathExists(
                file_b.to_str().unwrap().to_owned(),
            )],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "b.service".to_owned(),
        };

        let mut ino = PathInotify::new().expect("inotify available");
        ino.ensure_watches("a.path", &conf_a);
        ino.ensure_watches("b.path", &conf_b);

        // Note: both watch the same parent dir, so inotify may merge
        // them into one WD.  But our bookkeeping should have entries
        // for both.
        let a_count: usize = ino
            .watches
            .values()
            .flat_map(|v| v.iter())
            .filter(|e| e.path_unit_name == "a.path")
            .count();
        let b_count: usize = ino
            .watches
            .values()
            .flat_map(|v| v.iter())
            .filter(|e| e.path_unit_name == "b.path")
            .count();
        assert_eq!(a_count, 1);
        assert_eq!(b_count, 1);

        ino.remove_watches_for_unit("a.path");

        let a_count: usize = ino
            .watches
            .values()
            .flat_map(|v| v.iter())
            .filter(|e| e.path_unit_name == "a.path")
            .count();
        let b_count: usize = ino
            .watches
            .values()
            .flat_map(|v| v.iter())
            .filter(|e| e.path_unit_name == "b.path")
            .count();
        assert_eq!(a_count, 0);
        assert_eq!(b_count, 1);
    }

    #[test]
    fn test_inotify_detects_file_creation() {
        let dir = TempDir::new().unwrap();
        let trigger = dir.path().join("trigger");

        let conf = PathConfig {
            conditions: vec![PathCondition::PathExists(
                trigger.to_str().unwrap().to_owned(),
            )],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "test.service".to_owned(),
        };

        let mut ino = PathInotify::new().expect("inotify available");
        ino.ensure_watches("test.path", &conf);

        // No events yet.
        let events = ino.drain_events();
        assert!(events.is_empty());

        // Create the trigger file.
        fs::write(&trigger, "hello").unwrap();

        // Give inotify a moment to deliver the event.
        std::thread::sleep(Duration::from_millis(50));

        let events = ino.drain_events();
        assert!(
            events.contains_key("test.path"),
            "should detect file creation via parent watch"
        );
    }

    #[test]
    fn test_inotify_detects_file_modification() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("watched.txt");
        fs::write(&file, "initial").unwrap();

        let conf = PathConfig {
            conditions: vec![PathCondition::PathModified(
                file.to_str().unwrap().to_owned(),
            )],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "test.path".to_owned(),
        };

        let mut ino = PathInotify::new().expect("inotify available");
        ino.ensure_watches("test.path", &conf);

        // Drain any setup noise.
        let _ = ino.drain_events();

        // Modify the file.
        fs::write(&file, "modified content").unwrap();
        std::thread::sleep(Duration::from_millis(50));

        let events = ino.drain_events();
        assert!(
            events.contains_key("test.path"),
            "should detect file modification via direct watch"
        );
    }

    #[test]
    fn test_inotify_detects_directory_not_empty() {
        let dir = TempDir::new().unwrap();
        let watch_dir = dir.path().join("watched");
        fs::create_dir(&watch_dir).unwrap();

        let conf = PathConfig {
            conditions: vec![PathCondition::DirectoryNotEmpty(
                watch_dir.to_str().unwrap().to_owned(),
            )],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "test.path".to_owned(),
        };

        let mut ino = PathInotify::new().expect("inotify available");
        ino.ensure_watches("test.path", &conf);

        // Drain setup noise.
        let _ = ino.drain_events();

        // Create a file in the directory.
        fs::write(watch_dir.join("entry.txt"), "data").unwrap();
        std::thread::sleep(Duration::from_millis(50));

        let events = ino.drain_events();
        assert!(
            events.contains_key("test.path"),
            "should detect new entry in watched directory"
        );
    }

    #[test]
    fn test_inotify_refresh_upgrades_parent_to_direct() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("target.txt");

        let conf = PathConfig {
            conditions: vec![PathCondition::PathChanged(
                file.to_str().unwrap().to_owned(),
            )],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "test.service".to_owned(),
        };

        let mut ino = PathInotify::new().expect("inotify available");
        ino.ensure_watches("test.path", &conf);

        // Should be a parent watch since the file doesn't exist.
        let key = ("test.path".to_owned(), 0);
        let wd = ino.active_conditions[&key];
        let entry = ino.watches[&wd]
            .iter()
            .find(|e| e.path_unit_name == "test.path" && e.condition_index == 0)
            .expect("entry");
        assert!(entry.is_parent_watch);

        // Create the file.
        fs::write(&file, "hello").unwrap();

        // Refresh should upgrade to a direct watch.
        ino.refresh_watches("test.path", &conf);

        let wd = ino.active_conditions[&key];
        let entry = ino.watches[&wd]
            .iter()
            .find(|e| e.path_unit_name == "test.path" && e.condition_index == 0)
            .expect("entry");
        assert!(
            !entry.is_parent_watch,
            "should have upgraded to direct watch after file creation"
        );
    }

    #[test]
    fn test_inotify_refresh_no_op_when_already_direct() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("target.txt");
        fs::write(&file, "data").unwrap();

        let conf = PathConfig {
            conditions: vec![PathCondition::PathModified(
                file.to_str().unwrap().to_owned(),
            )],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "test.service".to_owned(),
        };

        let mut ino = PathInotify::new().expect("inotify available");
        ino.ensure_watches("test.path", &conf);

        let key = ("test.path".to_owned(), 0);
        let wd_before = ino.active_conditions[&key];
        let entry = ino.watches[&wd_before]
            .iter()
            .find(|e| e.path_unit_name == "test.path" && e.condition_index == 0)
            .expect("entry");
        assert!(!entry.is_parent_watch);

        // Refresh should be a no-op.
        ino.refresh_watches("test.path", &conf);
        let wd_after = ino.active_conditions[&key];
        assert_eq!(wd_before, wd_after);
    }

    #[test]
    fn test_inotify_multiple_conditions_same_unit() {
        let dir = TempDir::new().unwrap();
        let file_a = dir.path().join("a.txt");
        let file_b = dir.path().join("b.txt");
        fs::write(&file_a, "a").unwrap();

        let conf = PathConfig {
            conditions: vec![
                PathCondition::PathExists(file_a.to_str().unwrap().to_owned()),
                PathCondition::PathModified(file_b.to_str().unwrap().to_owned()),
                PathCondition::DirectoryNotEmpty(dir.path().to_str().unwrap().to_owned()),
            ],
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: Duration::from_secs(60),
            trigger_limit_burst: 10,
            unit: "test.service".to_owned(),
        };

        let mut ino = PathInotify::new().expect("inotify available");
        ino.ensure_watches("test.path", &conf);

        // Should have watches for all three conditions.
        assert_eq!(ino.active_conditions.len(), 3);
        assert!(
            ino.active_conditions
                .contains_key(&("test.path".to_owned(), 0))
        );
        assert!(
            ino.active_conditions
                .contains_key(&("test.path".to_owned(), 1))
        );
        assert!(
            ino.active_conditions
                .contains_key(&("test.path".to_owned(), 2))
        );
    }

    // -- Constants tests -----------------------------------------------------

    #[test]
    fn test_poll_fallback_interval() {
        assert!(POLL_FALLBACK_INTERVAL.as_secs() >= 5);
    }

    #[test]
    fn test_inotify_poll_timeout() {
        assert!(INOTIFY_POLL_TIMEOUT_MS > 0);
        assert!(INOTIFY_POLL_TIMEOUT_MS <= 2000);
    }

    #[test]
    fn test_startup_delay() {
        assert!(STARTUP_DELAY.as_secs() >= 1);
    }

    // -- Watch flag tests ----------------------------------------------------

    #[test]
    fn test_parent_watch_flags_include_create() {
        assert!(PARENT_WATCH_FLAGS.contains(AddWatchFlags::IN_CREATE));
        assert!(PARENT_WATCH_FLAGS.contains(AddWatchFlags::IN_MOVED_TO));
    }

    #[test]
    fn test_path_changed_flags_comprehensive() {
        assert!(PATH_CHANGED_FLAGS.contains(AddWatchFlags::IN_ATTRIB));
        assert!(PATH_CHANGED_FLAGS.contains(AddWatchFlags::IN_CLOSE_WRITE));
        assert!(PATH_CHANGED_FLAGS.contains(AddWatchFlags::IN_CREATE));
        assert!(PATH_CHANGED_FLAGS.contains(AddWatchFlags::IN_DELETE));
        assert!(PATH_CHANGED_FLAGS.contains(AddWatchFlags::IN_MOVED_FROM));
        assert!(PATH_CHANGED_FLAGS.contains(AddWatchFlags::IN_MOVED_TO));
        assert!(PATH_CHANGED_FLAGS.contains(AddWatchFlags::IN_DELETE_SELF));
        assert!(PATH_CHANGED_FLAGS.contains(AddWatchFlags::IN_MOVE_SELF));
    }

    #[test]
    fn test_path_modified_flags_include_close_write() {
        assert!(PATH_MODIFIED_FLAGS.contains(AddWatchFlags::IN_CLOSE_WRITE));
        assert!(PATH_MODIFIED_FLAGS.contains(AddWatchFlags::IN_DELETE_SELF));
    }

    #[test]
    fn test_dir_not_empty_flags() {
        assert!(DIR_NOT_EMPTY_FLAGS.contains(AddWatchFlags::IN_CREATE));
        assert!(DIR_NOT_EMPTY_FLAGS.contains(AddWatchFlags::IN_MOVED_TO));
    }
}
