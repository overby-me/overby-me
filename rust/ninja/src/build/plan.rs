//! Target resolution + dependency-order scheduling.
//!
//! Produces a topological order over the edges needed to satisfy the
//! requested build targets. Phony edges are kept in the plan so the
//! runner can correctly count them as "done" without dispatching work.

use crate::cli::Options;
use crate::graph::State;
use std::collections::HashSet;

/// Pick which targets to build:
///   1. explicit CLI targets, if any
///   2. else `default` statements
///   3. else every output node not consumed as input (the "root" set)
pub fn resolve_targets(state: &State, opts: &Options) -> Vec<String> {
    if !opts.targets.is_empty() {
        return opts.targets.clone();
    }
    if !state.defaults.is_empty() {
        return state.defaults.clone();
    }
    let mut consumed = HashSet::new();
    for e in &state.edges {
        for i in e
            .inputs
            .iter()
            .chain(&e.implicit_inputs)
            .chain(&e.order_only_inputs)
        {
            consumed.insert(i.clone());
        }
    }
    let mut roots = Vec::new();
    for e in &state.edges {
        for o in &e.outputs {
            if !consumed.contains(o) {
                roots.push(o.clone());
            }
        }
    }
    roots
}

/// Topologically order the edges needed to produce `targets`. Returns
/// edge indices; phony edges are kept (the runner skips dispatch for
/// them) so accounting and dependency wiring stay consistent.
pub fn build_plan(state: &State, targets: &[String]) -> Result<Vec<usize>, String> {
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    let mut on_stack = HashSet::new();
    for t in targets {
        visit(state, t, &mut order, &mut seen, &mut on_stack)?;
    }
    Ok(order)
}

fn visit(
    state: &State,
    target: &str,
    order: &mut Vec<usize>,
    seen: &mut HashSet<usize>,
    on_stack: &mut HashSet<usize>,
) -> Result<(), String> {
    let Some(&idx) = state.producers.get(target) else {
        return Ok(()); // source file
    };
    if !seen.insert(idx) {
        return Ok(());
    }
    if !on_stack.insert(idx) {
        return Err(format!("dependency cycle through '{target}'"));
    }
    let edge = &state.edges[idx];
    for inp in edge
        .inputs
        .iter()
        .chain(&edge.implicit_inputs)
        .chain(&edge.order_only_inputs)
    {
        visit(state, inp, order, seen, on_stack)?;
    }
    on_stack.remove(&idx);
    order.push(idx);
    Ok(())
}
