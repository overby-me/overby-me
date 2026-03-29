use std::cell::RefCell;
use std::collections::HashSet;

use log::trace;

use crate::runtime_info::RuntimeInfo;
use crate::units::{UnitId, UnitOperationError, UnitOperationErrorReason};

thread_local! {
    /// Track units currently being deactivated to prevent infinite recursion.
    /// Needed because PropagatesStopTo= can create cycles with required_by
    /// (e.g. A Requires=B + PropagatesStopTo=B → stopping A propagates to B,
    /// B.required_by includes A → would recurse back to A without this guard).
    static DEACTIVATING: RefCell<HashSet<UnitId>> = RefCell::new(HashSet::new());
}

pub fn deactivate_unit_recursive(
    id_to_kill: &UnitId,
    run_info: &RuntimeInfo,
) -> Result<(), UnitOperationError> {
    // Cycle guard: skip if this unit is already being deactivated up the stack.
    let already_in_progress = DEACTIVATING.with(|set| set.borrow().contains(id_to_kill));
    if already_in_progress {
        return Ok(());
    }

    let Some(unit) = run_info.unit_table.get(id_to_kill) else {
        // If this occurs, there is a flaw in the handling of dependencies
        // IDs should be purged globally when units get removed
        return Err(UnitOperationError {
            reason: UnitOperationErrorReason::GenericStartError(
                "Tried to activate a unit that can not be found".into(),
            ),
            unit_name: id_to_kill.name.clone(),
            unit_id: id_to_kill.clone(),
        });
    };

    // Mark this unit as being deactivated before recursing into dependencies.
    DEACTIVATING.with(|set| set.borrow_mut().insert(id_to_kill.clone()));

    let result = (|| {
        deactivate_units_recursive(&unit.common.dependencies.required_by, run_info)?;
        // PartOf= stop propagation: units that declared PartOf= this unit
        // should also be stopped when this unit is stopped.
        deactivate_units_recursive(&unit.common.dependencies.part_of_by, run_info)?;
        // BindsTo= stop propagation: units that declared BindsTo= this unit
        // should also be stopped when this unit stops (even cleanly).
        deactivate_units_recursive(&unit.common.dependencies.bound_by, run_info)?;

        // Stop self BEFORE propagating to PropagatesStopTo= targets.
        // PropagatesStopTo targets may have Requires= back to this unit,
        // so this unit must be Stopped before those targets' required_by
        // check in state_transition_stopping() can pass.
        deactivate_unit(id_to_kill, run_info)?;

        // PropagatesStopTo= forward stop propagation: when this unit stops,
        // also stop the listed units. Errors are logged but don't fail the
        // overall deactivation (self is already stopped).
        for stop_id in &unit.common.dependencies.propagates_stop_to {
            if let Err(e) = deactivate_unit_recursive(stop_id, run_info) {
                log::warn!(
                    "Failed to propagate stop from {} to {}: {e}",
                    id_to_kill.name,
                    stop_id.name
                );
            }
        }
        Ok(())
    })();

    // Remove from the deactivating set when done (whether success or error).
    DEACTIVATING.with(|set| set.borrow_mut().remove(id_to_kill));

    result
}

pub fn deactivate_unit(
    id_to_kill: &UnitId,
    run_info: &RuntimeInfo,
) -> Result<(), UnitOperationError> {
    let Some(unit) = run_info.unit_table.get(id_to_kill) else {
        // If this occurs, there is a flaw in the handling of dependencies
        // IDs should be purged globally when units get removed
        return Err(UnitOperationError {
            reason: UnitOperationErrorReason::GenericStartError(
                "Tried to activate a unit that can not be found".into(),
            ),
            unit_name: id_to_kill.name.clone(),
            unit_id: id_to_kill.clone(),
        });
    };
    unit.deactivate(run_info)?;

    Ok(())
}

pub fn deactivate_units_recursive(
    ids_to_kill: &[UnitId],
    run_info: &RuntimeInfo,
) -> Result<(), UnitOperationError> {
    for id in ids_to_kill {
        deactivate_unit_recursive(id, run_info)?;
    }
    Ok(())
}

pub fn deactivate_units(
    ids_to_kill: &[UnitId],
    run_info: &RuntimeInfo,
) -> Result<(), UnitOperationError> {
    for id in ids_to_kill {
        deactivate_unit(id, run_info)?;
    }
    Ok(())
}

pub fn reactivate_unit(
    id_to_restart: UnitId,
    run_info: &RuntimeInfo,
) -> std::result::Result<(), UnitOperationError> {
    trace!("Reactivation of unit: {id_to_restart:?}. Deactivate");
    let Some(unit) = run_info.unit_table.get(&id_to_restart) else {
        // If this occurs, there is a flaw in the handling of dependencies
        // IDs should be purged globally when units get removed
        return Err(UnitOperationError {
            reason: UnitOperationErrorReason::GenericStartError(
                "Tried to activate a unit that can not be found".into(),
            ),
            unit_name: id_to_restart.name.clone(),
            unit_id: id_to_restart,
        });
    };
    unit.reactivate(run_info, crate::units::ActivationSource::Regular)
}
