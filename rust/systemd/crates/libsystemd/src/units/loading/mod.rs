mod dependency_resolving;
pub(crate) mod directory_deps;
pub use dependency_resolving::*;
use log::{info, trace, warn};

use crate::runtime_info::UnitTable;
use crate::units::{ParsingError, Specific, Unit, UnitId, get_file_list, parse_file};

pub use directory_deps::collect_dir_deps_for_unit;
use directory_deps::{
    DirectoryDependency, apply_directory_dependencies, apply_dropins, collect_dep_dir_entries,
    collect_dropin_entries, create_implicit_slices_from_dropins, generate_fstab_mount_units,
    generate_getty_units, insert_parsed_unit, instantiate_template_units, is_template_unit,
    is_unit_file, parse_dep_dir_name, parse_dropin_dir_name, resolve_specifiers,
    resolve_symlink_aliases,
};

use std::collections::HashMap;

use std::path::PathBuf;

#[derive(Debug)]
pub enum LoadingError {
    Parsing(ParsingError),
    Dependency(DependencyError),
}

#[derive(Debug)]
pub struct DependencyError {
    msg: String,
}

impl std::convert::From<String> for DependencyError {
    fn from(s: String) -> Self {
        Self { msg: s }
    }
}

impl std::fmt::Display for DependencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Dependency resolving error: {}", self.msg)
    }
}

impl std::convert::From<DependencyError> for LoadingError {
    fn from(s: DependencyError) -> Self {
        Self::Dependency(s)
    }
}

impl std::convert::From<ParsingError> for LoadingError {
    fn from(s: ParsingError) -> Self {
        Self::Parsing(s)
    }
}

pub fn load_all_units(
    paths: &[PathBuf],
    target_unit: &str,
) -> Result<HashMap<UnitId, Unit>, LoadingError> {
    load_all_units_inner(paths, target_unit, true)
}

/// Like `load_all_units` but skips the pruning step, returning ALL parsed
/// units regardless of whether they are reachable from the boot target.
/// Used by daemon-reload to discover newly added standalone units.
pub fn load_all_units_no_prune(
    paths: &[PathBuf],
    target_unit: &str,
) -> Result<HashMap<UnitId, Unit>, LoadingError> {
    load_all_units_inner(paths, target_unit, false)
}

