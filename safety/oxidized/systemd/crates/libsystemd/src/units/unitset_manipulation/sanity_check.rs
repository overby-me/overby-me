use crate::units::{Unit, UnitId};
use log::warn;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Eq, PartialEq)]
pub enum SanityCheckError {
    Generic(String),
    CirclesFound(Vec<Vec<UnitId>>),
}

/// Detect dependency cycles and break them by removing ordering edges (before/after),
/// matching systemd's behavior of warning about cycles but continuing to boot.
/// Returns the list of cycles that were broken (empty if none found).
pub fn break_dependency_cycles(unit_table: &mut HashMap<UnitId, Unit>) -> Vec<Vec<UnitId>> {
    let mut all_broken_cycles = Vec::new();

    // Safety limit to prevent infinite loops in case of bugs
    let max_iterations = unit_table.len() * unit_table.len() + 1;
    let mut iteration = 0;

    // Repeat until no more cycles are found, since breaking one cycle
    // might not resolve all of them.
    loop {
        iteration += 1;
        if iteration > max_iterations {
            warn!(
                "Exceeded maximum iterations ({}) while breaking dependency cycles. \
                 This likely indicates a bug in cycle detection. Giving up.",
                max_iterations
            );
            break;
        }

        match sanity_check_dependencies(unit_table) {
            Ok(()) => break,
            Err(SanityCheckError::CirclesFound(cycles)) => {
                for circle in &cycles {
                    let unit_names: Vec<String> = circle.iter().map(|id| id.to_string()).collect();
                    warn!(
                        "Breaking ordering cycle by removing a dependency edge: [{}]",
                        unit_names.join(" -> ")
                    );

                    // Break the cycle by removing the ordering edge between
                    // the last unit and the first unit in the detected cycle.
                    // This is the "back edge" that closes the cycle.
                    if circle.len() >= 2 {
                        let from_id = &circle[circle.len() - 1];
                        let to_id = &circle[0];

                        // Remove "before" edge: from_id no longer needs to start before to_id
                        if let Some(from_unit) = unit_table.get_mut(from_id) {
                            from_unit
                                .common
                                .dependencies
                                .before
                                .retain(|id| id != to_id);
                        }

                        // Remove the corresponding "after" edge: to_id no longer needs to start after from_id
                        if let Some(to_unit) = unit_table.get_mut(to_id) {
                            to_unit.common.dependencies.after.retain(|id| id != from_id);
                        }

                        warn!("Removed ordering dependency: {} before {}", from_id, to_id);
                    } else if circle.len() == 1 {
                        // Self-loop: unit depends on itself
                        let self_id = &circle[0];
                        if let Some(unit) = unit_table.get_mut(self_id) {
                            unit.common.dependencies.before.retain(|id| id != self_id);
                            unit.common.dependencies.after.retain(|id| id != self_id);
                        }
                        warn!("Removed self-loop ordering dependency on {}", self_id);
                    }
                }
                all_broken_cycles.extend(cycles);
            }
            Err(SanityCheckError::Generic(msg)) => {
                warn!("Dependency sanity check error: {msg}");
                break;
            }
        }
    }

    all_broken_cycles
}

/// Check whether the unit dependency graph is a DAG (no cycles).
/// Returns Ok(()) if no cycles are found, or an error listing the cycles.
pub fn sanity_check_dependencies(
    unit_table: &HashMap<UnitId, Unit>,
) -> Result<(), SanityCheckError> {
    // Use Kahn's algorithm for topological sort to detect cycles.
    // When no node with in-degree 0 exists but unprocessed nodes remain,
    // we do a DFS from an arbitrary node to find the actual cycle path
    // instead of dumping all remaining nodes as a fake "cycle".
    let mut finished_ids = HashSet::new();
    let mut not_finished_ids: HashSet<_> = unit_table.keys().cloned().collect();
    let mut circles = Vec::new();

    loop {
        // If no nodes left -> no cycles
        if not_finished_ids.is_empty() {
            break;
        }

        // Find a node that has no incoming ordering edges from unfinished nodes.
        // Only count `after` dependencies that actually exist in the unit table,
        // since dependencies on non-existent units shouldn't block ordering.
        let root_id = not_finished_ids
            .iter()
            .find(|id| {
                let unit = unit_table.get(id).unwrap();
                let in_degree = unit
                    .common
                    .dependencies
                    .after
                    .iter()
                    .fold(0, |acc, dep_id| {
                        if finished_ids.contains(dep_id) || !unit_table.contains_key(dep_id) {
                            acc
                        } else {
                            acc + 1
                        }
                    });
                in_degree == 0
            })
            .cloned();

        let root_id = if let Some(id) = root_id {
            id
        } else {
            // No node with in-degree 0 exists, which means there's definitely a cycle.
            // Instead of dumping all remaining nodes as a fake "cycle", pick an arbitrary
            // node and do a DFS to find the actual cycle path.
            let arbitrary_id = not_finished_ids.iter().next().unwrap().clone();

            // Find a real cycle by following `before` edges with DFS
            if let Some(cycle) = find_cycle_from(&arbitrary_id, unit_table, &not_finished_ids) {
                circles.push(cycle);
            } else {
                // Shouldn't happen (we know there's a cycle since no node has in-degree 0),
                // but as a safety measure: force-finish this node to make progress.
                // This can happen if the cycle is only visible in `after` edges but not
                // reachable via `before` edges from this particular node.
                warn!(
                    "No cycle found via DFS from {}, but no root node exists. \
                     Force-finishing this node to make progress.",
                    arbitrary_id
                );
                finished_ids.insert(arbitrary_id.clone());
                not_finished_ids.remove(&arbitrary_id);
                continue;
            }
            break;
        };

        // Do DFS from root_id following `before` edges to detect cycles
        let mut visited_ids = Vec::new();
        if let Err(SanityCheckError::CirclesFound(new_circles)) = search_backedge(
            &root_id,
            unit_table,
            &mut visited_ids,
            &mut finished_ids,
            &mut not_finished_ids,
        ) {
            circles.extend(new_circles);
        }
    }

    if circles.is_empty() {
        Ok(())
    } else {
        Err(SanityCheckError::CirclesFound(circles))
    }
}

