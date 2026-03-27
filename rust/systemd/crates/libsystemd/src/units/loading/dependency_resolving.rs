use log::trace;
use log::warn;

use crate::runtime_info::UnitTable;
use crate::units::{Dependencies, ServiceConfig, SocketConfig, Specific, Unit, UnitId, UnitIdKind};

use std::collections::HashMap;
use std::convert::TryInto;

/// Takes a set of units and prunes those that are not needed to reach the specified target unit.
pub fn prune_units(
    target_unit_name: &str,
    unit_table: &mut HashMap<UnitId, Unit>,
) -> Result<(), String> {
    let startunit = unit_table.values().fold(None, |mut result, unit| {
        if unit.id.name == target_unit_name
            || unit
                .common
                .unit
                .aliases
                .iter()
                .any(|a| a == target_unit_name)
        {
            result = Some(unit.id.clone());
        }
        result
    });
    let Some(startunit_id) = startunit else {
        return Err(format!("Target unit {target_unit_name} not found"));
    };
    // This vec will record the unit ids that will be kept
    let mut ids_to_keep = vec![startunit_id];
    crate::units::collect_unit_start_subgraph(&mut ids_to_keep, unit_table);

    // Also keep services that are socket-activation targets of surviving sockets.
    // These services are not in the initial start subgraph (they start on-demand
    // when a connection arrives on the socket), but they must remain in the unit
    // table so the socket activation handler can find and start them.
    let mut socket_activation_services = Vec::new();
    for id in &ids_to_keep {
        if let Some(unit) = unit_table.get(id)
            && let Specific::Socket(sock) = &unit.specific
        {
            for srvc_id in &sock.conf.services {
                if !ids_to_keep.contains(srvc_id) {
                    trace!(
                        "Keeping socket-activation target {} (needed by socket {})",
                        srvc_id.name, id.name
                    );
                    socket_activation_services.push(srvc_id.clone());
                }
            }
        }
    }
    ids_to_keep.extend(socket_activation_services);

    // Remove all units that have been deemed unnecessary
    let mut ids_to_remove = Vec::new();
    for id in unit_table.keys() {
        if !ids_to_keep.contains(id) {
            ids_to_remove.push(id.clone());
        }
    }
    for id in &ids_to_remove {
        let unit = unit_table.remove(id).unwrap();
        trace!("Pruning unit: {}", unit.id.name);
    }

    // Cleanup all removed IDs
    for unit in unit_table.values_mut() {
        match &mut unit.specific {
            Specific::Service(specific) => {
                specific.conf.sockets = specific
                    .conf
                    .sockets
                    .iter()
                    .filter(|id| ids_to_keep.contains(id))
                    .cloned()
                    .collect();
            }
            Specific::Socket(specific) => {
                specific.conf.services = specific
                    .conf
                    .services
                    .iter()
                    .filter(|id| ids_to_keep.contains(id))
                    .cloned()
                    .collect();
            }
            Specific::Target(_)
            | Specific::Slice(_)
            | Specific::Mount(_)
            | Specific::Swap(_)
            | Specific::Timer(_)
            | Specific::Path(_)
            | Specific::Device(_) => { /**/ }
        }

        unit.common.dependencies.before = unit
            .common
            .dependencies
            .before
            .iter()
            .filter(|id| ids_to_keep.contains(id))
            .cloned()
            .collect();

        unit.common.dependencies.after = unit
            .common
            .dependencies
            .after
            .iter()
            .filter(|id| ids_to_keep.contains(id))
            .cloned()
            .collect();

        unit.common.dependencies.requires = unit
            .common
            .dependencies
            .requires
            .iter()
            .filter(|id| ids_to_keep.contains(id))
            .cloned()
            .collect();

        unit.common.dependencies.wants = unit
            .common
            .dependencies
            .wants
            .iter()
            .filter(|id| ids_to_keep.contains(id))
            .cloned()
            .collect();

        unit.common.dependencies.required_by = unit
            .common
            .dependencies
            .required_by
            .iter()
            .filter(|id| ids_to_keep.contains(id))
            .cloned()
            .collect();

        unit.common.dependencies.wanted_by = unit
            .common
            .dependencies
            .wanted_by
            .iter()
            .filter(|id| ids_to_keep.contains(id))
            .cloned()
            .collect();

        unit.common.dependencies.conflicts = unit
            .common
            .dependencies
            .conflicts
            .iter()
            .filter(|id| ids_to_keep.contains(id))
            .cloned()
            .collect();

        unit.common.dependencies.conflicted_by = unit
            .common
            .dependencies
            .conflicted_by
            .iter()
            .filter(|id| ids_to_keep.contains(id))
            .cloned()
            .collect();

        unit.dedup_dependencies();
    }
    Ok(())
}

