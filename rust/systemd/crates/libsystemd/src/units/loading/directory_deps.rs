//! Support for systemd directory-based dependencies (.wants/, .requires/, .upholds/),
//! drop-in override directories (.d/), template unit instantiation with
//! specifier resolution, and a minimal getty generator.

use log::{debug, info, trace, warn};
use std::collections::HashMap;
use std::convert::TryInto;
use std::path::{Path, PathBuf};

use crate::units::{
    Common, CommonState, Dependencies, MountConfig, MountSpecific, MountState, ParsedFile,
    ParsingErrorReason, Specific, SwapConfig, SwapSpecific, SwapState, Unit, UnitConfig, UnitId,
    UnitIdKind, UnitStatus, UnitTimestamps, parse_device, parse_file, parse_mount, parse_path,
    parse_service, parse_slice, parse_socket, parse_swap, parse_target, path_to_mount_unit_name,
    path_to_swap_unit_name,
};
use std::sync::RwLock;

/// The kind of directory-based dependency (.wants/, .requires/, .upholds/).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DepDirKind {
    Wants,
    Requires,
    Upholds,
}

impl DepDirKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wants => "Wants",
            Self::Requires => "Requires",
            Self::Upholds => "Upholds",
        }
    }
}

/// Represents a dependency relationship discovered from a `.wants/`, `.requires/`,
/// or `.upholds/` directory.
#[derive(Debug, Clone)]
pub struct DirectoryDependency {
    /// The unit that has the dependency (e.g., "multi-user.target")
    pub parent_unit: String,
    /// The unit that is wanted/required/upheld (e.g., "getty.target")
    pub child_unit: String,
    /// The kind of dependency (Wants, Requires, or Upholds).
    pub dep_kind: DepDirKind,
    /// Priority of the search path this was found in (lower = higher priority).
    /// E.g., /etc = 0, /run = 1, /usr/lib = 3.
    pub priority: usize,
    /// True if the symlink points to /dev/null (masking this dep at this priority).
    pub is_mask: bool,
}

/// Check if a directory name represents a `.wants`, `.requires`, or `.upholds`
/// dependency directory. Returns Some((parent_unit_name, dep_kind)) if it does.
pub fn parse_dep_dir_name(dir_name: &str) -> Option<(String, DepDirKind)> {
    if let Some(parent) = dir_name.strip_suffix(".wants") {
        Some((parent.to_owned(), DepDirKind::Wants))
    } else if let Some(parent) = dir_name.strip_suffix(".requires") {
        Some((parent.to_owned(), DepDirKind::Requires))
    } else {
        dir_name
            .strip_suffix(".upholds")
            .map(|parent| (parent.to_owned(), DepDirKind::Upholds))
    }
}

/// Check if a directory name represents a drop-in override directory.
/// Returns Some(unit_name) if it does (e.g., "getty@.service.d" -> "getty@.service").
pub fn parse_dropin_dir_name(dir_name: &str) -> Option<String> {
    dir_name.strip_suffix(".d").map(|s| s.to_owned())
}

/// Determine if a filename is a unit file we can parse.
pub fn is_unit_file(filename: &str) -> bool {
    filename.ends_with(".service")
        || filename.ends_with(".socket")
        || filename.ends_with(".target")
        || filename.ends_with(".slice")
        || filename.ends_with(".mount")
        || filename.ends_with(".swap")
        || filename.ends_with(".timer")
        || filename.ends_with(".path")
        || filename.ends_with(".device")
}

/// Check if a unit name is a template (e.g., "getty@.service", "modprobe@.service").
/// Template units contain `@.` before the extension and should never be activated directly —
/// only concrete instances (e.g., "getty@tty1.service") should be.
pub fn is_template_unit(name: &str) -> bool {
    // Find the '@' and then check if the next char before the extension dot is '.'
    // i.e., the instance part is empty: "foo@.service"
    if let Some(at_pos) = name.find('@')
        && let Some(dot_pos) = name.rfind('.')
        && dot_pos > at_pos
    {
        let instance = &name[at_pos + 1..dot_pos];
        return instance.is_empty();
    }
    false
}

