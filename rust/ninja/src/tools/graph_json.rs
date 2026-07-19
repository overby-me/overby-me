//! `-t graph-json` — emit the entire build graph as JSON.
//!
//! Unlike `-t compdb-targets` (which filters to edges reaching named targets
//! and emits only directory/command/file/output), this dumps EVERY edge with
//! its fully-expanded command plus all the metadata a per-edge Nix lowering
//! (`nix/lib/ninja`) needs: explicit/implicit/order-only inputs and outputs,
//! the expanded depfile and rspfile paths + content, the deps mode, and the
//! restat/generator flags and pool. Phony edges are included (with
//! `"phony": true`) so alias/`default` resolution is possible downstream.
//!
//! Output is a single JSON object: `{ "defaults": [...], "edges": [ {...} ] }`.
//!
//! With no arguments the whole graph is dumped. Given one or more target
//! outputs as positional arguments, only the edges that produce those targets
//! and their transitive dependencies are emitted — essential at CMake scale
//! (tens of thousands of edges) when only one target's subtree is wanted.

use crate::graph::{Edge, State};
use crate::manifest::expand;
use std::collections::HashSet;

pub fn run(state: &State, args: &[String]) -> Result<u8, String> {
    let targets: Vec<&String> = args.iter().filter(|s| !s.starts_with('-')).collect();

    // Select edge indices to emit: all, or the transitive subtree of `targets`.
    let indices: Vec<usize> = if targets.is_empty() {
        (0..state.edges.len()).collect()
    } else {
        let mut sel = Vec::new();
        let mut seen = HashSet::new();
        for t in &targets {
            collect(state, t, &mut sel, &mut seen);
        }
        sel
    };

    let mut out = String::from("{\n  \"defaults\": [");
    for (i, d) in state.defaults.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push('"');
        out.push_str(&json_escape(d));
        out.push('"');
    }
    out.push_str("],\n  \"edges\": [\n");

    for (n, &idx) in indices.iter().enumerate() {
        emit_edge(state, &state.edges[idx], &mut out);
        if n + 1 < indices.len() {
            out.push(',');
        }
        out.push('\n');
    }

    out.push_str("  ]\n}\n");
    print!("{out}");
    Ok(0)
}

/// Depth-first collect the edge producing `target` and all its transitive
/// dependency edges (explicit, implicit, and order-only inputs), flattening
/// phony aliases so their real producers are included.
fn collect(state: &State, target: &str, out: &mut Vec<usize>, seen: &mut HashSet<usize>) {
    let Some(&idx) = state.producers.get(target) else {
        return;
    };
    if !seen.insert(idx) {
        return;
    }
    let edge = &state.edges[idx];
    for inp in edge
        .inputs
        .iter()
        .chain(&edge.implicit_inputs)
        .chain(&edge.order_only_inputs)
    {
        collect(state, inp, out, seen);
    }
    out.push(idx);
}

fn emit_edge(state: &State, edge: &Edge, out: &mut String) {
    let rule = state.rules.get(&edge.rule);
    let phony = edge.is_phony();

    // Path/command-like attributes go through edge-context expansion; flags and
    // enum-like attributes are taken raw.
    let command = lookup(state, edge, rule, "command", true);
    let depfile = lookup(state, edge, rule, "depfile", true);
    let deps = lookup(state, edge, rule, "deps", false);
    let rspfile = lookup(state, edge, rule, "rspfile", true);
    let rspfile_content = lookup(state, edge, rule, "rspfile_content", true);
    let pool = lookup(state, edge, rule, "pool", false);
    let restat = lookup(state, edge, rule, "restat", false)
        .map(|v| truthy(&v))
        .unwrap_or(false);
    let generator = lookup(state, edge, rule, "generator", false)
        .map(|v| truthy(&v))
        .unwrap_or(false);

    out.push_str("    {\n");
    out.push_str(&format!(
        "      \"rule\": \"{}\",\n",
        json_escape(&edge.rule)
    ));
    out.push_str(&format!("      \"phony\": {phony},\n"));
    arr(out, "outputs", &edge.outputs);
    arr(out, "implicit_outputs", &edge.implicit_outputs);
    arr(out, "inputs", &edge.inputs);
    arr(out, "implicit_inputs", &edge.implicit_inputs);
    arr(out, "order_only_inputs", &edge.order_only_inputs);
    strfield(out, "command", &command);
    strfield(out, "depfile", &depfile);
    strfield(out, "deps", &deps);
    strfield(out, "rspfile", &rspfile);
    strfield(out, "rspfile_content", &rspfile_content);
    strfield(out, "pool", &pool);
    out.push_str(&format!("      \"restat\": {restat},\n"));
    out.push_str(&format!("      \"generator\": {generator}\n"));
    out.push_str("    }");
}

/// Look up an attribute in edge bindings, then rule bindings. When `do_expand`
/// is set, the value is run through edge-context expansion (`$in`/`$out`/vars).
fn lookup(
    state: &State,
    edge: &Edge,
    rule: Option<&crate::graph::Rule>,
    name: &str,
    do_expand: bool,
) -> Option<String> {
    let raw = edge
        .bindings
        .get(name)
        .cloned()
        .or_else(|| rule.and_then(|r| r.bindings.get(name).cloned()))?;
    Some(if do_expand {
        expand_in_edge(state, edge, &raw)
    } else {
        raw
    })
}

fn truthy(v: &str) -> bool {
    !v.is_empty() && v != "0" && v != "false"
}

fn arr(out: &mut String, key: &str, xs: &[String]) {
    out.push_str(&format!("      \"{key}\": ["));
    for (i, x) in xs.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push('"');
        out.push_str(&json_escape(x));
        out.push('"');
    }
    out.push_str("],\n");
}

fn strfield(out: &mut String, key: &str, v: &Option<String>) {
    match v {
        Some(s) => out.push_str(&format!("      \"{key}\": \"{}\",\n", json_escape(s))),
        None => out.push_str(&format!("      \"{key}\": null,\n")),
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Edge-context expansion identical to the build runner's, duplicated here to
/// keep tools/ self-contained (same approach as compdb_targets).
fn expand_in_edge(state: &State, edge: &Edge, value: &str) -> String {
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
                return Some(expand(v, &|n2| state.bindings.get(n2).cloned()));
            }
            state.bindings.get(name).cloned()
        }
    })
}