/// make edges between units visible on bot sides: required <-> `required_by`  after <-> before
///
/// Also adds all implicit dependencies between units (currently only a subset of the ones defined
/// by systemd)
pub fn fill_dependencies(units: &mut HashMap<UnitId, Unit>) -> Result<(), String> {
    let mut required_by = Vec::new();
    let mut wanted_by: Vec<(UnitId, UnitId)> = Vec::new();
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut conflicts = Vec::new();
    let mut part_of_by: Vec<(UnitId, UnitId)> = Vec::new();

    for unit in (*units).values_mut() {
        trace!("Fill deps for unit: {:?}", unit.id);
        let conf = &mut unit.common.dependencies;
        for id in &conf.wants {
            wanted_by.push((id.clone(), unit.id.clone()));
        }
        for id in &conf.requires {
            required_by.push((id.clone(), unit.id.clone()));
        }
        for id in &conf.conflicts {
            conflicts.push((unit.id.clone(), id.clone()));
        }
        for id in &conf.conflicted_by {
            conflicts.push((id.clone(), unit.id.clone()));
        }
        for id in &conf.before {
            after.push((unit.id.clone(), id.clone()));
        }
        for id in &conf.after {
            before.push((unit.id.clone(), id.clone()));
        }
        for id in &conf.wanted_by {
            wanted_by.push((unit.id.clone(), id.clone()));
        }
        for id in &conf.required_by {
            required_by.push((unit.id.clone(), id.clone()));
        }
        // PartOf=B on unit A means: when B stops, A stops too.
        // Collect (target, dependent) pairs so we can fill part_of_by on the target.
        for id in &conf.part_of {
            part_of_by.push((id.clone(), unit.id.clone()));
        }
    }

    for (wanted, wanting) in wanted_by {
        trace!("{wanting:?} wants {wanted:?}");
        if let Some(unit) = units.get_mut(&wanting) {
            unit.common.dependencies.wants.push(wanted.clone());
        } else {
            trace!("Dependency {wanting:?} wants {wanted:?}, but {wanting:?} not found");
        }
        if let Some(unit) = units.get_mut(&wanted) {
            unit.common.dependencies.wanted_by.push(wanting);
        } else {
            trace!("Dependency {wanted:?} wanted by {wanting:?}, but {wanted:?} not found");
        }
    }

    for (required, requiring) in required_by {
        if let Some(unit) = units.get_mut(&requiring) {
            unit.common.dependencies.requires.push(required.clone());
        } else {
            trace!("Dependency {requiring:?} requires {required:?}, but {requiring:?} not found");
        }
        if let Some(unit) = units.get_mut(&required) {
            unit.common.dependencies.required_by.push(requiring);
        } else {
            trace!("Dependency {required:?} required by {requiring:?}, but {required:?} not found");
        }
    }

    for (before, after) in before {
        if let Some(unit) = units.get_mut(&after) {
            unit.common.dependencies.before.push(before);
        } else {
            trace!("Dependency {before:?} before {after:?}, but {after:?} not found");
        }
    }
    for (after, before) in after {
        if let Some(unit) = units.get_mut(&before) {
            unit.common.dependencies.after.push(after);
        } else {
            trace!("Dependency {after:?} after {before:?}, but {before:?} not found");
        }
    }

    for (conflicting, conflicted) in conflicts {
        if let Some(unit) = units.get_mut(&conflicting) {
            unit.common.dependencies.conflicts.push(conflicted.clone());
        } else {
            trace!(
                "Dependency {conflicting:?} conflicts with {conflicted:?}, but {conflicting:?} not found"
            );
        }
        if let Some(unit) = units.get_mut(&conflicted) {
            unit.common.dependencies.conflicted_by.push(conflicting);
        } else {
            trace!(
                "Dependency {conflicted:?} conflicted by {conflicting:?}, but {conflicted:?} not found"
            );
        }
    }

    // PartOf= : unit A has PartOf=B, so B gets part_of_by=A
    for (target, dependent) in part_of_by {
        if let Some(unit) = units.get_mut(&target) {
            unit.common.dependencies.part_of_by.push(dependent);
        } else {
            trace!("Dependency {dependent:?} is PartOf {target:?}, but {target:?} not found");
        }
    }

    add_all_implicit_relations(units)?;

    // Remove activation dependency references to unit IDs that don't exist in the
    // unit table. Activation deps (Wants, Requires, BindsTo, etc.) would cause
    // failures if they reference non-existent units, so drop them. Ordering deps
    // (After, Before) are kept even for non-existent units — they become no-op
    // constraints, matching real systemd behavior.
    let existing_ids: std::collections::HashSet<UnitId> = units.keys().cloned().collect();
    for unit in units.values_mut() {
        let deps = &mut unit.common.dependencies;
        deps.wants.retain(|id| existing_ids.contains(id));
        deps.wanted_by.retain(|id| existing_ids.contains(id));
        deps.requires.retain(|id| existing_ids.contains(id));
        deps.required_by.retain(|id| existing_ids.contains(id));
        deps.conflicts.retain(|id| existing_ids.contains(id));
        deps.conflicted_by.retain(|id| existing_ids.contains(id));
        deps.part_of.retain(|id| existing_ids.contains(id));
        deps.part_of_by.retain(|id| existing_ids.contains(id));
        deps.binds_to.retain(|id| existing_ids.contains(id));
        deps.bound_by.retain(|id| existing_ids.contains(id));
        // Keep refs_by_name un-pruned: it preserves the original dependency
        // references from the unit file so that on-demand loading (e.g.
        // `systemctl restart` of a target whose Wants= service didn't exist
        // at daemon-reload time) can discover and load those units later.
    }

    for srvc in units.values_mut() {
        srvc.dedup_dependencies();
    }

    Ok(())
}

