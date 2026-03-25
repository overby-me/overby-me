use log::trace;

use crate::runtime_info::{RuntimeInfo, UnitTable};
use crate::units;

use std::collections::HashMap;
use std::convert::TryInto;
use std::fs;
use std::path::PathBuf;

fn find_new_unit_path(unit_dirs: &[PathBuf], find_name: &str) -> Result<Option<PathBuf>, String> {
    for dir in unit_dirs {
        let read_dir = match fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(e) => {
                return Err(format!("Error while opening dir {dir:?}: {e}"));
            }
        };
        for entry in read_dir {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let entry_name = entry.file_name();
            // Use symlink_metadata so we can handle symlinks explicitly.
            // NixOS unit files in /etc/systemd/system/ are symlinks into
            // the Nix store; entry.metadata() (which follows symlinks) can
            // fail on complex symlink chains, so we match on the raw type.
            let symlink_meta = match entry.path().symlink_metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if symlink_meta.file_type().is_symlink() {
                if entry_name == find_name {
                    return Ok(Some(entry.path()));
                }
                continue;
            }
            if symlink_meta.file_type().is_file() && entry_name == find_name {
                return Ok(Some(entry.path()));
            }
            if symlink_meta.file_type().is_dir()
                && let Some(p) = find_new_unit_path(&[entry.path()], find_name)?
            {
                return Ok(Some(p));
            }
        }
    }

    Ok(None)
}

/// Collect drop-in overrides for a unit from all unit directories.
/// Scans for type-level, hierarchical prefix, and exact name `.d/` dirs.
fn collect_dropins_for_unit(
    unit_dirs: &[PathBuf],
    unit_name: &str,
) -> HashMap<String, Vec<(String, String)>> {
    use crate::units::loading::directory_deps::collect_dropin_entries;

    let mut dropins: HashMap<String, Vec<(String, String)>> = HashMap::new();

    // Build the list of drop-in directory names to search for:
    // 1. Type-level (e.g., "service.d" for any .service unit)
    // 2. Hierarchical prefixes (e.g., "a-.service.d", "a-b-.service.d")
    // 3. Exact unit name (e.g., "a-b-c.service.d")
    let mut dropin_dir_keys: Vec<String> = Vec::new();

    // Type-level
    if let Some(dot_pos) = unit_name.rfind('.') {
        let type_suffix = &unit_name[dot_pos + 1..]; // e.g., "service"
        dropin_dir_keys.push(type_suffix.to_owned());

        // Hierarchical prefixes
        let base = &unit_name[..dot_pos]; // e.g., "a-b-c"
        let suffix = &unit_name[dot_pos..]; // e.g., ".service"
        let parts: Vec<&str> = base.split('-').collect();
        for i in 1..parts.len() {
            let prefix = parts[..i].join("-");
            dropin_dir_keys.push(format!("{prefix}-{suffix}")); // e.g., "a-.service"
        }
    }
    // Exact name
    dropin_dir_keys.push(unit_name.to_owned());

    for dir in unit_dirs {
        for key in &dropin_dir_keys {
            let dropin_dir = dir.join(format!("{key}.d"));
            if dropin_dir.is_dir() {
                collect_dropin_entries(&dropin_dir, key, &mut dropins);
            }
        }
    }

    dropins
}

/// Scan unit directories for symlinks that resolve to the same unit as
/// `find_name`. Checks both canonical path equality and symlink target
/// name matching (for template instances where the target doesn't exist
/// as a real file).
pub fn find_symlink_aliases(
    unit_dirs: &[PathBuf],
    unit_path: &std::path::Path,
    find_name: &str,
) -> Vec<String> {
    let canonical = fs::canonicalize(unit_path).ok();
    let mut aliases = Vec::new();
    for dir in unit_dirs {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == find_name {
                continue;
            }
            // Only consider symlinks
            let meta = match entry.path().symlink_metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if !meta.file_type().is_symlink() {
                continue;
            }
            // Check by canonical path (works for regular files)
            if let Some(ref canonical) = canonical
                && let Ok(c) = fs::canonicalize(entry.path())
                && c == *canonical
                && !aliases.contains(&name)
            {
                aliases.push(name);
                continue;
            }
            // Check by symlink target name (works for template instances
            // where the target file doesn't exist on disk)
            if let Ok(target) = fs::read_link(entry.path()) {
                let target_name = target
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                if target_name == find_name && !aliases.contains(&name) {
                    aliases.push(name);
                }
            }
        }
    }
    aliases
}

