//! `-t compdb-targets` — emit a JSON compilation database for the build
//! edges that produce the requested targets and their transitive deps.
//!
//! The output is a JSON array of objects with `directory`, `command`,
//! `file`, and `output` keys. `file` is the *first explicit* input of the
//! edge; `output` is the *first explicit* output. Edges are emitted in
//! depth-first dependency order from the named targets.

use crate::graph::State;
use crate::manifest::expand;
use std::collections::HashSet;

const USAGE: &str = "usage: ninja -t compdb [-hx] target [targets]\n\nopti\
ons:\n  -h     display this help message\n  -x     expand @rspfile style \
response file invocations\n";

pub fn run(state: &State, args: &[String]) -> Result<u8, String> {
    // Filter out tool-specific flags. Targets are positional args that
    // don't start with '-'.
    let targets: Vec<&String> = args.iter().filter(|s| !s.starts_with('-')).collect();
    if targets.is_empty() {
        // The reference binary writes both the error and the usage to
        // stdout (the test reads check_output -> stdout only).
        print!("ninja: error: compdb-targets expects the name of at least one target\n{USAGE}");
        return Ok(1);
    }
    // Validate each target: it must be an existing build statement output.
    for t in &targets {
        let known_output = state.producers.contains_key(t.as_str());
        let known_anywhere = known_output
            || state.edges.iter().any(|e| {
                e.inputs.iter().any(|i| i == *t)
                    || e.implicit_inputs.iter().any(|i| i == *t)
                    || e.order_only_inputs.iter().any(|i| i == *t)
            });
        if !known_output {
            if known_anywhere {
                println!(
                    "ninja: fatal: '{t}' is not a target (i.e. it is not an output of any `build` statement)"
                );
            } else {
                println!("ninja: fatal: unknown target '{t}'");
            }
            return Ok(1);
        }
    }

    let dir = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let mut edges_to_emit = Vec::new();
    let mut seen = HashSet::new();
    for t in &targets {
        collect(state, t, &mut edges_to_emit, &mut seen);
    }

    let mut out = String::from("[\n");
    for (i, &edge_idx) in edges_to_emit.iter().enumerate() {
        let edge = &state.edges[edge_idx];
        let rule = state.rules.get(&edge.rule);
        let command = rule
            .and_then(|r| r.bindings.get("command"))
            .map(|c| expand_in_edge(state, edge, c))
            .unwrap_or_default();
        let file = edge.inputs.first().cloned().unwrap_or_default();
        let output = edge.outputs.first().cloned().unwrap_or_default();
        out.push_str("  {\n");
        out.push_str(&format!("    \"directory\": \"{}\",\n", json_escape(&dir)));
        out.push_str(&format!(
            "    \"command\": \"{}\",\n",
            json_escape(&command)
        ));
        out.push_str(&format!("    \"file\": \"{}\",\n", json_escape(&file)));
        out.push_str(&format!("    \"output\": \"{}\"\n", json_escape(&output)));
        out.push_str("  }");
        if i + 1 < edges_to_emit.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("]\n");
    print!("{out}");
    Ok(0)
}

fn collect(state: &State, target: &str, out: &mut Vec<usize>, seen: &mut HashSet<usize>) {
    let Some(&idx) = state.producers.get(target) else {
        return;
    };
    let edge = &state.edges[idx];
    if edge.is_phony() {
        for inp in edge
            .inputs
            .iter()
            .chain(&edge.implicit_inputs)
            .chain(&edge.order_only_inputs)
        {
            collect(state, inp, out, seen);
        }
        return;
    }
    // Recurse into inputs first so dependency order is preserved.
    for inp in &edge.inputs {
        collect(state, inp, out, seen);
    }
    if seen.insert(idx) {
        out.push(idx);
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

/// Edge-context expansion identical to the build runner's, duplicated
/// here to keep tools/ self-contained without pulling in `build::`.
fn expand_in_edge(state: &State, edge: &crate::graph::Edge, value: &str) -> String {
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