/// Function to apply all implicit relations to the units in the table
///
/// This is currently only a subset of all implicit relations systemd applies
fn add_all_implicit_relations(units: &mut UnitTable) -> Result<(), String> {
    add_default_dependency_relations(units);
    add_socket_target_relations(units);
    apply_sockets_to_services(units)?;
    add_slice_dependencies(units);
    Ok(())
}

/// When a unit specifies `Slice=foo.slice`, it implicitly gets
/// `Requires=foo.slice` and `After=foo.slice` (matching systemd's behavior).
fn add_slice_dependencies(units: &mut UnitTable) {
    use crate::units::{Specific, UnitId};

    // Collect (unit_id, slice_id) pairs
    let mut slice_deps: Vec<(UnitId, UnitId)> = Vec::new();
    for unit in units.values() {
        let slice_name = match &unit.specific {
            Specific::Service(s) => s.conf.slice.as_deref(),
            _ => None,
        };
        if let Some(slice_name) = slice_name
            && let Ok(slice_id) = <&str as TryInto<UnitId>>::try_into(slice_name)
            && units.contains_key(&slice_id)
        {
            slice_deps.push((unit.id.clone(), slice_id));
        }
    }

    for (unit_id, slice_id) in slice_deps {
        if let Some(unit) = units.get_mut(&unit_id) {
            let deps = &mut unit.common.dependencies;
            if !deps.requires.contains(&slice_id) {
                deps.requires.push(slice_id.clone());
            }
            if !deps.after.contains(&slice_id) {
                deps.after.push(slice_id.clone());
            }
        }
    }
}