/// Resolve symlink aliases among loaded units.
///
/// When a unit file is a symlink to another unit file (e.g., `default.target` → `multi-user.target`),
/// both get loaded as separate units with the same content.  This means `.wants/` directories
/// keyed by the *real* name (e.g., `multi-user.target.wants/`) are not associated with the
/// alias name (`default.target`).
///
/// This function detects such symlink relationships and merges the real unit's dependencies
/// into the alias unit, ensuring `.wants/` and `.requires/` directories for either name
/// apply to both.
pub fn resolve_symlink_aliases(
    unit_table: &mut HashMap<UnitId, Unit>,
    _dir_deps: &[DirectoryDependency],
    unit_dirs: &[PathBuf],
) {
    // Build a mapping: filename -> symlink target filename (if it's a symlink to another unit)
    let mut alias_map: HashMap<String, String> = HashMap::new(); // alias_name -> real_name

    for dir in unit_dirs {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if !is_unit_file(&file_name) {
                continue;
            }
            let path = entry.path();
            // Check if this is a symlink
            if let Ok(metadata) = std::fs::symlink_metadata(&path)
                && metadata.file_type().is_symlink()
                && let Ok(target) = std::fs::read_link(&path)
            {
                let target_name = target
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                if is_unit_file(&target_name) && target_name != file_name {
                    trace!("Symlink alias detected: {} -> {}", file_name, target_name);
                    alias_map.insert(file_name, target_name);
                }
            }
        }
    }

    if alias_map.is_empty() {
        info!(
            "resolve_symlink_aliases: no symlink aliases detected in {} unit dir(s)",
            unit_dirs.len()
        );
        return;
    }

    info!(
        "resolve_symlink_aliases: detected {} symlink alias(es): {:?}",
        alias_map.len(),
        alias_map
            .iter()
            .map(|(a, r)| format!("{} -> {}", a, r))
            .collect::<Vec<_>>()
    );

    // For each alias relationship:
    // 1. Merge ALL dependencies from the alias unit into the real unit
    // 2. Register the alias name on the real unit
    // 3. Remove the alias unit from the table
    // 4. Rewrite all references to the alias UnitId → real UnitId
    //
    // In real systemd, aliases don't exist as separate units — they're just
    // alternative names that resolve to the same unit.  Having both in the
    // table causes the same service to be started twice (e.g.
    // systemd-resolved.service AND dbus-org.freedesktop.resolve1.service
    // both launching the resolved binary).

    // Collect (alias_id, real_id) pairs for aliases where both exist in the table.
    let mut alias_to_real: Vec<(UnitId, UnitId)> = Vec::new();

    for (alias_name, real_name) in &alias_map {
        let alias_id = unit_table.keys().find(|id| id.name == *alias_name).cloned();
        let real_id = unit_table.keys().find(|id| id.name == *real_name).cloned();

        match (alias_id, real_id) {
            (Some(alias_id), Some(real_id)) => {
                alias_to_real.push((alias_id, real_id));
            }
            (Some(alias_id), None) => {
                // Alias exists but real unit doesn't — the alias IS the real unit
                // under a different name. Keep it but register the mapping so
                // lookups by real_name can find it.
                if let Some(alias_unit) = unit_table.get_mut(&alias_id) {
                    // The "real name" from the symlink target is actually the
                    // canonical name. Register it as an alias on this unit.
                    if !alias_unit.common.unit.aliases.contains(real_name) {
                        alias_unit.common.unit.aliases.push(real_name.clone());
                    }
                    trace!(
                        "Alias {} exists but target {} not loaded — keeping alias as primary, registering target name",
                        alias_name, real_name
                    );
                }
            }
            _ => {}
        }
    }

    // Phase 1: Merge deps from each alias into its real unit, then remove the alias.
    for (alias_id, real_id) in &alias_to_real {
        let alias_name = alias_id.name.clone();
        let real_name = real_id.name.clone();

        // Extract all deps from the alias unit before removing it.
        let alias_deps = unit_table.get(alias_id).map(|u| {
            (
                u.common.dependencies.wants.clone(),
                u.common.dependencies.wanted_by.clone(),
                u.common.dependencies.requires.clone(),
                u.common.dependencies.required_by.clone(),
                u.common.dependencies.before.clone(),
                u.common.dependencies.after.clone(),
                u.common.dependencies.conflicts.clone(),
                u.common.dependencies.conflicted_by.clone(),
                u.common.dependencies.part_of.clone(),
                u.common.dependencies.part_of_by.clone(),
                u.common.dependencies.binds_to.clone(),
                u.common.dependencies.bound_by.clone(),
                u.common.dependencies.upholds.clone(),
                u.common.dependencies.upheld_by.clone(),
                u.common.dependencies.propagates_stop_to.clone(),
                u.common.unit.refs_by_name.clone(),
            )
        });

        if let Some((
            a_wants,
            a_wanted_by,
            a_requires,
            a_required_by,
            a_before,
            a_after,
            a_conflicts,
            a_conflicted_by,
            a_part_of,
            a_part_of_by,
            a_binds_to,
            a_bound_by,
            a_upholds,
            a_upheld_by,
            a_propagates_stop_to,
            a_refs,
        )) = alias_deps
            && let Some(real_unit) = unit_table.get_mut(real_id)
        {
            // Merge all dependency types from alias into real unit.
            // Skip deps that reference the real unit itself (self-loops).
            merge_dep_vec(&mut real_unit.common.dependencies.wants, &a_wants, real_id);
            merge_dep_vec(
                &mut real_unit.common.dependencies.wanted_by,
                &a_wanted_by,
                real_id,
            );
            merge_dep_vec(
                &mut real_unit.common.dependencies.requires,
                &a_requires,
                real_id,
            );
            merge_dep_vec(
                &mut real_unit.common.dependencies.required_by,
                &a_required_by,
                real_id,
            );
            merge_dep_vec(
                &mut real_unit.common.dependencies.before,
                &a_before,
                real_id,
            );
            merge_dep_vec(&mut real_unit.common.dependencies.after, &a_after, real_id);
            merge_dep_vec(
                &mut real_unit.common.dependencies.conflicts,
                &a_conflicts,
                real_id,
            );
            merge_dep_vec(
                &mut real_unit.common.dependencies.conflicted_by,
                &a_conflicted_by,
                real_id,
            );
            merge_dep_vec(
                &mut real_unit.common.dependencies.part_of,
                &a_part_of,
                real_id,
            );
            merge_dep_vec(
                &mut real_unit.common.dependencies.part_of_by,
                &a_part_of_by,
                real_id,
            );
            merge_dep_vec(
                &mut real_unit.common.dependencies.binds_to,
                &a_binds_to,
                real_id,
            );
            merge_dep_vec(
                &mut real_unit.common.dependencies.bound_by,
                &a_bound_by,
                real_id,
            );
            merge_dep_vec(
                &mut real_unit.common.dependencies.upholds,
                &a_upholds,
                real_id,
            );
            merge_dep_vec(
                &mut real_unit.common.dependencies.upheld_by,
                &a_upheld_by,
                real_id,
            );
            merge_dep_vec(
                &mut real_unit.common.dependencies.propagates_stop_to,
                &a_propagates_stop_to,
                real_id,
            );
            merge_dep_vec(&mut real_unit.common.unit.refs_by_name, &a_refs, real_id);

            // Register the alias name so find_units_with_name can match it.
            if !real_unit.common.unit.aliases.contains(&alias_name) {
                real_unit.common.unit.aliases.push(alias_name.clone());
            }

            info!(
                "Alias {} merged into {} and removed from unit table",
                alias_name, real_name
            );
        }

        // Remove the alias unit from the table.
        if unit_table.remove(alias_id).is_some() {
            info!(
                "resolve_symlink_aliases: removed alias unit {} from table (real: {})",
                alias_id.name, real_id.name
            );
        }
    }

    info!(
        "resolve_symlink_aliases: {} alias(es) removed, {} units remain in table",
        alias_to_real.len(),
        unit_table.len()
    );

    // Phase 1b: Handle instances of template aliases.
    //
    // When a template alias like `autovt@.service` → `getty@.service` was
    // resolved above, only the template unit itself was merged/removed.
    // However, `instantiate_template_units` may have already created
    // concrete instances (e.g. `autovt@tty1.service`) from the alias
    // template.  These instances need to be merged into the corresponding
    // real-template instances (e.g. `getty@tty1.service`) — or renamed if
    // the real instance doesn't exist yet — so that the rest of the system
    // sees a single canonical unit name.
    //
    // Build a template alias map from the filesystem alias_map: entries
    // where both the alias and target are template names (contain `@.`).
    let template_alias_map: HashMap<String, String> = alias_map
        .iter()
        .filter(|(alias, real)| is_template_unit(alias) && is_template_unit(real))
        .map(|(a, r)| (a.clone(), r.clone()))
        .collect();

    let mut instance_alias_to_real: Vec<(UnitId, UnitId)> = Vec::new();

    if !template_alias_map.is_empty() {
        info!(
            "resolve_symlink_aliases: detected {} template alias(es) for instance rewriting: {:?}",
            template_alias_map.len(),
            template_alias_map
                .iter()
                .map(|(a, r)| format!("{} -> {}", a, r))
                .collect::<Vec<_>>()
        );

        // For each template alias, find instances in the unit table whose
        // name matches the alias template pattern and compute the real name.
        for (alias_template, real_template) in &template_alias_map {
            // Extract the prefix and suffix from the alias template.
            // e.g. "autovt@.service" → prefix="autovt@", suffix=".service"
            let Some(alias_at) = alias_template.find('@') else {
                continue;
            };
            let alias_prefix = &alias_template[..alias_at + 1]; // "autovt@"
            let Some(alias_dot) = alias_template.rfind('.') else {
                continue;
            };
            let alias_suffix = &alias_template[alias_dot..]; // ".service"

            // Same for the real template.
            let Some(real_at) = real_template.find('@') else {
                continue;
            };
            let real_prefix = &real_template[..real_at + 1]; // "getty@"
            let Some(real_dot) = real_template.rfind('.') else {
                continue;
            };
            let real_suffix = &real_template[real_dot..]; // ".service"

            // Suffixes must match (both .service, both .socket, etc.)
            if alias_suffix != real_suffix {
                continue;
            }

            // Find all instances of the alias template in the unit table.
            let alias_instances: Vec<UnitId> = unit_table
                .keys()
                .filter(|id| {
                    id.name.starts_with(alias_prefix)
                        && id.name.ends_with(alias_suffix)
                        && !is_template_unit(&id.name)
                })
                .cloned()
                .collect();

            for alias_instance_id in alias_instances {
                // Extract the instance part: "autovt@tty1.service" → "tty1"
                let instance_part = &alias_instance_id.name
                    [alias_prefix.len()..alias_instance_id.name.len() - alias_suffix.len()];
                if instance_part.is_empty() {
                    continue;
                }

                // Build the real instance name: "getty@tty1.service"
                let real_instance_name = format!("{}{}{}", real_prefix, instance_part, real_suffix);

                let real_instance_id: UnitId = match real_instance_name.as_str().try_into() {
                    Ok(id) => id,
                    Err(_) => continue,
                };

                info!(
                    "resolve_symlink_aliases: template instance alias {} -> {}",
                    alias_instance_id.name, real_instance_name
                );

                if unit_table.contains_key(&real_instance_id) {
                    // Both exist — merge alias instance deps into real instance,
                    // then remove the alias instance.
                    let alias_deps = unit_table.get(&alias_instance_id).map(|u| {
                        (
                            u.common.dependencies.wants.clone(),
                            u.common.dependencies.wanted_by.clone(),
                            u.common.dependencies.requires.clone(),
                            u.common.dependencies.required_by.clone(),
                            u.common.dependencies.before.clone(),
                            u.common.dependencies.after.clone(),
                            u.common.dependencies.conflicts.clone(),
                            u.common.dependencies.conflicted_by.clone(),
                            u.common.dependencies.part_of.clone(),
                            u.common.dependencies.part_of_by.clone(),
                            u.common.dependencies.binds_to.clone(),
                            u.common.dependencies.bound_by.clone(),
                            u.common.dependencies.upholds.clone(),
                            u.common.dependencies.upheld_by.clone(),
                            u.common.dependencies.propagates_stop_to.clone(),
                            u.common.unit.refs_by_name.clone(),
                        )
                    });

                    if let Some((
                        a_wants,
                        a_wanted_by,
                        a_requires,
                        a_required_by,
                        a_before,
                        a_after,
                        a_conflicts,
                        a_conflicted_by,
                        a_part_of,
                        a_part_of_by,
                        a_binds_to,
                        a_bound_by,
                        a_upholds,
                        a_upheld_by,
                        a_propagates_stop_to,
                        a_refs,
                    )) = alias_deps
                        && let Some(real_unit) = unit_table.get_mut(&real_instance_id)
                    {
                        merge_dep_vec(
                            &mut real_unit.common.dependencies.wants,
                            &a_wants,
                            &real_instance_id,
                        );
                        merge_dep_vec(
                            &mut real_unit.common.dependencies.wanted_by,
                            &a_wanted_by,
                            &real_instance_id,
                        );
                        merge_dep_vec(
                            &mut real_unit.common.dependencies.requires,
                            &a_requires,
                            &real_instance_id,
                        );
                        merge_dep_vec(
                            &mut real_unit.common.dependencies.required_by,
                            &a_required_by,
                            &real_instance_id,
                        );
                        merge_dep_vec(
                            &mut real_unit.common.dependencies.before,
                            &a_before,
                            &real_instance_id,
                        );
                        merge_dep_vec(
                            &mut real_unit.common.dependencies.after,
                            &a_after,
                            &real_instance_id,
                        );
                        merge_dep_vec(
                            &mut real_unit.common.dependencies.conflicts,
                            &a_conflicts,
                            &real_instance_id,
                        );
                        merge_dep_vec(
                            &mut real_unit.common.dependencies.conflicted_by,
                            &a_conflicted_by,
                            &real_instance_id,
                        );
                        merge_dep_vec(
                            &mut real_unit.common.dependencies.part_of,
                            &a_part_of,
                            &real_instance_id,
                        );
                        merge_dep_vec(
                            &mut real_unit.common.dependencies.part_of_by,
                            &a_part_of_by,
                            &real_instance_id,
                        );
                        merge_dep_vec(
                            &mut real_unit.common.dependencies.binds_to,
                            &a_binds_to,
                            &real_instance_id,
                        );
                        merge_dep_vec(
                            &mut real_unit.common.dependencies.bound_by,
                            &a_bound_by,
                            &real_instance_id,
                        );
                        merge_dep_vec(
                            &mut real_unit.common.dependencies.upholds,
                            &a_upholds,
                            &real_instance_id,
                        );
                        merge_dep_vec(
                            &mut real_unit.common.dependencies.upheld_by,
                            &a_upheld_by,
                            &real_instance_id,
                        );
                        merge_dep_vec(
                            &mut real_unit.common.dependencies.propagates_stop_to,
                            &a_propagates_stop_to,
                            &real_instance_id,
                        );
                        merge_dep_vec(
                            &mut real_unit.common.unit.refs_by_name,
                            &a_refs,
                            &real_instance_id,
                        );

                        if !real_unit
                            .common
                            .unit
                            .aliases
                            .contains(&alias_instance_id.name)
                        {
                            real_unit
                                .common
                                .unit
                                .aliases
                                .push(alias_instance_id.name.clone());
                        }
                    }

                    unit_table.remove(&alias_instance_id);
                    info!(
                        "resolve_symlink_aliases: merged template instance {} into {} and removed",
                        alias_instance_id.name, real_instance_name
                    );
                } else {
                    // Only the alias instance exists — rename it to the real name.
                    if let Some(mut unit) = unit_table.remove(&alias_instance_id) {
                        unit.common
                            .unit
                            .aliases
                            .push(alias_instance_id.name.clone());
                        unit.id = real_instance_id.clone();
                        unit_table.insert(real_instance_id.clone(), unit);
                        info!(
                            "resolve_symlink_aliases: renamed template instance {} -> {}",
                            alias_instance_id.name, real_instance_name
                        );
                    }
                }

                instance_alias_to_real.push((alias_instance_id, real_instance_id));
            }
        }

        if !instance_alias_to_real.is_empty() {
            info!(
                "resolve_symlink_aliases: rewrote {} template instance alias(es)",
                instance_alias_to_real.len()
            );
        }
    }

    // Phase 2: Rewrite all references to alias UnitIds → real UnitIds
    // throughout the entire unit table.  This includes both the template
    // aliases from Phase 1 and the instance aliases from Phase 1b.
    let all_aliases: Vec<(UnitId, UnitId)> = alias_to_real
        .into_iter()
        .chain(instance_alias_to_real)
        .collect();

    if !all_aliases.is_empty() {
        // Build a lookup map: alias_id -> real_id
        let redirect: HashMap<UnitId, UnitId> = all_aliases.into_iter().collect();

        for unit in unit_table.values_mut() {
            rewrite_id_vec(&mut unit.common.dependencies.wants, &redirect);
            rewrite_id_vec(&mut unit.common.dependencies.wanted_by, &redirect);
            rewrite_id_vec(&mut unit.common.dependencies.requires, &redirect);
            rewrite_id_vec(&mut unit.common.dependencies.required_by, &redirect);
            rewrite_id_vec(&mut unit.common.dependencies.before, &redirect);
            rewrite_id_vec(&mut unit.common.dependencies.after, &redirect);
            rewrite_id_vec(&mut unit.common.dependencies.conflicts, &redirect);
            rewrite_id_vec(&mut unit.common.dependencies.conflicted_by, &redirect);
            rewrite_id_vec(&mut unit.common.dependencies.part_of, &redirect);
            rewrite_id_vec(&mut unit.common.dependencies.part_of_by, &redirect);
            rewrite_id_vec(&mut unit.common.dependencies.binds_to, &redirect);
            rewrite_id_vec(&mut unit.common.dependencies.bound_by, &redirect);
            rewrite_id_vec(&mut unit.common.dependencies.propagates_stop_to, &redirect);
            rewrite_id_vec(&mut unit.common.unit.refs_by_name, &redirect);

            // Also rewrite socket<->service cross-references
            match &mut unit.specific {
                Specific::Service(svc) => {
                    rewrite_id_vec(&mut svc.conf.sockets, &redirect);
                }
                Specific::Socket(sock) => {
                    rewrite_id_vec(&mut sock.conf.services, &redirect);
                }
                _ => {}
            }
        }
    }
}

/// Merge `source` UnitIds into `target` vec, skipping duplicates and self-references.
fn merge_dep_vec(target: &mut Vec<UnitId>, source: &[UnitId], skip_id: &UnitId) {
    for id in source {
        if id != skip_id && !target.contains(id) {
            target.push(id.clone());
        }
    }
}

/// Rewrite UnitIds in a vec according to the redirect map (alias→real).
/// Also deduplicates after rewriting (sort+dedup to catch non-consecutive dupes).
fn rewrite_id_vec(ids: &mut Vec<UnitId>, redirect: &HashMap<UnitId, UnitId>) {
    let mut changed = false;
    for id in ids.iter_mut() {
        if let Some(real_id) = redirect.get(id) {
            *id = real_id.clone();
            changed = true;
        }
    }
    if changed {
        ids.sort();
        ids.dedup();
    }
}

