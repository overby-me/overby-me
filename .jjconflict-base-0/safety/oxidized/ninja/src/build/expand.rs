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
    // `$in_newline` is the newline-separated form of `$in`. CMake's Ninja
    // generator puts a link edge's object files ONLY in the response file, via
    // `rspfile_content = $in_newline ...`; without this the rsp is written empty
    // and the link fails with "ld: no object files specified" (seen on the one
    // ~900-object link, libsystem_kernel_firstpass).
    let in_newline_str = edge.inputs.join("\n");
    let out_str = edge.outputs.join(" ");
    expand(value, &|name| match name {
        "in" => Some(in_str.clone()),
        "in_newline" => Some(in_newline_str.clone()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Edge, State};
    use std::collections::HashMap;

    fn edge(inputs: &[&str]) -> Edge {
        Edge {
            rule: "link".to_string(),
            outputs: vec!["libfoo.dylib".to_string()],
            implicit_outputs: vec![],
            inputs: inputs.iter().map(|s| s.to_string()).collect(),
            implicit_inputs: vec![],
            order_only_inputs: vec![],
            bindings: HashMap::new(),
            dyndep: None,
        }
    }

    #[test]
    fn in_is_space_separated_and_in_newline_is_newline_separated() {
        let state = State::default();
        let e = edge(&["a.o", "b.o", "c.o"]);
        assert_eq!(expand_in_edge(&state, &e, "$in"), "a.o b.o c.o");
        assert_eq!(expand_in_edge(&state, &e, "$in_newline"), "a.o\nb.o\nc.o");
    }

    #[test]
    fn in_newline_uses_only_explicit_inputs() {
        // CMake's linker rsp uses `rspfile_content = $in_newline ...`; implicit
        // and order-only inputs must stay out of it, matching `$in`.
        let state = State::default();
        let mut e = edge(&["a.o", "b.o"]);
        e.implicit_inputs = vec!["header.h".to_string()];
        e.order_only_inputs = vec!["gen_stamp".to_string()];
        assert_eq!(expand_in_edge(&state, &e, "$in_newline"), "a.o\nb.o");
    }
}
