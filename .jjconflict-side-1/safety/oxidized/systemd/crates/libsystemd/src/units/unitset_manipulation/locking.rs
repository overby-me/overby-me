use crate::runtime_info::UnitTable;
use crate::units::{UnitId, UnitStatus};
use std::collections::HashMap;

use log::warn;
use std::sync::{RwLockReadGuard, RwLockWriteGuard};

/// This is a helper function to lock a set of units either read or write
///
/// This is needed to be able to lock a unit exclusively and all related units shared so we can uphold
/// invariants while running an operation on the exclusively locked unit.
///
/// Units that are not found in the unit table (e.g. optional dependencies that were never loaded,
/// or units removed during cycle-breaking) are silently skipped. Poisoned locks are recovered
/// from with a warning, since panicking here (especially as PID 1) would be catastrophic.
#[must_use]
pub fn acquire_locks(
    mut lock_exclusive: Vec<UnitId>,
    mut lock_shared: Vec<UnitId>,
    unit_table: &UnitTable,
) -> (
    HashMap<UnitId, RwLockWriteGuard<'_, UnitStatus>>,
    HashMap<UnitId, RwLockReadGuard<'_, UnitStatus>>,
) {
    let mut exclusive = HashMap::new();
    let mut shared = HashMap::new();

    lock_exclusive.sort();
    lock_exclusive.dedup();
    lock_shared.sort();
    lock_shared.dedup();

    // Filter out any IDs that don't exist in the unit table before we start locking.
    // This prevents panics when a unit references a dependency that was never loaded
    // (e.g. optional units, units removed during pruning/cycle-breaking).
    lock_exclusive.retain(|id| {
        let exists = unit_table.contains_key(id);
        if !exists {
            warn!(
                "Unit {:?} requested for exclusive lock but not found in unit table. Skipping.",
                id
            );
        }
        exists
    });
    lock_shared.retain(|id| {
        let exists = unit_table.contains_key(id);
        if !exists {
            warn!(
                "Unit {:?} requested for shared lock but not found in unit table. Skipping.",
                id
            );
        }
        exists
    });

    assert!(
        !lock_exclusive.iter().any(|id| lock_shared.contains(id)),
        "Cant lock shared and exclusive at the same time!"
    );

    // Lock in a consistent order (by descending UnitId) to prevent deadlocks.
    // We interleave exclusive and shared locks so that we always lock in the
    // same global order regardless of the lock type.
    while !lock_shared.is_empty() && !lock_exclusive.is_empty() {
        if lock_exclusive.last().unwrap() < lock_shared.last().unwrap() {
            let id = lock_exclusive.remove(lock_exclusive.len() - 1);
            if let Some(unit) = unit_table.get(&id) {
                let locked_status = match unit.common.status.write() {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        warn!(
                            "Write lock poisoned for unit {:?}. Recovering from poison.",
                            id
                        );
                        poisoned.into_inner()
                    }
                };
                exclusive.insert(id, locked_status);
            }
        } else {
            let id = lock_shared.remove(lock_shared.len() - 1);
            if let Some(unit) = unit_table.get(&id) {
                let locked_status = match unit.common.status.read() {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        warn!(
                            "Read lock poisoned for unit {:?}. Recovering from poison.",
                            id
                        );
                        poisoned.into_inner()
                    }
                };
                shared.insert(id, locked_status);
            }
        }
    }

    lock_shared.reverse();
    lock_exclusive.reverse();
    for id in lock_shared {
        if let Some(unit) = unit_table.get(&id) {
            let locked_status = match unit.common.status.read() {
                Ok(guard) => guard,
                Err(poisoned) => {
                    warn!(
                        "Read lock poisoned for unit {:?}. Recovering from poison.",
                        id
                    );
                    poisoned.into_inner()
                }
            };
            shared.insert(id, locked_status);
        }
    }
    for id in lock_exclusive {
        if let Some(unit) = unit_table.get(&id) {
            let locked_status = match unit.common.status.write() {
                Ok(guard) => guard,
                Err(poisoned) => {
                    warn!(
                        "Write lock poisoned for unit {:?}. Recovering from poison.",
                        id
                    );
                    poisoned.into_inner()
                }
            };
            exclusive.insert(id, locked_status);
        }
    }

    (exclusive, shared)
}