/// Loads a unit with a given name. It searches all paths recursively until it finds a file with a matching name.
/// Also scans for and applies drop-in overrides from `.d/` directories.
pub fn load_new_unit(unit_dirs: &[PathBuf], find_name: &str) -> Result<units::Unit, String> {
    if let Some(unit_path) = find_new_unit_path(unit_dirs, find_name)? {
        let content = fs::read_to_string(&unit_path).map_err(|e| {
            format!(
                "{}",
                units::ParsingError::new(
                    units::ParsingErrorReason::from(Box::new(e)),
                    unit_path.clone()
                )
            )
        })?;

        // Collect and apply drop-in overrides
        let dropins = collect_dropins_for_unit(unit_dirs, find_name);
        let final_content = if dropins.is_empty() {
            content
        } else {
            use crate::units::loading::directory_deps::collect_applicable_dropins_pub;
            let overrides = collect_applicable_dropins_pub(find_name, &dropins);
            if overrides.is_empty() {
                content
            } else {
                trace!(
                    "Applying {} drop-in(s) to on-demand loaded unit {}",
                    overrides.len(),
                    find_name
                );
                crate::units::loading::directory_deps::merge_unit_contents_pub(&content, &overrides)
            }
        };

        // Resolve specifiers (%n, %i, %p, etc.) before parsing
        let final_content = crate::units::loading::directory_deps::resolve_specifiers(
            &final_content,
            find_name,
            "",
        );

        let parsed = units::parse_file(&final_content)
            .map_err(|e| format!("{}", units::ParsingError::new(e, unit_path.clone())))?;
        let mut unit: units::Unit = if find_name.ends_with(".service") {
            units::parse_service(parsed, &unit_path)
                .map_err(|e| format!("{}", units::ParsingError::new(e, unit_path.clone())))?
                .try_into()?
        } else if find_name.ends_with(".socket") {
            units::parse_socket(parsed, &unit_path)
                .map_err(|e| format!("{}", units::ParsingError::new(e, unit_path.clone())))?
                .try_into()?
        } else if find_name.ends_with(".target") {
            units::parse_target(parsed, &unit_path)
                .map_err(|e| format!("{}", units::ParsingError::new(e, unit_path.clone())))?
                .try_into()?
        } else if find_name.ends_with(".slice") {
            units::parse_slice(parsed, &unit_path)
                .map_err(|e| format!("{}", units::ParsingError::new(e, unit_path.clone())))?
                .try_into()?
        } else {
            return Err(format!("File suffix not recognized for file {unit_path:?}"));
        };

        // Discover filesystem-level symlink aliases (e.g. test15-b.service → test15-a.service)
        let aliases = find_symlink_aliases(unit_dirs, &unit_path, find_name);
        for alias in aliases {
            if !unit.common.unit.aliases.contains(&alias) {
                trace!("Discovered symlink alias for {}: {}", find_name, alias);
                unit.common.unit.aliases.push(alias);
            }
        }

        Ok(unit)
    } else {
        Err(format!("Cannot find unit file for unit: {find_name}"))
    }
}

// check that all names referenced in the new units exist either in the old units
// or in the new units
fn check_all_names_exist(
    new_units: &HashMap<units::UnitId, units::Unit>,
    unit_table_locked: &UnitTable,
) -> Result<(), String> {
    let mut names_needed = Vec::new();
    for new_unit in new_units.values() {
        names_needed.extend(new_unit.common.unit.refs_by_name.iter().cloned());
    }

    let mut names_needed: std::collections::HashMap<_, _> =
        names_needed.iter().map(|name| (name, ())).collect();

    for unit in unit_table_locked.values() {
        for new_unit in new_units.values() {
            if unit.id == new_unit.id {
                return Err(format!("Id {} exists already", new_unit.id));
            }
            if unit.id.name == new_unit.id.name {
                return Err(format!("Name {} exists already", new_unit.id.name));
            }
        }
        if names_needed.contains_key(&unit.id) {
            names_needed.remove(&unit.id).unwrap();
        }
    }
    for unit in new_units.values() {
        if names_needed.contains_key(&unit.id) {
            names_needed.remove(&unit.id).unwrap();
        }
    }
    if !names_needed.is_empty() {
        return Err(format!(
            "Names referenced by unit but not found in the known set of units: {:?}",
            names_needed.keys().collect::<Vec<_>>()
        ));
    }
    Ok(())
}