fn load_all_units_inner(
    paths: &[PathBuf],
    target_unit: &str,
    prune: bool,
) -> Result<HashMap<UnitId, Unit>, LoadingError> {
    let mut service_unit_table = HashMap::new();
    let mut socket_unit_table = HashMap::new();
    let mut target_unit_table = HashMap::new();
    let mut slice_unit_table = HashMap::new();

    // Collect directory dependencies (.wants/ and .requires/) across all unit dirs
    let mut dir_deps: Vec<DirectoryDependency> = Vec::new();

    // Collect drop-in overrides: unit_name -> Vec<(filename, content)>
    let mut dropins: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for (priority, path) in paths.iter().enumerate() {
        parse_all_units(
            &mut service_unit_table,
            &mut socket_unit_table,
            &mut target_unit_table,
            &mut slice_unit_table,
            path,
            priority,
            &mut dir_deps,
            &mut dropins,
        )?;
    }

    // Create implicit slice units for any slice drop-in directories that don't
    // have a corresponding unit file on disk (systemd creates slices on-demand).
    create_implicit_slices_from_dropins(&mut slice_unit_table, &dropins, paths);

    // Apply drop-in overrides to loaded units by re-parsing with merged content
    apply_dropins(&mut service_unit_table, &dropins, paths);
    apply_dropins(&mut socket_unit_table, &dropins, paths);
    apply_dropins(&mut target_unit_table, &dropins, paths);
    apply_dropins(&mut slice_unit_table, &dropins, paths);

    let mut unit_table = std::collections::HashMap::new();
    unit_table.extend(service_unit_table);
    unit_table.extend(socket_unit_table);
    unit_table.extend(target_unit_table);
    unit_table.extend(slice_unit_table);

    // Apply directory-based dependencies (.wants/ and .requires/)
    apply_directory_dependencies(&mut unit_table, &dir_deps);

    // Log directory deps for debugging
    for dep in &dir_deps {
        trace!(
            "Dir dep: {} {} {}",
            dep.parent_unit,
            if dep.is_requires { "Requires" } else { "Wants" },
            dep.child_unit
        );
    }

    // Instantiate template units referenced by directory dependencies
    instantiate_template_units(&mut unit_table, &dir_deps, paths, &dropins);

    // Generate mount units from /etc/fstab (replaces systemd-fstab-generator)
    generate_fstab_mount_units(&mut unit_table);

    // Generate getty instances from /proc/cmdline (replaces systemd-getty-generator)
    generate_getty_units(&mut unit_table, paths, &dropins);

    // Resolve symlink aliases (e.g., default.target -> multi-user.target)
    // so that .wants/ directories for the real name also apply to the alias.
    resolve_symlink_aliases(&mut unit_table, &dir_deps, paths);

    // Log multi-user.target deps after alias resolution (info-level for boot debugging)
    for name in &["default.target", "multi-user.target"] {
        if let Some(unit) = unit_table
            .values()
            .find(|u| u.id.name == *name || u.common.unit.aliases.iter().any(|a| a == *name))
        {
            let wants: Vec<&str> = unit
                .common
                .dependencies
                .wants
                .iter()
                .map(|d| d.name.as_str())
                .collect();
            let requires: Vec<&str> = unit
                .common
                .dependencies
                .requires
                .iter()
                .map(|d| d.name.as_str())
                .collect();
            info!(
                "Post-alias-resolution {}: id={}, aliases={:?}, wants({})={:?}, requires({})={:?}",
                name,
                unit.id.name,
                unit.common.unit.aliases,
                wants.len(),
                wants,
                requires.len(),
                requires
            );
        } else {
            info!("Post-alias-resolution {}: NOT IN TABLE", name);
        }
    }

    // Log getty-related units before pruning
    for id in unit_table.keys() {
        if id.name.contains("getty") || id.name.contains("ttyS") || id.name.contains("autovt") {
            let unit = &unit_table[id];
            let wants: Vec<&str> = unit
                .common
                .dependencies
                .wants
                .iter()
                .map(|d| d.name.as_str())
                .collect();
            let wanted_by: Vec<&str> = unit
                .common
                .dependencies
                .wanted_by
                .iter()
                .map(|d| d.name.as_str())
                .collect();
            let requires: Vec<&str> = unit
                .common
                .dependencies
                .requires
                .iter()
                .map(|d| d.name.as_str())
                .collect();
            trace!(
                "Pre-prune unit: {} | wants={:?} | wanted_by={:?} | requires={:?}",
                id.name, wants, wanted_by, requires
            );
        }
    }
    // Also log default.target and multi-user.target deps
    for name in &["default.target", "multi-user.target"] {
        if let Some(unit) = unit_table.values().find(|u| u.id.name == *name) {
            let wants: Vec<&str> = unit
                .common
                .dependencies
                .wants
                .iter()
                .map(|d| d.name.as_str())
                .collect();
            let requires: Vec<&str> = unit
                .common
                .dependencies
                .requires
                .iter()
                .map(|d| d.name.as_str())
                .collect();
            trace!(
                "Pre-prune {}: wants={:?} requires={:?}",
                name, wants, requires
            );
        } else {
            trace!("Pre-prune {}: NOT IN TABLE", name);
        }
    }

    // Remove template units (e.g., "getty@.service", "modprobe@.service") and
    // units with unresolved specifiers (e.g., "systemd-journald@%i.socket")
    // from the unit table. Templates should never be activated directly — only
    // concrete instances (e.g., "getty@tty1.service") should be started.
    // Units with unresolved specifiers come from template cross-references
    // like `Requires=systemd-journald@%i.socket` in systemd-journald@.service
    // and should not exist as real units.
    let bad_ids: Vec<UnitId> = unit_table
        .keys()
        .filter(|id| is_template_unit(&id.name) || id.name.contains('%'))
        .cloned()
        .collect();
    for id in &bad_ids {
        if is_template_unit(&id.name) {
            trace!("Removing template unit from table: {}", id.name);
        } else {
            info!(
                "Removing unit with unresolved specifier from table: {}",
                id.name
            );
        }
        unit_table.remove(id);
    }

    // List all loaded socket units at WARN level so they're always visible
    let socket_names: Vec<&str> = unit_table
        .keys()
        .filter(|id| id.name.ends_with(".socket"))
        .map(|id| id.name.as_str())
        .collect();
    trace!(
        "Units found before pruning: {} (sockets: {:?})",
        unit_table.len(),
        socket_names
    );

    info!("Units before fill_dependencies: {} total", unit_table.len());

    fill_dependencies(&mut unit_table).map_err(|e| LoadingError::Dependency(e.into()))?;

    if prune {
        info!(
            "Units before prune: {} total, pruning to target={}",
            unit_table.len(),
            target_unit
        );

        prune_units(target_unit, &mut unit_table).unwrap();

        info!("Units after prune: {} total", unit_table.len());
    } else {
        info!(
            "Skipping prune (reload mode): {} units total",
            unit_table.len()
        );
    }

    // Log multi-user.target deps after pruning
    for name in &["default.target", "multi-user.target"] {
        if let Some(unit) = unit_table
            .values()
            .find(|u| u.id.name == *name || u.common.unit.aliases.iter().any(|a| a == *name))
        {
            let wants: Vec<&str> = unit
                .common
                .dependencies
                .wants
                .iter()
                .map(|d| d.name.as_str())
                .collect();
            info!(
                "Post-prune {}: id={}, wants({})={:?}",
                name,
                unit.id.name,
                wants.len(),
                wants
            );
        }
    }

    // List surviving socket units
    let surviving_sockets: Vec<&str> = unit_table
        .keys()
        .filter(|id| id.name.ends_with(".socket"))
        .map(|id| id.name.as_str())
        .collect();
    trace!(
        "Units after pruning: {} (sockets: {:?})",
        unit_table.len(),
        surviving_sockets
    );

    let removed_ids = prune_unused_sockets(&mut unit_table);
    trace!("Finished pruning sockets");

    // Log final socket list
    let final_sockets: Vec<&str> = unit_table
        .keys()
        .filter(|id| id.name.ends_with(".socket"))
        .map(|id| id.name.as_str())
        .collect();
    trace!(
        "Final unit count after socket pruning: {} (sockets: {:?})",
        unit_table.len(),
        final_sockets
    );

    cleanup_removed_ids(&mut unit_table, &removed_ids);

    Ok(unit_table)
}

