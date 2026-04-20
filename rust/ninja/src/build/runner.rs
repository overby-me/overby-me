//! Per-edge subprocess execution and the parallel scheduler.
//!
//! The scheduler is dependency-driven: at any moment it dispatches every
//! ready edge (all of its inputs already produced or absent from the
//! plan) up to a `-j N` cap. Edge stdout/stderr is captured and handed
//! to the `Status` printer in completion order so logs interleave
//! correctly.
//!
//! Pools are not yet enforced; the `console` pool needs special handling
//! and lands in a later phase.

use super::expand::{expand_in_edge, lookup_either};
use super::plan::{build_plan, resolve_targets};
use crate::cli::Options;
use crate::graph::{Edge, State};
use crate::status::{Mode, Status};
use std::collections::HashSet;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

pub fn run(state: &State, opts: &Options) -> Result<u8, String> {
    let targets = resolve_targets(state, opts);
    if targets.is_empty() {
        if !opts.quiet {
            println!("ninja: no work to do.");
        }
        return Ok(0);
    }
    // Surface unknown explicit targets the way ninja does — an error to
    // stdout, exit 1. Source files (no producer) are accepted only when
    // they're listed as inputs of some edge.
    for t in &targets {
        if !state.producers.contains_key(t) {
            let known = state.edges.iter().any(|e| {
                e.inputs.iter().any(|i| i == t)
                    || e.implicit_inputs.iter().any(|i| i == t)
                    || e.order_only_inputs.iter().any(|i| i == t)
            });
            if !known {
                println!("ninja: error: unknown target '{t}'");
                return Ok(1);
            }
        }
    }

    let plan = build_plan(state, &targets)?;
    if plan.is_empty() {
        if !opts.quiet {
            println!("ninja: no work to do.");
        }
        return Ok(0);
    }

    let real_count = plan.iter().filter(|&&i| !state.edges[i].is_phony()).count();
    let mode = Mode::detect();
    let mut status = Status::new(mode, opts.quiet, opts.verbose, real_count);
    schedule(state, opts, &plan, &mut status)
}

/// Dependency-driven scheduler. Walks the topologically-ordered plan and
/// keeps up to `-j N` edges in flight at once. Phony edges complete
/// instantly without spawning a subprocess.
fn schedule(
    state: &State,
    opts: &Options,
    plan: &[usize],
    status: &mut Status,
) -> Result<u8, String> {
    let jobs = opts.jobs_count().max(1);

    // Map edge index → its dependencies (inputs that have a producer in
    // the plan). We use this to detect when an edge becomes ready.
    let in_plan: HashSet<usize> = plan.iter().copied().collect();
    let mut deps: Vec<Vec<usize>> = vec![Vec::new(); plan.len()];
    let mut idx_in_plan = std::collections::HashMap::with_capacity(plan.len());
    for (slot, &edge_idx) in plan.iter().enumerate() {
        idx_in_plan.insert(edge_idx, slot);
    }
    for (slot, &edge_idx) in plan.iter().enumerate() {
        let edge = &state.edges[edge_idx];
        for inp in edge
            .inputs
            .iter()
            .chain(&edge.implicit_inputs)
            .chain(&edge.order_only_inputs)
        {
            if let Some(&prod) = state.producers.get(inp) {
                if in_plan.contains(&prod) {
                    if let Some(&dep_slot) = idx_in_plan.get(&prod) {
                        deps[slot].push(dep_slot);
                    }
                }
            }
        }
    }

    let mut remaining: Vec<usize> = (0..plan.len()).map(|s| deps[s].len()).collect();
    let mut done = vec![false; plan.len()];
    let mut started = vec![false; plan.len()];
    let mut in_flight = 0usize;
    let (tx, rx) = mpsc::channel::<EdgeOutcome>();
    let mut hard_failure: Option<u8> = None;
    let mut interrupted = false;

    loop {
        // Dispatch every ready edge up to the parallelism cap. We stop
        // launching new work once a hard failure or interruption fires
        // — let in-flight jobs drain so their output is preserved.
        if hard_failure.is_none() && !interrupted {
            for slot in 0..plan.len() {
                if started[slot] || remaining[slot] > 0 {
                    continue;
                }
                if in_flight >= jobs {
                    break;
                }
                started[slot] = true;
                let edge_idx = plan[slot];
                let edge = state.edges[edge_idx].clone();
                if edge.is_phony() {
                    // Phony "completes" instantly; mark done inline so we
                    // can dispatch its dependents on this same loop turn.
                    done[slot] = true;
                    for s2 in 0..plan.len() {
                        if deps[s2].contains(&slot) && !done[s2] {
                            remaining[s2] = remaining[s2].saturating_sub(1);
                        }
                    }
                    continue;
                }
                in_flight += 1;
                let prepared = match prepare(state, &edge, opts) {
                    Ok(p) => p,
                    Err(e) => return Err(e),
                };
                status.build_started(&prepared.shown);
                let tx = tx.clone();
                thread::spawn(move || {
                    let outcome = execute(slot, prepared);
                    let _ = tx.send(outcome);
                });
            }
        }

        if in_flight == 0 {
            break;
        }

        // Wait for the next completion. We process them strictly in
        // arrival order so the status printer's interleaving is
        // deterministic relative to wall-clock completion.
        let outcome = match rx.recv() {
            Ok(o) => o,
            Err(_) => break,
        };
        in_flight -= 1;
        let slot = outcome.slot;
        done[slot] = true;
        for s2 in 0..plan.len() {
            if deps[s2].contains(&slot) && !done[s2] {
                remaining[s2] = remaining[s2].saturating_sub(1);
            }
        }
        match outcome.kind {
            OutcomeKind::Ok { combined } => {
                status.build_finished(&outcome.shown, &combined);
            }
            OutcomeKind::Failed { code, combined } => {
                status.build_finished(&outcome.shown, &combined);
                if hard_failure.is_none() {
                    hard_failure = Some(code);
                }
            }
            OutcomeKind::Interrupted => {
                interrupted = true;
            }
            OutcomeKind::SpawnError(e) => return Err(e),
        }
    }

    status.finish();
    if interrupted {
        println!("ninja: build stopped: interrupted by user.");
        return Ok(130);
    }
    if let Some(code) = hard_failure {
        eprintln!("ninja: build stopped: subcommand failed.");
        return Ok(code);
    }
    Ok(0)
}