/// Inserts a single unit without checking that all referenced dependencies exist.
/// Used for on-demand unit loading (e.g. `systemctl restart` for a unit that wasn't
/// part of the initial boot dependency graph). Missing dependency references are
/// silently ignored — the unit is inserted and wired up to whatever units are
/// already present in the table.
pub fn insert_new_unit_lenient(mut unit: units::Unit, run_info: &mut RuntimeInfo) {
    let new_id = unit.id.clone();
    let unit_table = &mut run_info.unit_table;

    // Wire up bidirectional dependency relations with existing units.
    // Direction 1: new unit references existing units → update existing units.
    // Direction 2: existing units reference new unit → update new unit.
    for existing in unit_table.values_mut() {
        // Direction 1: new → existing
        if unit.common.dependencies.after.contains(&existing.id) {
            existing.common.dependencies.before.push(new_id.clone());
        }
        if unit.common.dependencies.before.contains(&existing.id) {
            existing.common.dependencies.after.push(new_id.clone());
        }
        if unit.common.dependencies.requires.contains(&existing.id) {
            existing
                .common
                .dependencies
                .required_by
                .push(new_id.clone());
        }
        if unit.common.dependencies.wants.contains(&existing.id) {
            existing.common.dependencies.wanted_by.push(new_id.clone());
        }
        if unit.common.dependencies.required_by.contains(&existing.id) {
            existing.common.dependencies.requires.push(new_id.clone());
        }
        if unit.common.dependencies.wanted_by.contains(&existing.id) {
            existing.common.dependencies.wants.push(new_id.clone());
        }
        if unit.common.dependencies.conflicts.contains(&existing.id) {
            existing
                .common
                .dependencies
                .conflicted_by
                .push(new_id.clone());
        }
        if unit
            .common
            .dependencies
            .conflicted_by
            .contains(&existing.id)
        {
            existing.common.dependencies.conflicts.push(new_id.clone());
        }
        if unit.common.dependencies.binds_to.contains(&existing.id) {
            existing.common.dependencies.bound_by.push(new_id.clone());
        }
        if unit.common.dependencies.bound_by.contains(&existing.id) {
            existing.common.dependencies.binds_to.push(new_id.clone());
        }

        // Direction 2: existing → new (e.g., existing has Before=new_unit)
        if existing.common.dependencies.before.contains(&new_id) {
            unit.common.dependencies.after.push(existing.id.clone());
        }
        if existing.common.dependencies.after.contains(&new_id) {
            unit.common.dependencies.before.push(existing.id.clone());
        }
        if existing.common.dependencies.wants.contains(&new_id) {
            unit.common.dependencies.wanted_by.push(existing.id.clone());
        }
        if existing.common.dependencies.requires.contains(&new_id) {
            unit.common
                .dependencies
                .required_by
                .push(existing.id.clone());
        }
        if existing.common.dependencies.wanted_by.contains(&new_id) {
            unit.common.dependencies.wants.push(existing.id.clone());
        }
        if existing.common.dependencies.required_by.contains(&new_id) {
            unit.common.dependencies.requires.push(existing.id.clone());
        }
        if existing.common.dependencies.conflicts.contains(&new_id) {
            unit.common
                .dependencies
                .conflicted_by
                .push(existing.id.clone());
        }
        if existing.common.dependencies.conflicted_by.contains(&new_id) {
            unit.common.dependencies.conflicts.push(existing.id.clone());
        }
        if existing.common.dependencies.binds_to.contains(&new_id) {
            unit.common.dependencies.bound_by.push(existing.id.clone());
        }
        if existing.common.dependencies.bound_by.contains(&new_id) {
            unit.common.dependencies.binds_to.push(existing.id.clone());
        }
    }

    trace!("Leniently inserted unit: {}", unit.id.name);
    unit_table.insert(new_id, unit);
}

/// Inserts new units but first checks that the units referenced by the new units do exist
pub fn insert_new_units(new_units: UnitTable, run_info: &mut RuntimeInfo) -> Result<(), String> {
    // TODO check if new unit only refs existing units
    // TODO check if all ref'd units are not failed
    {
        let unit_table = &mut run_info.unit_table;
        trace!("Check all names exist");
        check_all_names_exist(&new_units, unit_table)?;

        for (new_id, new_unit) in new_units {
            trace!("Add new unit: {}", new_unit.id.name);
            // Setup relations of before <-> after / requires <-> requiredby
            for unit in unit_table.values_mut() {
                if new_unit.common.dependencies.after.contains(&unit.id) {
                    unit.common.dependencies.before.push(new_id.clone());
                }
                if new_unit.common.dependencies.before.contains(&unit.id) {
                    unit.common.dependencies.after.push(new_id.clone());
                }
                if new_unit.common.dependencies.requires.contains(&unit.id) {
                    unit.common.dependencies.required_by.push(new_id.clone());
                }
                if new_unit.common.dependencies.wants.contains(&unit.id) {
                    unit.common.dependencies.wanted_by.push(new_id.clone());
                }
                if new_unit.common.dependencies.required_by.contains(&unit.id) {
                    unit.common.dependencies.requires.push(new_id.clone());
                }
                if new_unit.common.dependencies.wanted_by.contains(&unit.id) {
                    unit.common.dependencies.wants.push(new_id.clone());
                }
                if new_unit.common.dependencies.conflicts.contains(&unit.id) {
                    unit.common.dependencies.conflicted_by.push(new_id.clone());
                }
                if new_unit
                    .common
                    .dependencies
                    .conflicted_by
                    .contains(&unit.id)
                {
                    unit.common.dependencies.conflicts.push(new_id.clone());
                }
                if new_unit.common.dependencies.binds_to.contains(&unit.id) {
                    unit.common.dependencies.bound_by.push(new_id.clone());
                }
                if new_unit.common.dependencies.bound_by.contains(&unit.id) {
                    unit.common.dependencies.binds_to.push(new_id.clone());
                }
            }
            unit_table.insert(new_id, new_unit);
        }
    }
    Ok(())
}