/// Find a cycle starting from the given node by following `before` edges.
/// Returns the cycle as a vec of UnitIds forming the cycle path, or None if no cycle is found.
fn find_cycle_from(
    start_id: &UnitId,
    unit_table: &HashMap<UnitId, Unit>,
    not_finished_ids: &HashSet<UnitId>,
) -> Option<Vec<UnitId>> {
    let mut visited = Vec::new();
    let mut visited_set = HashSet::new();
    find_cycle_dfs(
        start_id,
        unit_table,
        not_finished_ids,
        &mut visited,
        &mut visited_set,
    )
}

fn find_cycle_dfs(
    id: &UnitId,
    unit_table: &HashMap<UnitId, Unit>,
    not_finished_ids: &HashSet<UnitId>,
    visited: &mut Vec<UnitId>,
    visited_set: &mut HashSet<UnitId>,
) -> Option<Vec<UnitId>> {
    if visited_set.contains(id) {
        // Found a cycle - extract the cycle path
        let cycle_start = visited.iter().position(|v| v == id).unwrap();
        return Some(visited[cycle_start..].to_vec());
    }

    // Only follow edges to nodes that are still unfinished and exist in the table
    if !not_finished_ids.contains(id) || !unit_table.contains_key(id) {
        return None;
    }

    visited.push(id.clone());
    visited_set.insert(id.clone());

    let unit = unit_table.get(id).unwrap();
    for next_id in &unit.common.dependencies.before {
        // Only follow edges to units that exist and are unfinished
        if not_finished_ids.contains(next_id)
            && unit_table.contains_key(next_id)
            && let Some(cycle) =
                find_cycle_dfs(next_id, unit_table, not_finished_ids, visited, visited_set)
        {
            return Some(cycle);
        }
    }

    visited.pop();
    visited_set.remove(id);
    None
}

fn search_backedge(
    id: &UnitId,
    unit_table: &HashMap<UnitId, Unit>,
    visited_ids: &mut Vec<UnitId>,
    finished_ids: &mut HashSet<UnitId>,
    not_finished_ids: &mut HashSet<UnitId>,
) -> Result<(), SanityCheckError> {
    if finished_ids.contains(id) {
        return Ok(());
    }

    // Guard against edges pointing to units not in the table
    let Some(unit) = unit_table.get(id) else {
        return Ok(());
    };

    if visited_ids.contains(id) {
        let mut circle_start_idx = 0;
        for _ in 0..visited_ids.len() {
            if visited_ids[circle_start_idx] == *id {
                break;
            }
            circle_start_idx += 1;
        }
        let circle_ids = visited_ids[circle_start_idx..].to_vec();
        for circleid in &circle_ids {
            finished_ids.insert(circleid.clone());
            not_finished_ids.remove(circleid);
        }

        return Err(SanityCheckError::CirclesFound(vec![circle_ids]));
    }
    visited_ids.push(id.clone());

    for next_id in &unit.common.dependencies.before {
        // Only traverse edges to units that exist in the table
        if unit_table.contains_key(next_id) {
            let res = search_backedge(
                next_id,
                unit_table,
                visited_ids,
                finished_ids,
                not_finished_ids,
            );
            res?;
        }
    }
    visited_ids.pop();
    finished_ids.insert(id.clone());
    not_finished_ids.remove(id);

    Ok(())
}