/// Applies the implicit default dependencies for units that have `DefaultDependencies=yes` (the default).
///
/// Following systemd's behavior:
/// - For all unit types: `Conflicts=shutdown.target` and `Before=shutdown.target`
/// - For services and sockets: additionally `Requires=sysinit.target` and `After=sysinit.target basic.target`
///
/// These are only applied if the respective targets exist in the unit table.
/// `shutdown.target` itself is excluded from getting default dependencies to avoid circular deps.
fn add_default_dependency_relations(units: &mut UnitTable) {
    let shutdown_id: UnitId = "shutdown.target".try_into().unwrap();
    let sysinit_id: UnitId = "sysinit.target".try_into().unwrap();
    let basic_id: UnitId = "basic.target".try_into().unwrap();

    let has_shutdown = units.contains_key(&shutdown_id);
    let has_sysinit = units.contains_key(&sysinit_id);
    let has_basic = units.contains_key(&basic_id);

    if !has_shutdown && !has_sysinit && !has_basic {
        return;
    }

    let mut add_after_to_sysinit = Vec::new();
    let mut add_after_to_basic = Vec::new();
    let mut add_after_to_shutdown = Vec::new();

    for unit in units.values_mut() {
        if !unit.common.unit.default_dependencies {
            continue;
        }
        // Don't add default deps to shutdown.target itself
        if unit.id == shutdown_id {
            continue;
        }

        // All units with default deps get Conflicts= and Before= on shutdown.target
        if has_shutdown {
            unit.common.dependencies.conflicts.push(shutdown_id.clone());
            unit.common.dependencies.before.push(shutdown_id.clone());
            add_after_to_shutdown.push(unit.id.clone());
        }

        // Services and sockets get Requires= and After= on sysinit.target.
        // Services additionally get After= on basic.target.
        //
        // In real systemd, sockets get the same sysinit.target deps as
        // services. There is no circular dependency because sysinit.target
        // orders before sockets.target, not after it.
        match unit.id.kind {
            UnitIdKind::Service => {
                if has_sysinit {
                    unit.common.dependencies.requires.push(sysinit_id.clone());
                    unit.common.dependencies.after.push(sysinit_id.clone());
                    add_after_to_sysinit.push(unit.id.clone());
                }
                if has_basic {
                    unit.common.dependencies.after.push(basic_id.clone());
                    add_after_to_basic.push(unit.id.clone());
                }
            }
            UnitIdKind::Socket => {
                // Sockets get Requires/After sysinit.target (same as services)
                // but NOT After basic.target.
                if has_sysinit {
                    unit.common.dependencies.requires.push(sysinit_id.clone());
                    unit.common.dependencies.after.push(sysinit_id.clone());
                    add_after_to_sysinit.push(unit.id.clone());
                }
            }
            UnitIdKind::Target
            | UnitIdKind::Slice
            | UnitIdKind::Mount
            | UnitIdKind::Swap
            | UnitIdKind::Device
            | UnitIdKind::Timer
            | UnitIdKind::Path => {
                // Targets, slices, mounts, swaps, devices, timers, and paths only get the
                // shutdown.target conflict/before (already added above).
            }
        }

        unit.common.dependencies.dedup();
    }

    // Add the reverse relations to the targets
    if has_shutdown {
        let shutdown = units.get_mut(&shutdown_id).unwrap();
        for id in &add_after_to_shutdown {
            shutdown.common.dependencies.conflicted_by.push(id.clone());
            shutdown.common.dependencies.after.push(id.clone());
        }
        shutdown.common.dependencies.dedup();
    }
    if has_sysinit {
        let sysinit = units.get_mut(&sysinit_id).unwrap();
        for id in &add_after_to_sysinit {
            sysinit.common.dependencies.required_by.push(id.clone());
            sysinit.common.dependencies.before.push(id.clone());
        }
        sysinit.common.dependencies.dedup();
    }
    if has_basic {
        let basic = units.get_mut(&basic_id).unwrap();
        for id in &add_after_to_basic {
            basic.common.dependencies.before.push(id.clone());
        }
        basic.common.dependencies.dedup();
    }
}

/// There is an implicit *.socket before sockets.target relation
///
/// This is only applied if this target exists. I would like to
/// leave well known units as optional as possible but this is needed
/// for compatibility
fn add_socket_target_relations(units: &mut UnitTable) {
    let target_id: UnitId = "sockets.target".try_into().unwrap();
    let mut socket_ids = Vec::new();
    if units.contains_key(&target_id) {
        for unit in units.values_mut() {
            if UnitIdKind::Socket == unit.id.kind {
                // Add to socket
                unit.common.dependencies.before.push(target_id.clone());
                unit.common.dependencies.dedup();
                // Remember socket id to add to the target
                socket_ids.push(unit.id.clone());
            }
        }
        let target = units.get_mut(&target_id).unwrap();
        target.common.dependencies.after.extend(socket_ids);
        target.common.dependencies.dedup();
    }
}