/// Insert a parsed unit into the appropriate table based on the file extension.
pub fn insert_parsed_unit(
    services: &mut HashMap<UnitId, Unit>,
    sockets: &mut HashMap<UnitId, Unit>,
    targets: &mut HashMap<UnitId, Unit>,
    slices: &mut HashMap<UnitId, Unit>,
    parsed_file: ParsedFile,
    path: &PathBuf,
) {
    use crate::units::parse_timer;

    let path_str = path.to_str().unwrap_or("");

    // Handle .path files
    if path_str.ends_with(".path") {
        trace!("Path unit found: {:?}", path);
        match parse_path(parsed_file, path).and_then(|parsed| {
            TryInto::<Unit>::try_into(parsed).map_err(ParsingErrorReason::Generic)
        }) {
            Ok(unit) => {
                // Path units are stored in the targets table (which serves as
                // the catch-all for simple unit types like target/slice/timer/path)
                targets.insert(unit.id.clone(), unit);
            }
            Err(e) => {
                warn!("Skipping path unit {:?}: {:?}", path, e);
            }
        }
        return;
    }

    if path_str.ends_with(".service") {
        trace!("Service found: {:?}", path);
        match parse_service(parsed_file, path).and_then(|parsed| {
            TryInto::<Unit>::try_into(parsed).map_err(ParsingErrorReason::Generic)
        }) {
            Ok(unit) => {
                services.insert(unit.id.clone(), unit);
            }
            Err(e) => {
                warn!("Skipping service {:?}: {:?}", path, e);
            }
        }
    } else if path_str.ends_with(".socket") {
        trace!("Socket found: {:?}", path);
        match parse_socket(parsed_file, path).and_then(|parsed| {
            TryInto::<Unit>::try_into(parsed).map_err(ParsingErrorReason::Generic)
        }) {
            Ok(unit) => {
                sockets.insert(unit.id.clone(), unit);
            }
            Err(e) => {
                warn!("Skipping socket {:?}: {:?}", path, e);
            }
        }
    } else if path_str.ends_with(".target") {
        trace!("Target found: {:?}", path);
        match parse_target(parsed_file, path).and_then(|parsed| {
            TryInto::<Unit>::try_into(parsed).map_err(ParsingErrorReason::Generic)
        }) {
            Ok(unit) => {
                targets.insert(unit.id.clone(), unit);
            }
            Err(e) => {
                warn!("Skipping target {:?}: {:?}", path, e);
            }
        }
    } else if path_str.ends_with(".slice") {
        trace!("Slice found: {:?}", path);
        match parse_slice(parsed_file, path).and_then(|parsed| {
            TryInto::<Unit>::try_into(parsed).map_err(ParsingErrorReason::Generic)
        }) {
            Ok(unit) => {
                slices.insert(unit.id.clone(), unit);
            }
            Err(e) => {
                warn!("Skipping slice {:?}: {:?}", path, e);
            }
        }
    } else if path_str.ends_with(".mount") {
        trace!("Mount found: {:?}", path);
        match parse_mount(parsed_file, path).and_then(|parsed| {
            TryInto::<Unit>::try_into(parsed).map_err(ParsingErrorReason::Generic)
        }) {
            Ok(unit) => {
                // Mount units are stored in the slices table (which serves as
                // the catch-all for non-service/socket/target unit types)
                slices.insert(unit.id.clone(), unit);
            }
            Err(e) => {
                warn!("Skipping mount {:?}: {:?}", path, e);
            }
        }
    } else if path_str.ends_with(".swap") {
        trace!("Swap found: {:?}", path);
        match parse_swap(parsed_file, path).and_then(|parsed| {
            TryInto::<Unit>::try_into(parsed).map_err(ParsingErrorReason::Generic)
        }) {
            Ok(unit) => {
                // Swap units are stored in the slices table (which serves as
                // the catch-all for non-service/socket/target unit types)
                slices.insert(unit.id.clone(), unit);
            }
            Err(e) => {
                warn!("Skipping swap {:?}: {:?}", path, e);
            }
        }
    } else if path_str.ends_with(".timer") {
        trace!("Timer found: {:?}", path);
        match parse_timer(parsed_file, path).and_then(|parsed| {
            TryInto::<Unit>::try_into(parsed).map_err(ParsingErrorReason::Generic)
        }) {
            Ok(unit) => {
                // Timer units are stored in the targets table (which serves as
                // the catch-all for simple unit types like target/slice/timer)
                targets.insert(unit.id.clone(), unit);
            }
            Err(e) => {
                warn!("Skipping timer {:?}: {:?}", path, e);
            }
        }
    } else if path_str.ends_with(".device") {
        trace!("Device found: {:?}", path);
        match parse_device(parsed_file, path).and_then(|parsed| {
            TryInto::<Unit>::try_into(parsed).map_err(ParsingErrorReason::Generic)
        }) {
            Ok(unit) => {
                // Device units are stored in the targets table (which serves as
                // the catch-all for simple unit types like target/slice/timer/path/device)
                targets.insert(unit.id.clone(), unit);
            }
            Err(e) => {
                warn!("Skipping device {:?}: {:?}", path, e);
            }
        }
    }
}

/// Collect entries from a `.wants/`, `.requires/`, or `.upholds/` directory
/// and record the dependency relationships.
pub fn collect_dep_dir_entries(
    dir_path: &Path,
    parent_unit: &str,
    dep_kind: DepDirKind,
    priority: usize,
    dir_deps: &mut Vec<DirectoryDependency>,
) {
    let entries = match std::fs::read_dir(dir_path) {
        Ok(entries) => entries,
        Err(e) => {
            warn!("Could not read dependency directory {:?}: {}", dir_path, e);
            return;
        }
    };

    for entry in entries.flatten() {
        let child_name = entry.file_name().to_string_lossy().to_string();
        if is_unit_file(&child_name) {
            // Check if this symlink points to /dev/null (masking)
            let is_mask = entry
                .path()
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
                && std::fs::read_link(entry.path())
                    .map(|target| target == std::path::Path::new("/dev/null"))
                    .unwrap_or(false);

            trace!(
                "Directory dependency: {} {} {}{} (priority={})",
                parent_unit,
                dep_kind.label(),
                child_name,
                if is_mask { " [MASKED]" } else { "" },
                priority
            );
            dir_deps.push(DirectoryDependency {
                parent_unit: parent_unit.to_owned(),
                child_unit: child_name,
                dep_kind,
                priority,
                is_mask,
            });
        }
    }
}

/// Collect directory-based dependencies for a specific unit from all unit dirs.
/// Scans `{unit_name}.wants/`, `{unit_name}.requires/`, and `{unit_name}.upholds/`
/// in each search path, applies priority-based `/dev/null` masking, and returns
/// the effective deps.
pub fn collect_dir_deps_for_unit(
    unit_dirs: &[std::path::PathBuf],
    unit_name: &str,
) -> Vec<DirectoryDependency> {
    let mut dir_deps = Vec::new();
    for (priority, dir) in unit_dirs.iter().enumerate() {
        let wants_dir = dir.join(format!("{unit_name}.wants"));
        if wants_dir.is_dir() {
            collect_dep_dir_entries(
                &wants_dir,
                unit_name,
                DepDirKind::Wants,
                priority,
                &mut dir_deps,
            );
        }
        let requires_dir = dir.join(format!("{unit_name}.requires"));
        if requires_dir.is_dir() {
            collect_dep_dir_entries(
                &requires_dir,
                unit_name,
                DepDirKind::Requires,
                priority,
                &mut dir_deps,
            );
        }
        let upholds_dir = dir.join(format!("{unit_name}.upholds"));
        if upholds_dir.is_dir() {
            collect_dep_dir_entries(
                &upholds_dir,
                unit_name,
                DepDirKind::Upholds,
                priority,
                &mut dir_deps,
            );
        }
    }
    resolve_masked_dir_deps(&dir_deps)
}

/// Collect entries from a `.d/` drop-in directory and store their contents.
pub fn collect_dropin_entries(
    dir_path: &Path,
    unit_name: &str,
    dropins: &mut HashMap<String, Vec<(String, String)>>,
) {
    let mut entries: Vec<_> = match std::fs::read_dir(dir_path) {
        Ok(entries) => entries.flatten().collect(),
        Err(e) => {
            warn!("Could not read drop-in directory {:?}: {}", dir_path, e);
            return;
        }
    };
    // Sort by filename for deterministic override ordering
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let filename = entry.file_name().to_string_lossy().to_string();
        if !filename.ends_with(".conf") {
            continue;
        }
        let content = match std::fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(e) => {
                warn!("Could not read drop-in {:?}: {}", entry.path(), e);
                continue;
            }
        };
        trace!("Drop-in for {}: {:?}", unit_name, entry.path());
        dropins
            .entry(unit_name.to_owned())
            .or_default()
            .push((filename, content));
    }
}

/// Apply drop-in overrides to already-loaded units.
///
/// For each unit that has drop-in overrides, we find the base unit file,
/// merge the drop-in content, re-parse, and replace the unit in the table.
///
/// Drop-in merging follows systemd semantics:
/// - Sections from drop-ins are merged into the base file
/// - Settings in drop-ins override settings in the base file
/// - If a drop-in sets a list-type value to empty (e.g., `ExecStart=`),
///   it clears the base value before adding new entries
///
/// Collect all applicable drop-in overrides for a given unit name, ordered from
/// least specific to most specific:
///   1. Type-level drop-ins (e.g., key "service" applies to all .service units)
///   2. Hierarchical prefix drop-ins (e.g., "a-.service" → "a-b-.service" → …)
///   3. Exact unit name drop-ins (e.g., "a-b-c.service")
///
/// Specifier resolution (`%n` → unit name, etc.) is applied to each drop-in's
/// content before returning.
fn collect_applicable_dropins(
    unit_name: &str,
    dropins: &HashMap<String, Vec<(String, String)>>,
) -> Vec<(String, String)> {
    let mut result: Vec<(String, String)> = Vec::new();

    // Extract the unit type suffix (e.g., ".service", ".socket")
    let type_suffix = unit_name
        .rfind('.')
        .map(|i| &unit_name[i + 1..])
        .unwrap_or("");

    // 1. Type-level drop-ins: key is just the type name (e.g., "service")
    if !type_suffix.is_empty()
        && let Some(overrides) = dropins.get(type_suffix)
    {
        for (filename, content) in overrides {
            let resolved = resolve_specifiers(content, unit_name, "");
            result.push((filename.clone(), resolved));
        }
    }

    // 2. Hierarchical prefix drop-ins (e.g., "a-.service" for "a-b-c.service")
    //    systemd matches: "a-.service", "a-b-.service" (all dash-separated prefixes)
    if let Some(dot_pos) = unit_name.rfind('.') {
        let base = &unit_name[..dot_pos]; // e.g., "a-b-c"
        let suffix = &unit_name[dot_pos..]; // e.g., ".service"
        let parts: Vec<&str> = base.split('-').collect();
        // Generate prefixes: for "a-b-c" → "a-", "a-b-"
        for i in 1..parts.len() {
            let prefix = parts[..i].join("-");
            let key = format!("{prefix}-{suffix}"); // e.g., "a-.service"
            if let Some(overrides) = dropins.get(&key) {
                for (filename, content) in overrides {
                    let resolved = resolve_specifiers(content, unit_name, "");
                    result.push((filename.clone(), resolved));
                }
            }
        }
    }

    // 3. Exact unit name drop-ins
    if let Some(overrides) = dropins.get(unit_name) {
        for (filename, content) in overrides {
            let resolved = resolve_specifiers(content, unit_name, "");
            result.push((filename.clone(), resolved));
        }
    }

    result
}

/// Public wrapper for `collect_applicable_dropins` for use by transient unit drop-in application.
pub fn collect_applicable_dropins_pub(
    unit_name: &str,
    dropins: &HashMap<String, Vec<(String, String)>>,
) -> Vec<(String, String)> {
    collect_applicable_dropins(unit_name, dropins)
}

/// Public wrapper for `merge_unit_contents` for use by on-demand unit loading.
pub fn merge_unit_contents_pub(base: &str, overrides: &[(String, String)]) -> String {
    merge_unit_contents(base, overrides)
}

