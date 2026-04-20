//! Edge-context variable expansion.
//!
//! Resolves `$in`, `$out`, plus any per-edge / per-rule / file-scope
//! binding referenced by a rule's `command`, `description`, `depfile`,
//! `rspfile`, etc.

use crate::graph::{Edge, State};
use crate::manifest::expand;

/// Look up `name` in the standard layered scope:
///   1. synthesized `$in` / `$out`
///   2. per-edge bindings
///   3. rule bindings (recursively expanded against file scope)
///   4. file-scope bindings
pub fn expand_in_edge(state: &State, edge: &Edge, value: &str) -> String {
    let rule = state.rules.get(&edge.rule);
    let in_str = edge.inputs.join(" ");
    let out_str = edge.outputs.join(" ");
    expand(value, &|name| match name {
        "in" => Some(in_str.clone()),
        "out" => Some(out_str.clone()),
        _ => {
            if let Some(v) = edge.bindings.get(name) {
                return Some(v.clone());
            }
            if let Some(r) = rule
                && let Some(v) = r.bindings.get(name)
            {
                // Recursive expansion against file scope only.
                return Some(expand(v, &|n2| state.bindings.get(n2).cloned()));
            }
            state.bindings.get(name).cloned()
        }
    })
}

/// Look up `key` in `edge.bindings` first, falling back to `rule.bindings`.
/// Returns the raw (unexpanded) value.
pub fn lookup_either(edge: &Edge, rule: &crate::graph::Rule, key: &str) -> Option<String> {
    edge.bindings
        .get(key)
        .or_else(|| rule.bindings.get(key))
        .cloned()
}