/// Set up socket→service relations.
///
/// When the service explicitly lists the socket (via `Sockets=`) or names match,
/// we add both ordering (Before/After) AND hard dependency (Requires/RequiredBy).
///
/// When only the socket references the service (via `Service=`), we add ordering
/// only — the service does NOT get a hard Requires on the socket.  This matches
/// real systemd behavior where optional/conditional sockets (e.g. the audit
/// socket) don't block the service from starting.
fn add_sock_srvc_relations(
    srvc_id: UnitId,
    srvc_install: &mut Dependencies,
    srvc_conf: &mut ServiceConfig,
    sock_id: UnitId,
    sock_install: &mut Dependencies,
    sock_conf: &mut SocketConfig,
    strong: bool,
) {
    // Always add ordering: service After socket, socket Before service
    srvc_install.after.push(sock_id.clone());
    sock_install.before.push(srvc_id.clone());

    // Only add hard dependency when the service explicitly lists the socket
    // (name match or Sockets= directive).  When only the socket's Service=
    // points at the service, the socket is optional and should not block it.
    if strong {
        srvc_install.requires.push(sock_id.clone());
        sock_install.required_by.push(srvc_id.clone());
    }

    sock_install.dedup();
    srvc_install.dedup();

    if !srvc_conf.sockets.contains(&sock_id) {
        srvc_conf.sockets.push(sock_id);
    }
    if !sock_conf.services.contains(&srvc_id) {
        sock_conf.services.push(srvc_id);
    }
}

/// This takes a set of services and sockets and matches them both by their name and their
/// respective explicit settings.  It adds appropriate before/after and (where appropriate)
/// requires/required_by relations.
///
/// Matching rules (inclusive OR — all that apply are used):
/// 1. Names match (e.g. `foo.socket` ↔ `foo.service`) → strong (Requires + ordering)
/// 2. Service lists socket in `Sockets=` → strong
/// 3. Socket lists service in `Service=` → weak (ordering only, no Requires)
///
/// `add_sock_srvc_relations` handles dedup, so calling it when some relations
/// already exist from unit-file parsing is safe.
fn apply_sockets_to_services(unit_table: &mut UnitTable) -> Result<(), String> {
    let mut service_ids = Vec::new();
    let mut socket_ids = Vec::new();
    for id in unit_table.keys() {
        match id.kind {
            UnitIdKind::Timer => {
                // Timer units are simple — no special cross-references needed.
            }
            UnitIdKind::Service => {
                service_ids.push(id.clone());
            }
            UnitIdKind::Socket => {
                socket_ids.push(id.clone());
            }
            UnitIdKind::Target
            | UnitIdKind::Slice
            | UnitIdKind::Mount
            | UnitIdKind::Swap
            | UnitIdKind::Device
            | UnitIdKind::Path => {
                // ignore targets, slices, mounts, devices, and paths here
            }
        }
    }

    for sock_unit in &socket_ids {
        let mut sock_unit = unit_table.remove(sock_unit).unwrap();
        let mut counter = 0;

        if let Specific::Socket(sock) = &mut sock_unit.specific {
            trace!("Searching services for socket: {}", sock_unit.id.name);
            for srvc_unit in &service_ids {
                let mut srvc_unit = unit_table.remove(srvc_unit).unwrap();

                let srvc = &mut srvc_unit.specific;
                if let Specific::Service(srvc) = srvc {
                    let names_match =
                        srvc_unit.id.name_without_suffix() == sock_unit.id.name_without_suffix();
                    let srvc_has_sock = srvc.conf.sockets.contains(&sock_unit.id);
                    let sock_has_srvc = sock.conf.services.contains(&srvc_unit.id);

                    if names_match || srvc_has_sock || sock_has_srvc {
                        // Strong relation when the service explicitly owns the
                        // socket (name match or Sockets= directive).  Weak
                        // (ordering-only) when only the socket's Service= points
                        // at the service — the socket may be optional/conditional.
                        let strong = names_match || srvc_has_sock;
                        trace!(
                            "add socket: {} to service: {} (names_match={}, srvc_has_sock={}, sock_has_srvc={}, strong={})",
                            sock_unit.id.name,
                            srvc_unit.id.name,
                            names_match,
                            srvc_has_sock,
                            sock_has_srvc,
                            strong,
                        );

                        add_sock_srvc_relations(
                            srvc_unit.id.clone(),
                            &mut srvc_unit.common.dependencies,
                            &mut srvc.conf,
                            sock_unit.id.clone(),
                            &mut sock_unit.common.dependencies,
                            &mut sock.conf,
                            strong,
                        );
                        counter += 1;
                    }
                }
                unit_table.insert(srvc_unit.id.clone(), srvc_unit);
            }
        }
        let sock_name = sock_unit.id.name.clone();
        unit_table.insert(sock_unit.id.clone(), sock_unit);
        if counter > 1 {
            // Multiple services matched is only an error for strong matches.
            // For now just warn, since conditional sockets may match weakly.
            warn!("Added socket: {sock_name} to {counter} services (expected at most one)");
        }
        if counter == 0 {
            trace!("Added socket: {sock_name} to no service");
        }
    }

    Ok(())
}
