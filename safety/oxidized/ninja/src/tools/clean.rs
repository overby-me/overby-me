//! `-t clean` — remove built outputs.
//!
//! Mirrors reference ninja's behavior:
//!   - With no targets, clean every output of every non-phony edge.
//!   - With `-r`, treat each target as a rule name and clean every
//!     output of every edge using that rule.
//!   - Otherwise treat targets as output paths and recursively clean
//!     their producing edge plus the entire dependency subtree.
//!   - For each cleaned edge, also remove its `depfile` and `rspfile`
//!     when present (gcc-deps style or response-file rules).
//!   - `-g` (clean generator outputs) is parsed but currently a no-op
//!     since we don't track the `generator = 1` rule binding for
//!     each edge yet — every edge is treated as non-generator.
//!   - Print `Cleaning... N files.` exactly like upstream so callers
//!     can grep for the line.
//!
//! Files that don't exist on disk are silently skipped — that's the
//! "already clean" path.

use crate::build::expand_in_edge_pub;
use crate::graph::State;
use std::collections::HashSet;

#[derive(Default)]
struct Args {
    /// Clean files marked as ninja generator output. Currently a
    /// no-op — see module doc.
    generator: bool,
    /// Treat positional arguments as rule names instead of paths.
    by_rule: bool,
    targets: Vec<String>,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut out = Args::default();
    for a in args {
        match a.as_str() {
            "-g" => out.generator = true,
            "-r" => out.by_rule = true,
            s if s.starts_with('-') => return Err(format!("clean: unknown flag {s}")),
            s => out.targets.push(s.to_string()),
        }
    }
    Ok(out)
}

pub fn run(state: &State, args: &[String]) -> Result<u8, String> {
    let parsed = parse_args(args)?;

    // Collect the edge index set we're going to clean.
    let mut edge_set: HashSet<usize> = HashSet::new();
    if parsed.by_rule {
        if parsed.targets.is_empty() {
            return Err("clean: -r requires at least one rule name".into());
        }
        let wanted: HashSet<&str> = parsed.targets.iter().map(|s| s.as_str()).collect();
        for (idx, edge) in state.edges.iter().enumerate() {
            if wanted.contains(edge.rule.as_str()) {
                edge_set.insert(idx);
            }
        }
    } else if parsed.targets.is_empty() {
        // Clean every non-phony, non-generator edge. Generator edges
        // (those whose rule sets `generator = 1`) only get cleaned
        // when the user passes `-g`, matching reference ninja so a
        // bare `ninja -t clean` doesn't wipe out CMake's RERUN_CMAKE
        // outputs (`build.ninja`, `cmake_install.cmake`).
        for (idx, edge) in state.edges.iter().enumerate() {
            if edge.is_phony() {
                continue;
            }
            if !parsed.generator && is_generator(state, edge) {
                continue;
            }
            edge_set.insert(idx);
        }
    } else {
        // Walk the dependency subtree of each target so transitively
        // produced files are cleaned too — matches upstream's
        // "clean target X cleans X's whole subtree" behavior.
        for t in &parsed.targets {
            collect_subtree(state, t, &mut edge_set);
        }
    }

    let mut removed = 0usize;
    for &idx in &edge_set {
        let edge = &state.edges[idx];
        if edge.is_phony() {
            continue;
        }
        for out in edge.outputs.iter().chain(&edge.implicit_outputs) {
            if try_remove(out) {
                removed += 1;
            }
        }
        // Depfile / rspfile cleanup. Both may contain edge-context
        // expansions ($out, etc.), so resolve them via the same
        // expander the runner uses.
        if let Some(rule) = state.rules.get(&edge.rule) {
            let deps_mode = edge
                .bindings
                .get("deps")
                .or_else(|| rule.bindings.get("deps"))
                .map(|s| s.as_str())
                .unwrap_or("");
            for key in ["depfile", "rspfile"] {
                if key == "depfile" && (deps_mode == "gcc" || deps_mode == "msvc") {
                    continue;
                }
                let raw = edge
                    .bindings
                    .get(key)
                    .or_else(|| rule.bindings.get(key))
                    .cloned();
                if let Some(raw) = raw {
                    let path = expand_in_edge_pub(state, edge, &raw);
                    if !path.is_empty() && try_remove(&path) {
                        removed += 1;
                    }
                }
            }
        }
    }

    println!("Cleaning... {removed} files.");
    Ok(0)
}

fn try_remove(path: &str) -> bool {
    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => false,
    }
}

/// True if `edge` is a generator edge — i.e. the rule it points at
/// has `generator = 1`. Edge-level overrides win over the rule
/// definition, matching ninja’s standard layered binding lookup.
fn is_generator(state: &State, edge: &crate::graph::Edge) -> bool {
    let from_edge = edge
        .bindings
        .get("generator")
        .map(|v| matches!(v.as_str(), "1" | "true"));
    if let Some(b) = from_edge {
        return b;
    }
    state
        .rules
        .get(&edge.rule)
        .and_then(|r| r.bindings.get("generator"))
        .map(|v| matches!(v.as_str(), "1" | "true"))
        .unwrap_or(false)
}

fn collect_subtree(state: &State, target: &str, edge_set: &mut HashSet<usize>) {
    let Some(&idx) = state.producers.get(target) else {
        return;
    };
    if !edge_set.insert(idx) {
        return;
    }
    let edge = &state.edges[idx];
    for inp in edge
        .inputs
        .iter()
        .chain(&edge.implicit_inputs)
        .chain(&edge.order_only_inputs)
    {
        collect_subtree(state, inp, edge_set);
    }
}