fn cleanup_removed_ids(
    units: &mut std::collections::HashMap<UnitId, Unit>,
    removed_ids: &Vec<UnitId>,
) {
    for unit in units.values_mut() {
        for id in removed_ids {
            unit.common.dependencies.remove_id(id);
        }
    }
}

fn prune_unused_sockets(sockets: &mut UnitTable) -> Vec<UnitId> {
    // Collect socket IDs that are explicitly Wants= or Requires= by
    // non-socket units (services, targets, etc.).  These sockets must be
    // kept even when they have no associated service — they serve as
    // stand-alone activation points (e.g. dbus.socket for D-Bus activation).
    //
    // We intentionally do NOT include After= or the implicit sockets.target
    // relationship here: After= is ordering-only and doesn't imply the
    // socket is actually needed, and sockets.target automatically gets
    // After= on every socket via add_socket_target_relations.
    let mut explicitly_wanted_sockets: std::collections::HashSet<UnitId> =
        std::collections::HashSet::new();
    for unit in sockets.values() {
        // Skip socket units themselves — we only care about services/targets
        // that explicitly pull in sockets via Wants=/Requires=/BindsTo=.
        if unit.id.kind == crate::units::UnitIdKind::Socket {
            continue;
        }
        let deps = &unit.common.dependencies;
        for id in deps
            .wants
            .iter()
            .chain(deps.requires.iter())
            .chain(deps.binds_to.iter())
        {
            if id.kind == crate::units::UnitIdKind::Socket {
                explicitly_wanted_sockets.insert(id.clone());
            }
        }
    }

    let mut ids_to_remove = Vec::new();
    for unit in sockets.values() {
        if let Specific::Socket(sock) = &unit.specific {
            if sock.conf.services.is_empty() && !explicitly_wanted_sockets.contains(&unit.id) {
                trace!(
                    "Prune socket {} because it has no associated service and no unit explicitly wants it",
                    unit.id.name
                );
                ids_to_remove.push(unit.id.clone());
            } else if sock.conf.services.is_empty() {
                trace!(
                    "Keeping socket {} despite no associated service: explicitly wanted by another unit",
                    unit.id.name
                );
            }
        }
    }
    for id in &ids_to_remove {
        sockets.remove(id);
    }
    ids_to_remove
}