/// Scan the dropins map for keys that look like slice units (e.g., "a-b-c.slice")
/// and create implicit (fragmentless) slice units for any that don't already exist
/// in the unit table. In systemd, slices exist implicitly without a unit file.
pub fn create_implicit_slices_from_dropins(
    slice_units: &mut HashMap<UnitId, Unit>,
    dropins: &HashMap<String, Vec<(String, String)>>,
    unit_dirs: &[PathBuf],
) {
    // Collect slice names that have drop-ins but no unit file on disk and no entry in the table.
    let mut implicit_names: Vec<String> = Vec::new();
    for key in dropins.keys() {
        if key.ends_with(".slice") {
            let already_loaded = slice_units.values().any(|u| u.id.name == *key);
            if !already_loaded && find_unit_file_in_dirs(unit_dirs, key).is_none() {
                implicit_names.push(key.clone());
            }
        }
    }

    for slice_name in implicit_names {
        let base = slice_name.strip_suffix(".slice").unwrap_or(&slice_name);
        let path = base.replace('-', "/");
        let desc = format!("Slice /{path}");
        let content = format!("[Unit]\nDescription={desc}\n\n[Slice]\n");
        let fake_path = PathBuf::from(format!("/run/systemd/system/{slice_name}"));
        let parsed_file = match parse_file(&content) {
            Ok(pf) => pf,
            Err(_) => continue,
        };
        let parsed_config = match parse_slice(parsed_file, &fake_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let unit: Result<Unit, _> = parsed_config.try_into();
        if let Ok(unit) = unit {
            info!("Created implicit slice unit from drop-ins: {}", slice_name);
            slice_units.insert(unit.id.clone(), unit);
        }
    }
}

pub fn apply_dropins(
    units: &mut HashMap<UnitId, Unit>,
    dropins: &HashMap<String, Vec<(String, String)>>,
    unit_dirs: &[PathBuf],
) {
    let unit_names: Vec<String> = units.keys().map(|id| id.name.clone()).collect();

    for unit_name in unit_names {
        let overrides = collect_applicable_dropins(&unit_name, dropins);
        if overrides.is_empty() {
            continue;
        }

        // Find the unit in the table
        let unit_id = match units.keys().find(|id| id.name == unit_name) {
            Some(id) => id.clone(),
            None => continue,
        };

        // Get the base content from the unit's source path.
        // For implicit units (e.g. slices without a fragment file), use a minimal default.
        let (base_path, base_content) =
            if let Some(p) = find_unit_file_in_dirs(unit_dirs, &unit_name) {
                match std::fs::read_to_string(&p) {
                    Ok(c) => (p, c),
                    Err(_) => continue,
                }
            } else if unit_name.ends_with(".slice") {
                // Implicit slice: generate a minimal base
                let base = unit_name.strip_suffix(".slice").unwrap_or(&unit_name);
                let path = base.replace('-', "/");
                let desc = format!("Slice /{path}");
                let fake = PathBuf::from(format!("/run/systemd/system/{unit_name}"));
                let content = format!("[Unit]\nDescription={desc}\n\n[Slice]\n");
                (fake, content)
            } else {
                continue;
            };

        // Resolve specifiers in the base content (%n, %i, %p, etc.)
        let base_content = resolve_specifiers(&base_content, &unit_name, "");

        // Merge base + all drop-in contents
        let merged = merge_unit_contents(&base_content, &overrides);

        let parsed_file = match parse_file(&merged) {
            Ok(pf) => pf,
            Err(e) => {
                warn!("Failed to re-parse {} with drop-ins: {:?}", unit_name, e);
                continue;
            }
        };

        // Re-parse the merged unit
        let new_unit = if unit_name.ends_with(".service") {
            parse_service(parsed_file, &base_path)
                .and_then(|p| TryInto::<Unit>::try_into(p).map_err(ParsingErrorReason::Generic))
        } else if unit_name.ends_with(".socket") {
            parse_socket(parsed_file, &base_path)
                .and_then(|p| TryInto::<Unit>::try_into(p).map_err(ParsingErrorReason::Generic))
        } else if unit_name.ends_with(".target") {
            parse_target(parsed_file, &base_path)
                .and_then(|p| TryInto::<Unit>::try_into(p).map_err(ParsingErrorReason::Generic))
        } else if unit_name.ends_with(".slice") {
            parse_slice(parsed_file, &base_path)
                .and_then(|p| TryInto::<Unit>::try_into(p).map_err(ParsingErrorReason::Generic))
        } else {
            continue;
        };

        match new_unit {
            Ok(mut unit) => {
                // Record the drop-in file paths for NeedDaemonReload detection.
                let dropin_dirname = format!("{}.d", unit_name);
                let mut dropin_files = Vec::new();
                for dir in unit_dirs {
                    let dropin_dir = dir.join(&dropin_dirname);
                    if let Ok(entries) = std::fs::read_dir(&dropin_dir) {
                        for entry in entries.flatten() {
                            if entry.file_name().to_string_lossy().ends_with(".conf") {
                                dropin_files.push(entry.path());
                            }
                        }
                    }
                }
                dropin_files.sort();
                unit.common.unit.loaded_dropin_files = dropin_files;

                trace!("Applied drop-in overrides to {}", unit_name);
                units.remove(&unit_id);
                units.insert(unit.id.clone(), unit);
            }
            Err(e) => {
                warn!("Failed to apply drop-ins to {}: {:?}", unit_name, e);
            }
        }
    }
}

/// Apply directory-based dependencies to the unit table.
/// For each .wants/, .requires/, and .upholds/ relationship discovered, add
/// the appropriate dependency to the parent unit. Respects priority-based
/// masking: if the highest-priority entry for a (parent, child, dep_type)
/// triple is a `/dev/null` symlink, the dependency is suppressed.
pub fn apply_directory_dependencies(
    unit_table: &mut HashMap<UnitId, Unit>,
    dir_deps: &[DirectoryDependency],
) {
    // Resolve masking: for each (parent, child, dep_kind) triple, find the
    // highest-priority (lowest priority value) entry. If it's a mask, skip it.
    let effective_deps = resolve_masked_dir_deps(dir_deps);

    for dep in &effective_deps {
        // Find the parent unit
        let parent_id = match unit_table.keys().find(|id| id.name == dep.parent_unit) {
            Some(id) => id.clone(),
            None => {
                trace!(
                    "Directory dependency: parent unit {} not found, skipping",
                    dep.parent_unit
                );
                continue;
            }
        };

        // The child unit might be a template instance (e.g., "serial-getty@ttyS0.service")
        // Check if it exists in the unit table
        let child_id: UnitId = match dep.child_unit.as_str().try_into() {
            Ok(id) => id,
            Err(_) => continue,
        };

        // Only add the dependency if the child unit exists (or will be instantiated later)
        let child_exists = unit_table.contains_key(&child_id);

        if child_exists {
            if let Some(parent) = unit_table.get_mut(&parent_id) {
                apply_dep_to_unit(
                    parent,
                    &child_id,
                    dep.dep_kind,
                    &dep.parent_unit,
                    &dep.child_unit,
                );
                if !parent.common.unit.refs_by_name.contains(&child_id) {
                    parent.common.unit.refs_by_name.push(child_id);
                }
            }
        } else {
            trace!(
                "Directory dependency: child unit {} not yet loaded (may be template instance)",
                dep.child_unit
            );
        }
    }
}

/// Add a directory dependency of the given kind to the parent unit.
fn apply_dep_to_unit(
    parent: &mut Unit,
    child_id: &UnitId,
    dep_kind: DepDirKind,
    parent_name: &str,
    child_name: &str,
) {
    match dep_kind {
        DepDirKind::Requires => {
            if !parent.common.dependencies.requires.contains(child_id) {
                trace!("Adding directory Requires dependency: {parent_name} -> {child_name}");
                parent.common.dependencies.requires.push(child_id.clone());
            }
        }
        DepDirKind::Wants => {
            if !parent.common.dependencies.wants.contains(child_id) {
                trace!("Adding directory Wants dependency: {parent_name} -> {child_name}");
                parent.common.dependencies.wants.push(child_id.clone());
            }
        }
        DepDirKind::Upholds => {
            if !parent.common.dependencies.upholds.contains(child_id) {
                trace!("Adding directory Upholds dependency: {parent_name} -> {child_name}");
                parent.common.dependencies.upholds.push(child_id.clone());
            }
        }
    }
}

/// Resolve priority-based masking for directory dependencies.
/// For each unique (parent, child, dep_kind) triple, the highest-priority
/// entry (lowest priority number) wins. If that entry is a mask (`/dev/null`
/// symlink), the dependency is suppressed entirely.
pub fn resolve_masked_dir_deps(dir_deps: &[DirectoryDependency]) -> Vec<DirectoryDependency> {
    // Key: (parent_unit, child_unit, dep_kind) -> best (lowest priority) entry
    let mut best: HashMap<(String, String, DepDirKind), &DirectoryDependency> = HashMap::new();

    for dep in dir_deps {
        let key = (
            dep.parent_unit.clone(),
            dep.child_unit.clone(),
            dep.dep_kind,
        );
        match best.get(&key) {
            Some(existing) if existing.priority <= dep.priority => {
                // Existing entry has equal or higher priority; keep it
            }
            _ => {
                best.insert(key, dep);
            }
        }
    }

    best.into_values()
        .filter(|dep| {
            if dep.is_mask {
                trace!(
                    "Masking directory dependency: {} {} {} (priority={})",
                    dep.parent_unit,
                    dep.dep_kind.label(),
                    dep.child_unit,
                    dep.priority
                );
                false
            } else {
                true
            }
        })
        .cloned()
        .collect()
}

/// Extract the template name and instance from a unit name like "serial-getty@ttyS0.service".
/// Returns (template_name, instance) e.g., ("serial-getty@.service", "ttyS0").
pub fn parse_template_instance(unit_name: &str) -> Option<(String, String)> {
    let at_pos = unit_name.find('@')?;
    // Find the suffix (.service, .socket, etc.)
    let dot_pos = unit_name.rfind('.')?;
    if dot_pos <= at_pos {
        return None;
    }
    let instance = &unit_name[at_pos + 1..dot_pos];
    if instance.is_empty() {
        // This is a template itself (e.g., "getty@.service"), not an instance
        return None;
    }
    let template = format!("{}@{}", &unit_name[..at_pos], &unit_name[dot_pos..]);
    Some((template, instance.to_owned()))
}

/// Check whether a string contains unresolved systemd specifiers like `%i`, `%I`,
/// `%n`, `%N`, `%p`, `%P`, etc.  These appear in template unit files as
/// cross-references (e.g. `Requires=systemd-journald@%i.socket`) and must not be
/// treated as literal instance names.
///
/// Delegates to [`crate::specifier::has_unresolved_specifiers`].
fn has_unresolved_specifiers(s: &str) -> bool {
    crate::specifier::has_unresolved_specifiers(s)
}

/// Lazily-initialized system specifier context.
///
/// Created once on first use so that every `resolve_specifiers` call
/// shares the same snapshot of hostname, machine-id, boot-id, etc.
fn system_specifier_context() -> &'static crate::specifier::SpecifierContext {
    use std::sync::OnceLock;
    static CTX: OnceLock<crate::specifier::SpecifierContext> = OnceLock::new();
    CTX.get_or_init(crate::specifier::SpecifierContext::for_system)
}

/// Resolve systemd specifiers in a unit file content string.
///
/// Supports the full set of systemd specifiers (`%i`, `%I`, `%n`, `%N`,
/// `%p`, `%P`, `%u`, `%U`, `%h`, `%s`, `%m`, `%b`, `%H`, `%v`, `%t`,
/// `%a`, `%o`, `%w`, `%q`, `%l`, `%f`, `%j`, `%J`, `%C`, `%E`, `%L`,
/// `%S`, `%T`, `%V`, `%%`, and more).
///
/// Delegates to [`crate::specifier::resolve_specifiers`].
pub fn resolve_specifiers(content: &str, unit_name: &str, instance: &str) -> String {
    crate::specifier::resolve_specifiers(content, unit_name, instance, system_specifier_context())
}

/// Load a template unit, instantiate it with the given instance name,
/// and return the resulting Unit.
pub fn instantiate_template(
    template_name: &str,
    instance_name: &str,
    instance_unit_name: &str,
    unit_dirs: &[PathBuf],
    dropins: &HashMap<String, Vec<(String, String)>>,
) -> Option<Unit> {
    // Find the template file
    let template_path = find_unit_file_in_dirs(unit_dirs, template_name)?;
    let base_content = std::fs::read_to_string(&template_path).ok()?;

    // Get drop-ins for both the template and the instance
    let mut all_overrides: Vec<(String, String)> = Vec::new();
    if let Some(template_dropins) = dropins.get(template_name) {
        all_overrides.extend(template_dropins.iter().cloned());
    }
    if let Some(instance_dropins) = dropins.get(instance_unit_name) {
        all_overrides.extend(instance_dropins.iter().cloned());
    }

    // Merge base + drop-ins
    let merged = if all_overrides.is_empty() {
        base_content
    } else {
        merge_unit_contents(&base_content, &all_overrides)
    };

    // Resolve specifiers
    let resolved = resolve_specifiers(&merged, instance_unit_name, instance_name);

    // Parse
    let parsed_file = parse_file(&resolved).ok()?;

    // Create a fake path with the instance name for the parser
    let instance_path = template_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(instance_unit_name);

    // Parse and convert to Unit
    if instance_unit_name.ends_with(".service") {
        parse_service(parsed_file, &instance_path)
            .and_then(|p| TryInto::<Unit>::try_into(p).map_err(ParsingErrorReason::Generic))
            .ok()
    } else if instance_unit_name.ends_with(".socket") {
        parse_socket(parsed_file, &instance_path)
            .and_then(|p| TryInto::<Unit>::try_into(p).map_err(ParsingErrorReason::Generic))
            .ok()
    } else if instance_unit_name.ends_with(".target") {
        parse_target(parsed_file, &instance_path)
            .and_then(|p| TryInto::<Unit>::try_into(p).map_err(ParsingErrorReason::Generic))
            .ok()
    } else {
        None
    }
}

