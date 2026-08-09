//! Slice-level concurrency limits (`ConcurrencyHardMax=` / `ConcurrencySoftMax=`).
//!
//! Ports systemd's `slice_get_currently_active` plus the soft/hard "reached"
//! checks (`src/core/slice.c`). A slice's limit counts the units in its subtree
//! (recursively, with sub-slice units themselves counting as one), and is
//! enforced against every ancestor slice up the dash-encoded hierarchy.

use crate::lock_ext::{MutexExt, RwLockExt};
use crate::runtime_info::RuntimeInfo;
use crate::units::from_parsed_config::mangle_slice_name;
use crate::units::id::{UnitId, UnitIdKind};
use crate::units::jobs::{JobKind, JobState};
use crate::units::status::UnitStatus;
use crate::units::unit::{SliceConfig, Specific, Unit};

/// A unit counts against its slice while it is neither never-started nor stopped
/// (starting/started/restarting/stopping all count as "live"), matching
/// `!UNIT_IS_INACTIVE_OR_FAILED(...)`.
fn is_live(status: &UnitStatus) -> bool {
    !matches!(
        status,
        UnitStatus::NeverStarted | UnitStatus::Stopped(..)
    )
}

/// The slice a unit belongs to: a service's `Slice=` (mangled to a full unit
/// name), or a slice unit's parent derived from its dash-encoded name. None for
/// units not tracked in a (non-root) slice.
fn parent_slice_of(unit: &Unit) -> Option<String> {
    if unit.id.kind == UnitIdKind::Slice {
        return slice_parent_name(&unit.id.name);
    }
    match &unit.specific {
        Specific::Service(s) => s.conf.slice.as_deref().map(mangle_slice_name),
        _ => None,
    }
}

/// The parent slice unit name of a dash-encoded slice name, or None for a
/// top-level slice (whose parent is the root slice `-.slice`, which carries no
/// concurrency limits).
fn slice_parent_name(slice_name: &str) -> Option<String> {
    let base = slice_name.strip_suffix(".slice")?;
    if base == "-" {
        return None;
    }
    base.rfind('-').map(|idx| format!("{}.slice", &base[..idx]))
}

/// Look up a slice unit's parsed config by name.
fn slice_config<'a>(ri: &'a RuntimeInfo, slice_name: &str) -> Option<&'a SliceConfig> {
    let id = UnitId {
        kind: UnitIdKind::Slice,
        name: slice_name.to_string(),
    };
    match &ri.unit_table.get(&id)?.specific {
        Specific::Slice(s) => Some(&s.conf),
        _ => None,
    }
}

/// Whether a unit currently has a pending (waiting or running) start job.
fn has_pending_start(ri: &RuntimeInfo, id: &UnitId) -> bool {
    let jobs = ri.jobs.lock_poisoned();
    jobs.job_for_unit(id).is_some_and(|j| {
        j.kind == JobKind::Start && matches!(j.state, JobState::Waiting | JobState::Running)
    })
}

/// Port of `slice_get_currently_active`: count the units in `slice_name`'s
/// subtree (recursively) that are live, plus (when `with_pending`) those with a
/// pending start job. The unit named `ignore` is excluded (it is the one whose
/// start is being evaluated).
pub fn slice_currently_active(
    ri: &RuntimeInfo,
    slice_name: &str,
    ignore: &str,
    with_pending: bool,
) -> u32 {
    let mut n = 0u32;
    for unit in ri.unit_table.values() {
        if unit.id.name == ignore {
            continue;
        }
        if parent_slice_of(unit).as_deref() != Some(slice_name) {
            continue;
        }
        if is_live(&unit.common.status.read_poisoned()) {
            n += 1;
        } else if with_pending && has_pending_start(ri, &unit.id) {
            n += 1;
        }
        if unit.id.kind == UnitIdKind::Slice {
            n += slice_currently_active(ri, &unit.id.name, ignore, with_pending);
        }
    }
    n
}

/// Whether the soft concurrency limit of `slice_name` or any ancestor slice is
/// reached (counting live units only). New starts should queue when true.
pub fn soft_max_reached(ri: &RuntimeInfo, slice_name: &str, ignore: &str) -> bool {
    concurrency_reached(ri, slice_name, ignore, false)
}

/// Whether the hard concurrency limit of `slice_name` or any ancestor slice is
/// reached (counting live units and pending starts). New starts should be refused
/// when true.
pub fn hard_max_reached(ri: &RuntimeInfo, slice_name: &str, ignore: &str) -> bool {
    concurrency_reached(ri, slice_name, ignore, true)
}

fn concurrency_reached(ri: &RuntimeInfo, slice_name: &str, ignore: &str, hard: bool) -> bool {
    let mut cur = Some(slice_name.to_string());
    while let Some(s) = cur {
        let limit = slice_config(ri, &s).and_then(|c| {
            if hard {
                c.concurrency_hard_max
            } else {
                c.concurrency_soft_max
            }
        });
        if let Some(m) = limit
            && slice_currently_active(ri, &s, ignore, hard) >= m
        {
            return true;
        }
        cur = slice_parent_name(&s);
    }
    false
}

/// The mangled slice name a unit is a member of, if it belongs to one, used by
/// callers deciding whether concurrency limits apply to a start.
pub fn unit_slice_name(unit: &Unit) -> Option<String> {
    parent_slice_of(unit)
}

/// The unit's slice, but only when that slice or an ancestor actually configures
/// a concurrency limit. Returns None for the common unlimited case so callers can
/// skip the soft-limit machinery entirely (and avoid touching normal starts).
pub fn concurrency_limited_slice(ri: &RuntimeInfo, unit: &Unit) -> Option<String> {
    let slice = parent_slice_of(unit)?;
    let mut cur = Some(slice.clone());
    while let Some(s) = cur {
        if let Some(c) = slice_config(ri, &s)
            && (c.concurrency_soft_max.is_some() || c.concurrency_hard_max.is_some())
        {
            return Some(slice);
        }
        cur = slice_parent_name(&s);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slice_parent_name() {
        assert_eq!(slice_parent_name("a-b-c.slice").as_deref(), Some("a-b.slice"));
        assert_eq!(slice_parent_name("a-b.slice").as_deref(), Some("a.slice"));
        assert_eq!(slice_parent_name("a.slice"), None); // top-level
        assert_eq!(slice_parent_name("-.slice"), None); // root
        assert_eq!(
            slice_parent_name("concurrency1-concurrency2.slice").as_deref(),
            Some("concurrency1.slice")
        );
    }
}
