//! `-t inputs` — list every input file feeding the named targets.
//!
//! Modes:
//!   - default: alphabetical, shell-escaped output
//!   - `--no-shell-escape`: alphabetical, no quoting
//!   - `--dependency-order`: dependency-order traversal (no sorting)
//!   - `--print0`: NUL-separate items instead of `\n`
//!
//! Phony output paths are not part of the result set per upstream rules,
//! but their inputs are still recursed into.

use crate::graph::State;
use std::collections::HashSet;

#[derive(Default)]
struct Args {
    dependency_order: bool,
    no_shell_escape: bool,
    print0: bool,
    targets: Vec<String>,
}

pub fn run(state: &State, args: &[String]) -> Result<u8, String> {
    let parsed = parse_args(args)?;
    let mut results = Vec::new();
    let mut added = HashSet::new();
    let mut visited = HashSet::new();
    for t in &parsed.targets {
        gather(
            state,
            t,
            &mut results,
            &mut added,
            &mut visited,
            parsed.dependency_order,
        );
    }
    if !parsed.no_shell_escape {
        results = results.into_iter().map(shell_quote).collect();
    }
    if !parsed.dependency_order {
        // Sort *after* quoting so quoted forms order naturally relative
        // to unquoted ones (matches ninja's behavior).
        results.sort();
    }
    let sep = if parsed.print0 { '\0' } else { '\n' };
    let mut out = String::new();
    for r in &results {
        out.push_str(r);
        out.push(sep);
    }
    print!("{out}");
    Ok(0)
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    for arg in args {
        match arg.as_str() {
            "--dependency-order" => a.dependency_order = true,
            "--no-shell-escape" => a.no_shell_escape = true,
            "--print0" => a.print0 = true,
            s if s.starts_with('-') => return Err(format!("inputs: unknown flag {s}")),
            _ => a.targets.push(arg.clone()),
        }
    }
    Ok(a)
}

/// Recursively collect inputs of `target`. Behavior:
/// - The target itself is *not* added (the user asked for its *inputs*).
/// - Inputs that are produced by a phony edge are *not* added; their
///   producer's inputs are recursed into instead.
/// - Inputs without a producer (source files) ARE added.
/// - Inputs produced by a non-phony edge are added AND recursed.
///
/// `added` deduplicates the output list. `visited` deduplicates recursion
/// independently — a node may need to be recursed past even when it
/// already appeared in the output (e.g. when reached again through a
/// different path).
fn gather(
    state: &State,
    target: &str,
    out: &mut Vec<String>,
    added: &mut HashSet<String>,
    visited: &mut HashSet<String>,
    dep_order: bool,
) {
    if !visited.insert(target.to_string()) {
        return;
    }
    let Some(&idx) = state.producers.get(target) else {
        return;
    };
    let edge = &state.edges[idx];
    for inp in edge
        .inputs
        .iter()
        .chain(&edge.implicit_inputs)
        .chain(&edge.order_only_inputs)
    {
        let inp_phony = state
            .producers
            .get(inp)
            .map(|&i| state.edges[i].is_phony())
            .unwrap_or(false);
        // Pre-order add (later sorted alphabetically anyway).
        if !dep_order && !inp_phony && added.insert(inp.clone()) {
            out.push(inp.clone());
        }
        gather(state, inp, out, added, visited, dep_order);
        // Post-order add for dependency-order: leaves first, then the
        // edges that consume them.
        if dep_order && !inp_phony && added.insert(inp.clone()) {
            out.push(inp.clone());
        }
    }
}

/// POSIX-shell-escape `s` if it contains characters that would be
/// re-interpreted by sh. Matches what ninja's `shell_escape` does.
fn shell_quote(s: String) -> String {
    let needs_quote = s.chars().any(|c| {
        !(c.is_ascii_alphanumeric()
            || matches!(c, '_' | '-' | '.' | '/' | ':' | '@' | '%' | '+' | '='))
    });
    if !needs_quote {
        return s;
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}