/// Instantiate template units that are referenced by directory dependencies
/// but not yet in the unit table.
pub fn instantiate_template_units(
    unit_table: &mut HashMap<UnitId, Unit>,
    dir_deps: &[DirectoryDependency],
    unit_dirs: &[PathBuf],
    dropins: &HashMap<String, Vec<(String, String)>>,
) {
    // Collect unique instance names that need to be instantiated
    // Each entry: (instance_unit_name, template_name, instance_name)
    let mut instances_to_create: Vec<(String, String, String)> = Vec::new();

    for dep in dir_deps {
        let child_id: UnitId = match dep.child_unit.as_str().try_into() {
            Ok(id) => id,
            Err(_) => continue,
        };

        // Skip if already loaded
        if unit_table.contains_key(&child_id) {
            continue;
        }

        // Check if this is a template instance
        if let Some((template_name, instance_name)) = parse_template_instance(&dep.child_unit)
            && !instances_to_create
                .iter()
                .any(|(n, _, _)| n == &dep.child_unit)
        {
            // Skip instances with unresolved specifiers (e.g. %i, %I, %n, %N).
            // These come from template unit cross-references like
            // `Requires=systemd-journald@%i.socket` in systemd-journald@.service
            // and should not be instantiated literally.
            if has_unresolved_specifiers(&instance_name) {
                debug!(
                    "Skipping template instance with unresolved specifier: {} (instance={})",
                    dep.child_unit, instance_name
                );
                continue;
            }
            instances_to_create.push((dep.child_unit.clone(), template_name, instance_name));
        }
    }

    // Also check for template instances referenced in Wants=/Requires= of existing units
    let existing_refs: Vec<String> = unit_table
        .values()
        .flat_map(|u| {
            u.common
                .dependencies
                .wants
                .iter()
                .chain(u.common.dependencies.requires.iter())
                .map(|id| id.name.clone())
        })
        .collect();

    for ref_name in &existing_refs {
        let ref_id: UnitId = match ref_name.as_str().try_into() {
            Ok(id) => id,
            Err(_) => continue,
        };
        if unit_table.contains_key(&ref_id) {
            continue;
        }
        if let Some((template_name, instance_name)) = parse_template_instance(ref_name)
            && !instances_to_create.iter().any(|(n, _, _)| n == ref_name)
        {
            // Skip instances with unresolved specifiers (same reason as above).
            if has_unresolved_specifiers(&instance_name) {
                debug!(
                    "Skipping template instance with unresolved specifier: {} (instance={})",
                    ref_name, instance_name
                );
                continue;
            }
            instances_to_create.push((ref_name.clone(), template_name, instance_name));
        }
    }

    // Instantiate each template
    for (instance_unit_name, template_name, instance_name) in &instances_to_create {
        trace!(
            "Instantiating template {} with instance {} -> {}",
            template_name, instance_name, instance_unit_name
        );

        if let Some(mut unit) = instantiate_template(
            template_name,
            instance_name,
            instance_unit_name,
            unit_dirs,
            dropins,
        ) {
            // Discover template-level aliases (e.g. bar-alias@0.service
            // for bar@0.service when bar-alias@.service → bar@.service).
            let unit_path = unit_dirs
                .iter()
                .map(|d| d.join(instance_unit_name))
                .find(|p| p.exists())
                .or_else(|| {
                    unit_dirs
                        .iter()
                        .map(|d| d.join(template_name))
                        .find(|p| p.exists())
                })
                .unwrap_or_default();
            let aliases =
                crate::units::find_symlink_aliases(unit_dirs, &unit_path, instance_unit_name);
            for alias in aliases {
                if !unit.common.unit.aliases.contains(&alias) {
                    trace!("Template instance {} alias: {}", instance_unit_name, alias);
                    unit.common.unit.aliases.push(alias);
                }
            }
            unit_table.insert(unit.id.clone(), unit);
            trace!("Successfully instantiated {}", instance_unit_name);
        } else {
            warn!(
                "Failed to instantiate template {} for instance {}",
                template_name, instance_unit_name
            );
        }
    }

    // Now re-apply directory dependencies for newly instantiated units
    for dep in dir_deps {
        let parent_id = match unit_table.keys().find(|id| id.name == dep.parent_unit) {
            Some(id) => id.clone(),
            None => continue,
        };

        let child_id: UnitId = match dep.child_unit.as_str().try_into() {
            Ok(id) => id,
            Err(_) => continue,
        };

        if !unit_table.contains_key(&child_id) {
            continue;
        }

        if let Some(parent) = unit_table.get_mut(&parent_id) {
            apply_dep_to_unit(
                parent,
                &child_id,
                dep.dep_kind,
                &dep.parent_unit,
                &dep.child_unit,
            );
            if !parent.common.unit.refs_by_name.contains(&child_id) {
                parent.common.unit.refs_by_name.push(child_id);
            }
        }
    }
}

/// Minimal implementation of systemd-getty-generator functionality.
/// Reads /proc/cmdline and creates serial-getty instances for kernel consoles.
pub fn generate_getty_units(
    unit_table: &mut HashMap<UnitId, Unit>,
    unit_dirs: &[PathBuf],
    dropins: &HashMap<String, Vec<(String, String)>>,
) {
    // Only run if /proc/cmdline exists (i.e., on a real Linux system)
    let cmdline = match std::fs::read_to_string("/proc/cmdline") {
        Ok(c) => c,
        Err(_) => return,
    };

    // Parse console= arguments from kernel command line
    let mut consoles: Vec<String> = Vec::new();
    for param in cmdline.split_whitespace() {
        if let Some(console) = param.strip_prefix("console=") {
            // console=ttyS0,115200 -> extract "ttyS0"
            let tty = console.split(',').next().unwrap_or(console);
            consoles.push(tty.to_owned());
        }
    }

    if consoles.is_empty() {
        return;
    }

    // Find the getty.target to add Wants dependencies
    let getty_target_id = UnitId {
        name: "getty.target".to_owned(),
        kind: UnitIdKind::Target,
    };

    for console in &consoles {
        // Only create serial-getty for serial consoles (ttyS*, ttyAMA*, ttyUSB*, etc.)
        // Skip virtual consoles (tty0, tty1, etc.)
        let is_serial = console.starts_with("ttyS")
            || console.starts_with("ttyAMA")
            || console.starts_with("ttyUSB")
            || console.starts_with("ttyACM")
            || console.starts_with("hvc");

        if !is_serial {
            continue;
        }

        let instance_name = format!("serial-getty@{}.service", console);
        let instance_id: UnitId = match instance_name.as_str().try_into() {
            Ok(id) => id,
            Err(_) => continue,
        };

        // Skip if already loaded
        if unit_table.contains_key(&instance_id) {
            continue;
        }

        // Try to instantiate from the serial-getty@.service template
        if let Some(unit) = instantiate_template(
            "serial-getty@.service",
            console,
            &instance_name,
            unit_dirs,
            dropins,
        ) {
            trace!("Getty generator: created {}", instance_name);
            unit_table.insert(unit.id.clone(), unit);

            // Add to getty.target's Wants
            if let Some(getty_target) = unit_table.get_mut(&getty_target_id) {
                if !getty_target
                    .common
                    .dependencies
                    .wants
                    .contains(&instance_id)
                {
                    getty_target
                        .common
                        .dependencies
                        .wants
                        .push(instance_id.clone());
                }
                if !getty_target.common.unit.refs_by_name.contains(&instance_id) {
                    getty_target.common.unit.refs_by_name.push(instance_id);
                }
            }
        } else {
            warn!(
                "Getty generator: could not instantiate serial-getty@.service for {}",
                console
            );
        }
    }
}