/// Everything needed to run an edge in a worker thread, pre-resolved on
/// the main thread so the worker does no graph access.
struct Prepared {
    shown: String,
    command: String,
    outputs: String,
    rspfile: Option<(String, String)>,
}

fn prepare(state: &State, edge: &Edge, opts: &Options) -> Result<Prepared, String> {
    let rule = state
        .rules
        .get(&edge.rule)
        .ok_or_else(|| format!("unknown rule '{}'", edge.rule))?;
    let command = expand_in_edge(
        state,
        edge,
        rule.bindings
            .get("command")
            .map(|s| s.as_str())
            .unwrap_or(""),
    );
    let description = rule
        .bindings
        .get("description")
        .map(|d| expand_in_edge(state, edge, d))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| command.clone());
    let shown = if opts.verbose {
        command.clone()
    } else {
        description
    };

    // Output and depfile parent dirs are created up-front so commands can
    // assume the output directory exists. Matches
    // `test_depfile_directory_creation`.
    for out in edge.outputs.iter().chain(&edge.implicit_outputs) {
        ensure_parent_dir(out);
    }
    if let Some(depfile) = lookup_either(edge, rule, "depfile") {
        let path = expand_in_edge(state, edge, &depfile);
        if !path.is_empty() {
            ensure_parent_dir(&path);
        }
    }

    let rspfile = lookup_either(edge, rule, "rspfile")
        .map(|v| expand_in_edge(state, edge, &v))
        .filter(|s| !s.is_empty())
        .map(|path| {
            let content = lookup_either(edge, rule, "rspfile_content")
                .map(|v| expand_in_edge(state, edge, &v))
                .unwrap_or_default();
            (path, content)
        });

    Ok(Prepared {
        shown,
        command,
        outputs: edge.outputs.join(" "),
        rspfile,
    })
}

struct EdgeOutcome {
    slot: usize,
    shown: String,
    kind: OutcomeKind,
}

enum OutcomeKind {
    Ok { combined: Vec<u8> },
    Failed { code: u8, combined: Vec<u8> },
    Interrupted,
    SpawnError(String),
}

/// Worker-side: write the rspfile, spawn the command, capture output,
/// remove the rspfile. Decides whether the exit code maps to Ok, Failed,
/// or Interrupted (SIGINT).
fn execute(slot: usize, p: Prepared) -> EdgeOutcome {
    if let Some((path, content)) = &p.rspfile {
        ensure_parent_dir(path);
        if let Err(e) = std::fs::write(path, content) {
            return EdgeOutcome {
                slot,
                shown: p.shown,
                kind: OutcomeKind::SpawnError(format!("rspfile '{path}': {e}")),
            };
        }
    }

    let result = Command::new("sh").arg("-c").arg(&p.command).output();

    if let Some((path, _)) = &p.rspfile {
        let _ = std::fs::remove_file(path);
    }

    let output = match result {
        Ok(o) => o,
        Err(e) => {
            return EdgeOutcome {
                slot,
                shown: p.shown,
                kind: OutcomeKind::SpawnError(format!("failed to spawn: {e}")),
            };
        }
    };

    let mut combined = output.stdout;
    combined.extend_from_slice(&output.stderr);

    if output.status.success() {
        return EdgeOutcome {
            slot,
            shown: p.shown,
            kind: OutcomeKind::Ok { combined },
        };
    }

    let raw = output.status.code().unwrap_or(-1);
    if raw == 130 {
        return EdgeOutcome {
            slot,
            shown: p.shown,
            kind: OutcomeKind::Interrupted,
        };
    }
    let code: u8 = if !(0..=255).contains(&raw) {
        1
    } else {
        raw as u8
    };
    // Prepend the standard `FAILED: [code=N] outputs\ncommand\n` block
    // so it shows up between the status line and any captured output.
    let header = format!("FAILED: [code={raw}] {} \n{}\n", p.outputs, p.command);
    let mut buf = header.into_bytes();
    buf.extend_from_slice(&combined);
    EdgeOutcome {
        slot,
        shown: p.shown,
        kind: OutcomeKind::Failed {
            code,
            combined: buf,
        },
    }
}

fn ensure_parent_dir(path: &str) {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
}