#[allow(clippy::too_many_arguments)]
fn parse_all_units(
    services: &mut std::collections::HashMap<UnitId, Unit>,
    sockets: &mut std::collections::HashMap<UnitId, Unit>,
    targets: &mut std::collections::HashMap<UnitId, Unit>,
    slices: &mut std::collections::HashMap<UnitId, Unit>,
    path: &PathBuf,
    priority: usize,
    dir_deps: &mut Vec<DirectoryDependency>,
    dropins: &mut HashMap<String, Vec<(String, String)>>,
) -> Result<(), ParsingError> {
    let files = get_file_list(path).map_err(|e| ParsingError::new(e, path.clone()))?;
    for entry in files {
        let entry_path = entry.path();
        if entry_path.is_dir() {
            let dir_name = entry.file_name().to_string_lossy().to_string();

            // Handle .wants/ and .requires/ directories
            if let Some((parent_unit, is_requires)) = parse_dep_dir_name(&dir_name) {
                collect_dep_dir_entries(&entry_path, &parent_unit, is_requires, priority, dir_deps);
                continue;
            }

            // Handle .d/ drop-in directories
            if let Some(unit_name) = parse_dropin_dir_name(&dir_name) {
                collect_dropin_entries(&entry_path, &unit_name, dropins);
                continue;
            }

            // Regular subdirectory — recurse
            parse_all_units(
                services,
                sockets,
                targets,
                slices,
                &entry_path,
                priority,
                dir_deps,
                dropins,
            )?;
        } else {
            let filename = entry.file_name().to_string_lossy().to_string();
            if !is_unit_file(&filename) {
                continue;
            }

            let raw = match std::fs::read_to_string(&entry_path) {
                Ok(raw) => raw,
                Err(e) => {
                    warn!("Skipping unit {:?}: could not read file: {}", entry_path, e);
                    continue;
                }
            };

            // Resolve specifiers (%n, %i, %p, etc.) before parsing.
            // For non-template units the instance is empty.
            let raw = resolve_specifiers(&raw, &filename, "");

            let parsed_file = match parse_file(&raw) {
                Ok(pf) => pf,
                Err(e) => {
                    warn!(
                        "Skipping unit {:?}: could not parse file: {:?}",
                        entry_path, e
                    );
                    continue;
                }
            };

            insert_parsed_unit(services, sockets, targets, slices, parsed_file, &entry_path);
        }
    }
    Ok(())
}