/// Minimal implementation of systemd-fstab-generator functionality.
///
/// Reads `/etc/fstab` and creates `.mount` units for each entry that doesn't
/// already have a corresponding mount unit in the unit table.  This is critical
/// for NixOS where mount points like `/run/wrappers` are defined in fstab and
/// services use `RequiresMountsFor=` to depend on them.  Without these
/// synthetic mount units the dependencies are silently dropped, which can lead
/// to race conditions (e.g. `suid-sgid-wrappers.service` starting before its
/// mount point is ready, breaking PAM/NSS and causing "Authentication service
/// cannot retrieve authentication info").
pub fn generate_fstab_mount_units(unit_table: &mut HashMap<UnitId, Unit>) {
    let fstab_path = std::path::Path::new("/etc/fstab");
    if !fstab_path.exists() {
        return;
    }

    let contents = match std::fs::read_to_string(fstab_path) {
        Ok(c) => c,
        Err(e) => {
            warn!("fstab generator: failed to read /etc/fstab: {}", e);
            return;
        }
    };

    let local_target_id = UnitId {
        name: "local-fs.target".to_owned(),
        kind: UnitIdKind::Target,
    };
    let remote_target_id = UnitId {
        name: "remote-fs.target".to_owned(),
        kind: UnitIdKind::Target,
    };

    for line in contents.lines() {
        let line = line.trim();
        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // fstab format: <device> <mountpoint> <fstype> <options> <dump> <pass>
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 3 {
            continue;
        }

        let device = fields[0];
        let mountpoint = fields[1];
        let fstype = fields[2];
        let options = if fields.len() > 3 {
            fields[3]
        } else {
            "defaults"
        };
        // fields[4] = dump, fields[5] = pass (ignored)

        // Handle swap entries — generate .swap units instead of .mount units
        if fstype == "swap" || mountpoint == "none" || mountpoint == "swap" {
            let unit_name = path_to_swap_unit_name(device);
            let unit_id = UnitId {
                name: unit_name.clone(),
                kind: UnitIdKind::Swap,
            };
            if unit_table.contains_key(&unit_id) {
                trace!(
                    "fstab generator: skipping {} — swap unit already exists",
                    unit_name
                );
                continue;
            }

            let opt_list: Vec<&str> = options.split(',').collect();
            let is_noauto = opt_list.contains(&"noauto");
            let is_nofail = opt_list.contains(&"nofail");

            // Parse priority from options (pri=N) or fstab fields
            let mut priority: Option<i32> = None;
            let mut swap_options: Vec<&str> = Vec::new();
            for opt in &opt_list {
                let opt = opt.trim();
                if let Some(pri_str) = opt.strip_prefix("pri=") {
                    if let Ok(p) = pri_str.parse::<i32>() {
                        priority = Some(p);
                    }
                } else if !matches!(opt, "noauto" | "auto" | "nofail" | "defaults" | "sw" | "")
                    && !opt.starts_with("x-systemd.")
                    && !opt.starts_with("comment=")
                {
                    swap_options.push(opt);
                }
            }

            let options_str = if swap_options.is_empty() {
                None
            } else {
                Some(swap_options.join(","))
            };

            let swap_target_id = UnitId {
                name: "swap.target".to_owned(),
                kind: UnitIdKind::Target,
            };

            let before = vec![swap_target_id.clone()];
            let mut wanted_by = Vec::new();
            let mut required_by = Vec::new();

            if !is_noauto {
                if is_nofail {
                    wanted_by.push(swap_target_id.clone());
                } else {
                    required_by.push(swap_target_id.clone());
                }
            }

            let mut refs_by_name = Vec::new();
            refs_by_name.extend(before.iter().cloned());
            refs_by_name.extend(wanted_by.iter().cloned());
            refs_by_name.extend(required_by.iter().cloned());

            let unit = Unit {
                id: unit_id.clone(),
                common: Common {
                    status: RwLock::new(UnitStatus::NeverStarted),
                    unit: UnitConfig {
                        description: format!("Swap unit for {} (from /etc/fstab)", device),
                        documentation: Vec::new(),
                        fragment_path: None,
                        refs_by_name,
                        default_dependencies: true,
                        ignore_on_isolate: false,
                        conditions: Vec::new(),
                        assertions: Vec::new(),
                        success_action: Default::default(),
                        failure_action: Default::default(),
                        job_timeout_action: Default::default(),
                        job_timeout_sec: None,
                        allow_isolate: false,
                        refuse_manual_start: false,
                        refuse_manual_stop: false,
                        on_success: Vec::new(),
                        on_success_job_mode: Default::default(),
                        on_failure: Vec::new(),
                        on_failure_job_mode: Default::default(),
                        start_limit_interval_sec: None,
                        start_limit_burst: None,
                        start_limit_action: Default::default(),
                        aliases: Vec::new(),
                        default_instance: None,
                        loaded_at: std::time::SystemTime::now(),
                        loaded_dropin_files: Vec::new(),
                        joins_namespace_of: Vec::new(),
                    },
                    dependencies: Dependencies {
                        wants: Vec::new(),
                        wanted_by,
                        requires: Vec::new(),
                        required_by,
                        conflicts: Vec::new(),
                        conflicted_by: Vec::new(),
                        before,
                        after: Vec::new(),
                        part_of: Vec::new(),
                        part_of_by: Vec::new(),
                        binds_to: Vec::new(),
                        bound_by: Vec::new(),
                        upholds: Vec::new(),
                        upheld_by: Vec::new(),
                        propagates_stop_to: Vec::new(),
                    },
                    timestamps: RwLock::new(UnitTimestamps::default()),
                    n_restarts: std::sync::atomic::AtomicU64::new(0),
                    deactivation_in_progress: std::sync::atomic::AtomicBool::new(false),
                    deactivation_irreversible: std::sync::atomic::AtomicBool::new(false),
                    start_requested_during_deactivation: std::sync::atomic::AtomicBool::new(false),
                    invocation_id: std::sync::Mutex::new(String::new()),
                },
                specific: Specific::Swap(SwapSpecific {
                    conf: SwapConfig {
                        what: device.to_owned(),
                        priority,
                        options: options_str,
                        timeout_sec: None,
                    },
                    state: RwLock::new(SwapState {
                        common: CommonState::default(),
                    }),
                }),
            };

            trace!(
                "fstab generator: created {} for swap device {}",
                unit_name, device
            );
            unit_table.insert(unit_id, unit);
            continue;
        }

        let unit_name = path_to_mount_unit_name(mountpoint);

        // Skip if a mount unit already exists (explicitly defined unit file
        // takes precedence over fstab-generated ones)
        let unit_id = UnitId {
            name: unit_name.clone(),
            kind: UnitIdKind::Mount,
        };
        if unit_table.contains_key(&unit_id) {
            trace!(
                "fstab generator: skipping {} — unit already exists",
                unit_name
            );
            continue;
        }

        // Determine if this is a "noauto" mount (should not be started
        // automatically).  Also detect "nofail" (failures are not fatal)
        // and "x-systemd.automount" (should create an automount unit — we
        // just skip those for now).
        let opt_list: Vec<&str> = options.split(',').collect();
        let is_noauto = opt_list.contains(&"noauto");
        let is_nofail = opt_list.contains(&"nofail");
        let _is_automount = opt_list.contains(&"x-systemd.automount");

        // Build the mount options string, filtering out fstab-only options
        let mount_options: Vec<&str> = opt_list
            .iter()
            .copied()
            .filter(|o| {
                !matches!(
                    *o,
                    "noauto"
                        | "auto"
                        | "nofail"
                        | "user"
                        | "nouser"
                        | "users"
                        | "group"
                        | "_netdev"
                        | "defaults"
                        | "x-systemd.automount"
                ) && !o.starts_with("comment=")
                    && !o.starts_with("x-systemd.")
            })
            .collect();

        let options_str = if mount_options.is_empty() {
            None
        } else {
            Some(mount_options.join(","))
        };

        // Determine whether this is a network or local filesystem
        let is_network = matches!(
            fstype,
            "nfs" | "nfs4" | "cifs" | "smbfs" | "ncpfs" | "glusterfs" | "ceph" | "fuse.sshfs"
        ) || opt_list.contains(&"_netdev");

        let target_id = if is_network {
            &remote_target_id
        } else {
            &local_target_id
        };

        // Build dependencies: Before= the appropriate fs target, and
        // the target Wants/Requires this mount.
        let mut before = vec![target_id.clone()];
        let mut wanted_by = Vec::new();
        let mut required_by = Vec::new();

        if !is_noauto {
            if is_nofail {
                wanted_by.push(target_id.clone());
            } else {
                required_by.push(target_id.clone());
            }
        }

        // Root mount should be before local-fs-pre.target
        if mountpoint == "/" {
            let pre_target = UnitId {
                name: "local-fs-pre.target".to_owned(),
                kind: UnitIdKind::Target,
            };
            before.push(pre_target);
        }

        let mut refs_by_name = Vec::new();
        refs_by_name.extend(before.iter().cloned());
        refs_by_name.extend(wanted_by.iter().cloned());
        refs_by_name.extend(required_by.iter().cloned());

        let unit = Unit {
            id: unit_id.clone(),
            common: Common {
                status: RwLock::new(UnitStatus::NeverStarted),
                unit: UnitConfig {
                    description: format!("Mount unit for {mountpoint} (from /etc/fstab)"),
                    documentation: Vec::new(),
                    fragment_path: None,
                    refs_by_name,
                    default_dependencies: true,
                    ignore_on_isolate: false,
                    conditions: Vec::new(),
                    assertions: Vec::new(),
                    success_action: Default::default(),
                    failure_action: Default::default(),
                    job_timeout_action: Default::default(),
                    job_timeout_sec: None,
                    allow_isolate: false,
                    refuse_manual_start: false,
                    refuse_manual_stop: false,
                    on_success: Vec::new(),
                    on_success_job_mode: Default::default(),
                    on_failure: Vec::new(),
                    on_failure_job_mode: Default::default(),
                    start_limit_interval_sec: None,
                    start_limit_burst: None,
                    start_limit_action: Default::default(),
                    aliases: Vec::new(),
                    default_instance: None,
                    loaded_at: std::time::SystemTime::now(),
                    loaded_dropin_files: Vec::new(),
                    joins_namespace_of: Vec::new(),
                },
                dependencies: Dependencies {
                    wants: Vec::new(),
                    wanted_by,
                    requires: Vec::new(),
                    required_by,
                    conflicts: Vec::new(),
                    conflicted_by: Vec::new(),
                    before,
                    after: Vec::new(),
                    part_of: Vec::new(),
                    part_of_by: Vec::new(),
                    binds_to: Vec::new(),
                    bound_by: Vec::new(),
                    upholds: Vec::new(),
                    upheld_by: Vec::new(),
                    propagates_stop_to: Vec::new(),
                },
                timestamps: RwLock::new(UnitTimestamps::default()),
                n_restarts: std::sync::atomic::AtomicU64::new(0),
                deactivation_in_progress: std::sync::atomic::AtomicBool::new(false),
                deactivation_irreversible: std::sync::atomic::AtomicBool::new(false),
                start_requested_during_deactivation: std::sync::atomic::AtomicBool::new(false),
                invocation_id: std::sync::Mutex::new(String::new()),
            },
            specific: Specific::Mount(MountSpecific {
                conf: MountConfig {
                    what: device.to_owned(),
                    where_: mountpoint.to_owned(),
                    fs_type: if fstype == "auto" {
                        None
                    } else {
                        Some(fstype.to_owned())
                    },
                    options: options_str,
                    sloppy_options: false,
                    lazy_unmount: false,
                    read_write_only: false,
                    force_unmount: false,
                    directory_mode: 0o755,
                    timeout_sec: None,
                },
                state: RwLock::new(MountState {
                    common: CommonState::default(),
                }),
            }),
        };

        trace!(
            "fstab generator: created {} for {} on {} (type={})",
            unit_name, device, mountpoint, fstype
        );
        unit_table.insert(unit_id, unit);
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────

/// Merge base unit file content with drop-in override contents.
fn merge_unit_contents(base: &str, overrides: &[(String, String)]) -> String {
    let mut result = base.to_owned();

    for (_filename, content) in overrides {
        result = merge_ini_sections(&result, content);
    }

    result
}

/// Merge two INI-format unit file contents, combining sections with the same name.
/// If both files have the same section, the settings from the override are appended
/// to the base section. If an override sets a key to empty value, it clears the key.
/// Settings that accumulate (list-type) when specified multiple times.
/// For these keys, drop-in values are appended to existing values.
/// All other settings are scalar: the drop-in value replaces the base value
/// (last writer wins), matching real systemd's behavior.
fn is_list_setting(key: &str) -> bool {
    matches!(
        key,
        // Exec settings that accumulate
        "ExecStartPre"
            | "ExecStartPost"
            | "ExecStart"
            | "ExecStop"
            | "ExecStopPre"
            | "ExecStopPost"
            | "ExecReload"
            | "ExecCondition"
            // Environment accumulates (reset with empty value)
            | "Environment"
            | "EnvironmentFile"
            | "PassEnvironment"
            | "UnsetEnvironment"
            // Dependency directives accumulate
            | "Requires"
            | "Wants"
            | "After"
            | "Before"
            | "BindsTo"
            | "PartOf"
            | "Conflicts"
            | "Upholds"
            | "RequiredBy"
            | "WantedBy"
            | "Also"
            // Path/resource lists
            | "ReadWritePaths"
            | "ReadOnlyPaths"
            | "InaccessiblePaths"
            | "BindPaths"
            | "BindReadOnlyPaths"
            | "SupplementaryGroups"
            | "CapabilityBoundingSet"
            | "AmbientCapabilities"
            | "SystemCallFilter"
            | "SystemCallArchitectures"
            | "RestrictAddressFamilies"
            | "DeviceAllow"
            // Socket multi-value settings
            | "ListenStream"
            | "ListenDatagram"
            | "ListenSequentialPacket"
            | "ListenFIFO"
            | "ListenSpecial"
            | "ListenNetlink"
            | "ListenMessageQueue"
            | "ListenUSBFunction"
            // Credential lists
            | "ImportCredential"
            | "LoadCredential"
            | "LoadCredentialEncrypted"
            | "SetCredential"
            | "SetCredentialEncrypted"
            // Condition/Assert lists
            | "ConditionPathExists"
            | "ConditionPathExistsGlob"
            | "ConditionPathIsDirectory"
            | "ConditionPathIsSymbolicLink"
            | "ConditionPathIsMountPoint"
            | "ConditionPathIsReadWrite"
            | "ConditionDirectoryNotEmpty"
            | "ConditionFileNotEmpty"
            | "ConditionFileIsExecutable"
            | "ConditionNeedsUpdate"
            | "ConditionFirstBoot"
            | "ConditionSecurity"
            | "ConditionCapability"
            | "ConditionACPower"
            | "ConditionVirtualization"
            | "ConditionHost"
            | "ConditionKernelCommandLine"
            | "ConditionKernelVersion"
            | "ConditionEnvironment"
            | "ConditionUser"
            | "ConditionGroup"
            | "ConditionControlGroupController"
            | "ConditionMemory"
            | "ConditionCPUs"
            | "AssertPathExists"
            | "AssertPathExistsGlob"
            | "AssertPathIsDirectory"
            | "AssertPathIsSymbolicLink"
            | "AssertPathIsMountPoint"
            | "AssertPathIsReadWrite"
            | "AssertDirectoryNotEmpty"
            | "AssertFileNotEmpty"
            | "AssertFileIsExecutable"
            | "AssertNeedsUpdate"
            | "AssertFirstBoot"
            | "AssertSecurity"
            | "AssertCapability"
            | "AssertACPower"
            | "AssertVirtualization"
            | "AssertHost"
            | "AssertKernelCommandLine"
            | "AssertKernelVersion"
            | "AssertEnvironment"
            | "AssertUser"
            | "AssertGroup"
            | "AssertControlGroupController"
            | "AssertMemory"
            | "AssertCPUs"
            // Install section lists
            | "Alias"
            // State/runtime directory lists
            | "StateDirectory"
            | "RuntimeDirectory"
            | "LogsDirectory"
            | "ConfigurationDirectory"
            | "CacheDirectory"
    )
}

/// Public wrapper for `merge_ini_sections` for use by implicit slice drop-in application.
pub fn merge_ini_sections_pub(base: &str, overlay: &str) -> String {
    merge_ini_sections(base, overlay)
}

fn merge_ini_sections(base: &str, overlay: &str) -> String {
    let base_sections = parse_ini_to_sections(base);
    let overlay_sections = parse_ini_to_sections(overlay);

    // Start with base sections
    let mut merged_sections: Vec<(String, Vec<String>)> = Vec::new();
    let mut seen_sections: HashMap<String, usize> = HashMap::new();

    for (name, lines) in &base_sections {
        seen_sections.insert(name.clone(), merged_sections.len());
        merged_sections.push((name.clone(), lines.clone()));
    }

    // Merge overlay sections
    for (name, lines) in &overlay_sections {
        if let Some(&idx) = seen_sections.get(name) {
            // Merge into existing section
            for line in lines {
                let trimmed = line.trim();
                if let Some(pos) = trimmed.find('=') {
                    let key = trimmed[..pos].trim();
                    let value = trimmed[pos + 1..].trim();
                    if value.is_empty() {
                        // This is a reset: remove all previous lines with this key
                        merged_sections[idx].1.retain(|existing| {
                            let existing_trimmed = existing.trim();
                            if let Some(epos) = existing_trimmed.find('=') {
                                existing_trimmed[..epos].trim() != key
                            } else {
                                true
                            }
                        });
                        // Don't add the empty assignment itself
                        continue;
                    }

                    if !is_list_setting(key) {
                        // Scalar setting: remove the old value so the overlay
                        // replaces it (last writer wins, matching real systemd).
                        merged_sections[idx].1.retain(|existing| {
                            let existing_trimmed = existing.trim();
                            if let Some(epos) = existing_trimmed.find('=') {
                                existing_trimmed[..epos].trim() != key
                            } else {
                                true
                            }
                        });
                    }
                }
                merged_sections[idx].1.push(line.clone());
            }
        } else {
            // New section from overlay
            seen_sections.insert(name.clone(), merged_sections.len());
            merged_sections.push((name.clone(), lines.clone()));
        }
    }

    // Reconstruct the INI content
    let mut result = String::new();
    for (name, lines) in &merged_sections {
        result.push_str(name);
        result.push('\n');
        for line in lines {
            result.push_str(line);
            result.push('\n');
        }
        result.push('\n');
    }

    result
}

/// Parse INI content into a list of (section_name, lines) pairs.
fn parse_ini_to_sections(content: &str) -> Vec<(String, Vec<String>)> {
    let mut sections = Vec::new();
    let mut current_section = String::new();
    let mut current_lines = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if !current_section.is_empty() {
                sections.push((current_section.clone(), current_lines.clone()));
                current_lines.clear();
            }
            current_section = trimmed.to_owned();
        } else if !current_section.is_empty() {
            // Skip empty lines and comments
            if !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with(';') {
                current_lines.push(trimmed.to_owned());
            }
        }
    }

    if !current_section.is_empty() {
        sections.push((current_section, current_lines));
    }

    sections
}

/// Find a unit file by name across all unit directories.
fn find_unit_file_in_dirs(unit_dirs: &[PathBuf], unit_name: &str) -> Option<PathBuf> {
    for dir in unit_dirs {
        let candidate = dir.join(unit_name);
        if candidate.exists() && !candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dep_dir_name() {
        assert_eq!(
            parse_dep_dir_name("multi-user.target.wants"),
            Some(("multi-user.target".to_owned(), DepDirKind::Wants))
        );
        assert_eq!(
            parse_dep_dir_name("sysinit.target.requires"),
            Some(("sysinit.target".to_owned(), DepDirKind::Requires))
        );
        assert_eq!(
            parse_dep_dir_name("my-uphold.service.upholds"),
            Some(("my-uphold.service".to_owned(), DepDirKind::Upholds))
        );
        assert_eq!(parse_dep_dir_name("some-directory"), None);
    }

    #[test]
    fn test_parse_dropin_dir_name() {
        assert_eq!(
            parse_dropin_dir_name("getty@.service.d"),
            Some("getty@.service".to_owned())
        );
        assert_eq!(
            parse_dropin_dir_name("sshd.service.d"),
            Some("sshd.service".to_owned())
        );
        assert_eq!(parse_dropin_dir_name("some-directory"), None);
    }

    #[test]
    fn test_parse_template_instance() {
        assert_eq!(
            parse_template_instance("serial-getty@ttyS0.service"),
            Some(("serial-getty@.service".to_owned(), "ttyS0".to_owned()))
        );
        assert_eq!(
            parse_template_instance("getty@tty1.service"),
            Some(("getty@.service".to_owned(), "tty1".to_owned()))
        );
        // Template itself (no instance)
        assert_eq!(parse_template_instance("getty@.service"), None);
        // No @ at all
        assert_eq!(parse_template_instance("sshd.service"), None);
    }

    #[test]
    fn test_resolve_specifiers() {
        let result = resolve_specifiers(
            "TTYPath=/dev/%I\nDescription=Getty on %I\nUtmpIdentifier=%I",
            "serial-getty@ttyS0.service",
            "ttyS0",
        );
        assert_eq!(
            result,
            "TTYPath=/dev/ttyS0\nDescription=Getty on ttyS0\nUtmpIdentifier=ttyS0"
        );
    }

    #[test]
    fn test_resolve_specifiers_all() {
        let result = resolve_specifiers("%n %N %i %I %p %P %%", "foo@bar.service", "bar");
        // %N strips the suffix (prefix+instance without .service),
        // %I unescapes the instance ("bar" has nothing to unescape),
        // %P unescapes the prefix ("foo" has nothing to unescape).
        assert_eq!(result, "foo@bar.service foo@bar bar bar foo foo %");
    }

    #[test]
    fn test_merge_ini_sections_scalar_override() {
        let base = "[Service]\nPrivateDevices=yes\nType=notify\n";
        let overlay = "[Service]\nPrivateDevices=false\n";
        let merged = merge_ini_sections(base, overlay);
        // Scalar setting: overlay replaces base (last writer wins)
        assert!(
            merged.contains("PrivateDevices=false"),
            "overlay value should be present: {merged}"
        );
        assert!(
            !merged.contains("PrivateDevices=yes"),
            "base value should be removed: {merged}"
        );
        // Other settings should be preserved
        assert!(
            merged.contains("Type=notify"),
            "unrelated setting should be kept: {merged}"
        );
    }

    #[test]
    fn test_merge_ini_sections_list_accumulates() {
        let base = "[Service]\nEnvironment=FOO=1\n";
        let overlay = "[Service]\nEnvironment=BAR=2\n";
        let merged = merge_ini_sections(base, overlay);
        // List setting: both values should be present
        assert!(
            merged.contains("Environment=FOO=1"),
            "base list value should be kept: {merged}"
        );
        assert!(
            merged.contains("Environment=BAR=2"),
            "overlay list value should be added: {merged}"
        );
    }

    #[test]
    fn test_merge_ini_sections_basic() {
        let base = "[Unit]\nDescription=Base\n\n[Service]\nExecStart=/bin/true\n";
        let overlay = "[Service]\nEnvironment=FOO=bar\n";
        let merged = merge_ini_sections(base, overlay);

        assert!(merged.contains("Description=Base"));
        assert!(merged.contains("ExecStart=/bin/true"));
        assert!(merged.contains("Environment=FOO=bar"));
    }

    #[test]
    fn test_merge_ini_sections_reset() {
        let base = "[Service]\nExecStart=/bin/old\n";
        let overlay = "[Service]\nExecStart=\nExecStart=/bin/new\n";
        let merged = merge_ini_sections(base, overlay);

        assert!(!merged.contains("/bin/old"));
        assert!(merged.contains("ExecStart=/bin/new"));
    }

    #[test]
    fn test_merge_ini_sections_new_section() {
        let base = "[Unit]\nDescription=Test\n";
        let overlay = "[Install]\nWantedBy=multi-user.target\n";
        let merged = merge_ini_sections(base, overlay);

        assert!(merged.contains("[Unit]"));
        assert!(merged.contains("Description=Test"));
        assert!(merged.contains("[Install]"));
        assert!(merged.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn test_is_unit_file() {
        assert!(is_unit_file("sshd.service"));
        assert!(is_unit_file("sshd.socket"));
        assert!(is_unit_file("multi-user.target"));
        assert!(is_unit_file("user.slice"));
        assert!(is_unit_file("dev-sda1.device"));
        assert!(!is_unit_file("override.conf"));
        assert!(!is_unit_file("README.md"));
    }

    #[test]
    fn test_is_template_unit() {
        // Templates have an empty instance between '@' and the extension
        assert!(is_template_unit("getty@.service"));
        assert!(is_template_unit("serial-getty@.service"));
        assert!(is_template_unit("modprobe@.service"));
        assert!(is_template_unit("container-getty@.service"));
        assert!(is_template_unit("systemd-backlight@.service"));
        assert!(is_template_unit("autovt@.service"));
        assert!(is_template_unit("capsule@.service"));

        // Concrete instances are NOT templates
        assert!(!is_template_unit("getty@tty1.service"));
        assert!(!is_template_unit("serial-getty@ttyS0.service"));
        assert!(!is_template_unit("modprobe@dm_mod.service"));
        assert!(!is_template_unit("autovt@tty1.service"));

        // Regular units are NOT templates
        assert!(!is_template_unit("sshd.service"));
        assert!(!is_template_unit("multi-user.target"));
        assert!(!is_template_unit("dbus.socket"));
        assert!(!is_template_unit("user.slice"));

        // Edge cases
        assert!(!is_template_unit("no-extension"));
        assert!(!is_template_unit(""));
    }

    #[test]
    fn test_has_unresolved_specifiers() {
        assert!(has_unresolved_specifiers("%i"));
        assert!(has_unresolved_specifiers("%I"));
        assert!(has_unresolved_specifiers("%n"));
        assert!(has_unresolved_specifiers("foo%ibar"));
        assert!(has_unresolved_specifiers("systemd-journald@%i"));
        assert!(!has_unresolved_specifiers("ttyS0"));
        assert!(!has_unresolved_specifiers("tty1"));
        assert!(!has_unresolved_specifiers("dm_mod"));
        assert!(!has_unresolved_specifiers("%%")); // escaped percent
        assert!(!has_unresolved_specifiers(""));
        assert!(!has_unresolved_specifiers("no-specifiers-here"));
    }

    fn make_unit_id(name: &str) -> UnitId {
        let kind = if name.ends_with(".service") {
            UnitIdKind::Service
        } else if name.ends_with(".target") {
            UnitIdKind::Target
        } else if name.ends_with(".socket") {
            UnitIdKind::Socket
        } else {
            UnitIdKind::Service
        };
        UnitId {
            kind,
            name: name.to_string(),
        }
    }

    #[test]
    fn test_merge_dep_vec() {
        let a = make_unit_id("a.service");
        let b = make_unit_id("b.service");
        let c = make_unit_id("c.service");
        let skip = make_unit_id("skip.service");

        let mut target = vec![a.clone()];
        let source = vec![a.clone(), b.clone(), c.clone(), skip.clone()];
        merge_dep_vec(&mut target, &source, &skip);

        assert_eq!(target.len(), 3);
        assert!(target.contains(&a));
        assert!(target.contains(&b));
        assert!(target.contains(&c));
        assert!(!target.contains(&skip));
    }

    #[test]
    fn test_merge_dep_vec_empty_source() {
        let a = make_unit_id("a.service");
        let skip = make_unit_id("skip.service");

        let mut target = vec![a.clone()];
        merge_dep_vec(&mut target, &[], &skip);
        assert_eq!(target.len(), 1);
    }

    #[test]
    fn test_rewrite_id_vec() {
        let alias = make_unit_id("dbus-org.freedesktop.resolve1.service");
        let real = make_unit_id("systemd-resolved.service");
        let other = make_unit_id("other.service");

        let mut ids = vec![alias.clone(), other.clone()];
        let redirect: HashMap<UnitId, UnitId> =
            [(alias.clone(), real.clone())].into_iter().collect();

        rewrite_id_vec(&mut ids, &redirect);

        assert!(ids.contains(&real));
        assert!(ids.contains(&other));
        assert!(!ids.contains(&alias));
    }

    #[test]
    fn test_rewrite_id_vec_deduplicates() {
        let alias = make_unit_id("dbus-org.freedesktop.resolve1.service");
        let real = make_unit_id("systemd-resolved.service");

        // Both alias and real are present — after rewrite, alias becomes real,
        // and dedup should collapse them.
        let mut ids = vec![real.clone(), alias.clone()];
        let redirect: HashMap<UnitId, UnitId> =
            [(alias.clone(), real.clone())].into_iter().collect();

        rewrite_id_vec(&mut ids, &redirect);

        assert_eq!(ids.iter().filter(|id| **id == real).count(), 1);
        assert!(!ids.contains(&alias));
    }

    #[test]
    fn test_rewrite_id_vec_no_matches() {
        let a = make_unit_id("a.service");
        let b = make_unit_id("b.service");

        let mut ids = vec![a.clone(), b.clone()];
        let redirect: HashMap<UnitId, UnitId> = HashMap::new();

        rewrite_id_vec(&mut ids, &redirect);

        assert_eq!(ids, vec![a, b]);
    }

    /// Helper to create a minimal Unit for testing alias resolution.
    fn make_test_unit(name: &str) -> Unit {
        let id = make_unit_id(name);
        Unit {
            id: id.clone(),
            common: Common {
                unit: UnitConfig {
                    description: name.to_string(),
                    documentation: vec![],
                    fragment_path: None,
                    refs_by_name: vec![],
                    default_dependencies: true,
                    conditions: vec![],
                    assertions: vec![],
                    success_action: crate::units::UnitAction::None,
                    failure_action: crate::units::UnitAction::None,
                    aliases: vec![],
                    ignore_on_isolate: false,
                    default_instance: None,
                    allow_isolate: false,
                    job_timeout_sec: None,
                    job_timeout_action: crate::units::UnitAction::None,
                    refuse_manual_start: false,
                    refuse_manual_stop: false,
                    on_success: vec![],
                    on_success_job_mode: crate::units::OnFailureJobMode::Replace,
                    on_failure: vec![],
                    on_failure_job_mode: crate::units::OnFailureJobMode::Replace,
                    start_limit_interval_sec: None,
                    start_limit_burst: None,
                    start_limit_action: crate::units::UnitAction::None,
                    loaded_at: std::time::SystemTime::now(),
                    loaded_dropin_files: Vec::new(),
                    joins_namespace_of: Vec::new(),
                },
                dependencies: Dependencies {
                    wants: vec![],
                    wanted_by: vec![],
                    requires: vec![],
                    required_by: vec![],
                    conflicts: vec![],
                    conflicted_by: vec![],
                    before: vec![],
                    after: vec![],
                    part_of: vec![],
                    part_of_by: vec![],
                    binds_to: vec![],
                    bound_by: vec![],
                    upholds: vec![],
                    upheld_by: vec![],
                    propagates_stop_to: vec![],
                },
                status: RwLock::new(UnitStatus::NeverStarted),
                timestamps: RwLock::new(UnitTimestamps::default()),
                n_restarts: std::sync::atomic::AtomicU64::new(0),
                deactivation_in_progress: std::sync::atomic::AtomicBool::new(false),
                deactivation_irreversible: std::sync::atomic::AtomicBool::new(false),
                start_requested_during_deactivation: std::sync::atomic::AtomicBool::new(false),
                invocation_id: std::sync::Mutex::new(String::new()),
            },
            specific: Specific::Target(crate::units::TargetSpecific {
                state: RwLock::new(crate::units::TargetState {
                    common: CommonState {
                        up_since: None,
                        restart_count: 0,
                        start_timestamps: Vec::new(),
                        freezer_state: crate::units::FreezerState::Running,
                    },
                }),
            }),
        }
    }

    #[test]
    fn test_template_instance_alias_rename() {
        // Simulate: autovt@.service is a template alias for getty@.service.
        // autovt@tty1.service was instantiated. After resolve, only
        // getty@tty1.service should remain.

        let alias_instance_id = make_unit_id("autovt@tty1.service");
        let getty_target_id = make_unit_id("getty.target");

        let mut alias_instance = make_test_unit("autovt@tty1.service");
        // Give it a wanted_by so we can verify deps are carried over
        alias_instance
            .common
            .dependencies
            .wanted_by
            .push(getty_target_id.clone());

        let mut getty_target = make_test_unit("getty.target");
        getty_target
            .common
            .dependencies
            .wants
            .push(alias_instance_id.clone());

        let mut unit_table: HashMap<UnitId, Unit> = HashMap::new();
        unit_table.insert(alias_instance_id.clone(), alias_instance);
        unit_table.insert(getty_target_id.clone(), getty_target);

        // Simulate what Phase 1b does: detect that autovt@tty1.service
        // should become getty@tty1.service.
        // We'll call the logic directly by testing the rename path.

        let real_instance_name = "getty@tty1.service";
        let real_instance_id = make_unit_id(real_instance_name);

        // Since getty@tty1.service doesn't exist, the alias should be renamed
        assert!(!unit_table.contains_key(&real_instance_id));
        assert!(unit_table.contains_key(&alias_instance_id));

        // Rename: remove alias, insert as real
        if let Some(mut unit) = unit_table.remove(&alias_instance_id) {
            unit.common
                .unit
                .aliases
                .push(alias_instance_id.name.clone());
            unit.id = real_instance_id.clone();
            unit_table.insert(real_instance_id.clone(), unit);
        }

        // Rewrite references
        let redirect: HashMap<UnitId, UnitId> =
            [(alias_instance_id.clone(), real_instance_id.clone())]
                .into_iter()
                .collect();
        for unit in unit_table.values_mut() {
            rewrite_id_vec(&mut unit.common.dependencies.wants, &redirect);
            rewrite_id_vec(&mut unit.common.dependencies.wanted_by, &redirect);
        }

        // Verify: alias instance removed, real instance exists
        assert!(!unit_table.contains_key(&alias_instance_id));
        assert!(unit_table.contains_key(&real_instance_id));

        // Verify: renamed unit has old name as alias
        let real = unit_table.get(&real_instance_id).unwrap();
        assert!(
            real.common
                .unit
                .aliases
                .contains(&"autovt@tty1.service".to_string())
        );

        // Verify: getty.target now wants getty@tty1.service (rewritten)
        let target = unit_table.get(&getty_target_id).unwrap();
        assert!(target.common.dependencies.wants.contains(&real_instance_id));
        assert!(
            !target
                .common
                .dependencies
                .wants
                .contains(&alias_instance_id)
        );
    }

    #[test]
    fn test_template_instance_alias_merge() {
        // Simulate: both autovt@tty1.service and getty@tty1.service exist.
        // After merge, only getty@tty1.service should remain with merged deps.

        let alias_instance_id = make_unit_id("autovt@tty1.service");
        let real_instance_id = make_unit_id("getty@tty1.service");
        let getty_target_id = make_unit_id("getty.target");
        let some_target_id = make_unit_id("some.target");

        let mut alias_instance = make_test_unit("autovt@tty1.service");
        alias_instance
            .common
            .dependencies
            .wanted_by
            .push(getty_target_id.clone());

        let mut real_instance = make_test_unit("getty@tty1.service");
        real_instance
            .common
            .dependencies
            .wanted_by
            .push(some_target_id.clone());

        let mut unit_table: HashMap<UnitId, Unit> = HashMap::new();
        unit_table.insert(alias_instance_id.clone(), alias_instance);
        unit_table.insert(real_instance_id.clone(), real_instance);

        // Merge alias into real
        let alias_wanted_by = unit_table
            .get(&alias_instance_id)
            .unwrap()
            .common
            .dependencies
            .wanted_by
            .clone();
        if let Some(real_unit) = unit_table.get_mut(&real_instance_id) {
            merge_dep_vec(
                &mut real_unit.common.dependencies.wanted_by,
                &alias_wanted_by,
                &real_instance_id,
            );
            real_unit
                .common
                .unit
                .aliases
                .push("autovt@tty1.service".to_string());
        }
        unit_table.remove(&alias_instance_id);

        // Verify: only real remains
        assert!(!unit_table.contains_key(&alias_instance_id));
        assert!(unit_table.contains_key(&real_instance_id));

        // Verify: merged deps
        let real = unit_table.get(&real_instance_id).unwrap();
        assert!(
            real.common
                .dependencies
                .wanted_by
                .contains(&getty_target_id)
        );
        assert!(real.common.dependencies.wanted_by.contains(&some_target_id));
        assert!(
            real.common
                .unit
                .aliases
                .contains(&"autovt@tty1.service".to_string())
        );
    }

    #[test]
    fn test_resolve_symlink_aliases_removes_alias_unit() {
        // Simulate: dbus-org.freedesktop.resolve1.service is an alias for
        // systemd-resolved.service. Both are in the unit table. After
        // resolve_symlink_aliases, only the real unit should remain.

        // We can't easily create real symlinks in a test, so we test the
        // merge/remove/rewrite helpers directly and verify the logic.
        let real_id = make_unit_id("systemd-resolved.service");
        let alias_id = make_unit_id("dbus-org.freedesktop.resolve1.service");
        let dep_id = make_unit_id("network.target");

        let mut real_unit = make_test_unit("systemd-resolved.service");
        real_unit.common.dependencies.after.push(dep_id.clone());

        let mut alias_unit = make_test_unit("dbus-org.freedesktop.resolve1.service");
        alias_unit
            .common
            .dependencies
            .wanted_by
            .push(make_unit_id("multi-user.target"));

        let mut dep_unit = make_test_unit("network.target");
        dep_unit.common.dependencies.before.push(alias_id.clone());

        let mut unit_table: HashMap<UnitId, Unit> = HashMap::new();
        unit_table.insert(real_id.clone(), real_unit);
        unit_table.insert(alias_id.clone(), alias_unit);
        unit_table.insert(dep_id.clone(), dep_unit);

        // Simulate what resolve_symlink_aliases does internally:
        // Phase 1: merge alias deps into real, remove alias
        let alias_wanted_by = unit_table
            .get(&alias_id)
            .unwrap()
            .common
            .dependencies
            .wanted_by
            .clone();

        if let Some(real_unit) = unit_table.get_mut(&real_id) {
            merge_dep_vec(
                &mut real_unit.common.dependencies.wanted_by,
                &alias_wanted_by,
                &real_id,
            );
            real_unit
                .common
                .unit
                .aliases
                .push("dbus-org.freedesktop.resolve1.service".to_string());
        }
        unit_table.remove(&alias_id);

        // Phase 2: rewrite references
        let redirect: HashMap<UnitId, UnitId> =
            [(alias_id.clone(), real_id.clone())].into_iter().collect();
        for unit in unit_table.values_mut() {
            rewrite_id_vec(&mut unit.common.dependencies.before, &redirect);
            rewrite_id_vec(&mut unit.common.dependencies.after, &redirect);
            rewrite_id_vec(&mut unit.common.dependencies.wants, &redirect);
            rewrite_id_vec(&mut unit.common.dependencies.wanted_by, &redirect);
        }

        // Verify: alias removed from table
        assert!(!unit_table.contains_key(&alias_id));

        // Verify: real unit has the alias registered
        let real = unit_table.get(&real_id).unwrap();
        assert!(
            real.common
                .unit
                .aliases
                .contains(&"dbus-org.freedesktop.resolve1.service".to_string())
        );

        // Verify: alias's wanted_by was merged into real
        assert!(
            real.common
                .dependencies
                .wanted_by
                .contains(&make_unit_id("multi-user.target"))
        );

        // Verify: dep_unit's before now points to real, not alias
        let dep = unit_table.get(&dep_id).unwrap();
        assert!(dep.common.dependencies.before.contains(&real_id));
        assert!(!dep.common.dependencies.before.contains(&alias_id));
    }
}
