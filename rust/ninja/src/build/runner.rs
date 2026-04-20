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

use super::dyndep;
use super::expand::{expand_in_edge, lookup_either};
use super::plan::{build_plan, resolve_targets};
use crate::cli::Options;
use crate::graph::{Edge, State};
use crate::status::{Mode, Status};
use std::collections::{HashMap, HashSet};
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
        let dyndep_iter = edge.dyndep.iter();
        let mut seen_dep = HashSet::new();
        for inp in edge
            .inputs
            .iter()
            .chain(&edge.implicit_inputs)
            .chain(&edge.order_only_inputs)
            .chain(dyndep_iter)
        {
            if let Some(&prod) = state.producers.get(inp)
                && in_plan.contains(&prod)
                && let Some(&dep_slot) = idx_in_plan.get(&prod)
                && seen_dep.insert(dep_slot)
            {
                deps[slot].push(dep_slot);
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
    // Per-edge mutable copies so dyndep merges (extra outputs/inputs)
    // can land on the right edge before dispatch.
    let mut edges: Vec<Edge> = state.edges.to_vec();
    // Authoritative output → producing-edge map. Seeded from the
    // manifest, then extended as dyndep files are loaded. A duplicate
    // insert is the "multiple rules generate X" error.
    let mut producers: HashMap<String, usize> = state.producers.clone();
    // Cache of parsed dyndep files keyed by path so we only read each
    // one once even if multiple edges reference the same file.
    let mut dyndep_cache: HashMap<String, dyndep::DyndepFile> = HashMap::new();
    let mut dyndep_failure: Option<String> = None;
    // Edge indices whose dyndep file has already been merged in. Without
    // this we'd re-merge (and re-collide-against-self) every loop turn.
    let mut dyndep_loaded: HashSet<usize> = HashSet::new();

    loop {
        // Dispatch every ready edge up to the parallelism cap. We stop
        // launching new work once a hard failure or interruption fires
        // — let in-flight jobs drain so their output is preserved.
        if hard_failure.is_none() && !interrupted && dyndep_failure.is_none() {
            // Eagerly merge every loadable dyndep file before deciding
            // which edges to dispatch. Loading them up-front means a
            // "multiple rules generate X" collision is detected as
            // soon as the producing files exist on disk, and *before*
            // either consumer commits to running.
            for &edge_idx in plan.iter() {
                if let Some(dd_path) = edges[edge_idx].dyndep.clone()
                    && !dyndep_loaded.contains(&edge_idx)
                    && std::path::Path::new(&dd_path).exists()
                {
                    match merge_dyndep(
                        &mut edges,
                        edge_idx,
                        &dd_path,
                        &mut producers,
                        &mut dyndep_cache,
                    ) {
                        Ok(()) => {
                            dyndep_loaded.insert(edge_idx);
                        }
                        Err(e) => {
                            dyndep_failure = Some(e);
                            break;
                        }
                    }
                }
            }
            if dyndep_failure.is_some() {
                continue;
            }
            // To reliably detect dyndep collisions, defer dispatching
            // any consumer of a dyndep file until *every* dyndep-producing
            // edge in this plan has finished — otherwise we can race a
            // consumer ahead of the second producer's completion and miss
            // the "multiple rules generate X" check.
            let pending_dyndep_producers: HashSet<usize> = edges
                .iter()
                .filter_map(|e| e.dyndep.as_deref())
                .filter_map(|p| state.producers.get(p))
                .copied()
                .filter(|prod| {
                    in_plan.contains(prod) && idx_in_plan.get(prod).is_some_and(|s| !done[*s])
                })
                .collect();

            for slot in 0..plan.len() {
                if started[slot] || remaining[slot] > 0 {
                    continue;
                }
                if in_flight >= jobs {
                    break;
                }
                let edge_idx = plan[slot];
                if edges[edge_idx].dyndep.is_some() && !pending_dyndep_producers.is_empty() {
                    continue;
                }
                started[slot] = true;
                let edge = edges[edge_idx].clone();
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
                let prepared = prepare(state, &edge, opts)?;
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

        // Wait for the next completion, then opportunistically drain
        // any other already-arrived completions before re-entering the
        // dispatch loop. This keeps pacing deterministic and — more
        // importantly — lets us load *all* freshly-written dyndep files
        // before deciding which dependent edges to dispatch, so a
        // "multiple rules generate X" collision is detected before
        // either consumer commits to running.
        let mut batch = match rx.recv() {
            Ok(o) => vec![o],
            Err(_) => break,
        };
        while let Ok(more) = rx.try_recv() {
            batch.push(more);
        }
        for outcome in batch {
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
    }

    status.finish();
    if interrupted {
        println!("ninja: build stopped: interrupted by user.");
        return Ok(130);
    }
    if let Some(msg) = dyndep_failure {
        // Match upstream wording exactly:
        //   ninja: build stopped: multiple rules generate X.
        eprintln!("ninja: build stopped: {msg}.");
        return Ok(1);
    }
    if let Some(code) = hard_failure {
        eprintln!("ninja: build stopped: subcommand failed.");
        return Ok(code);
    }
    Ok(0)
}

/// Load (or fetch from cache) the dyndep file at `path`, locate the
/// entry whose explicit output matches one of `edges[idx]`'s outputs,
/// and merge the discovered implicit outputs + inputs into the edge.
/// On a duplicate output a "multiple rules generate X" string is
/// returned so the scheduler can short-circuit.
fn merge_dyndep(
    edges: &mut [Edge],
    idx: usize,
    path: &str,
    producers: &mut HashMap<String, usize>,
    cache: &mut HashMap<String, dyndep::DyndepFile>,
) -> Result<(), String> {
    if !cache.contains_key(path) {
        let src =
            std::fs::read_to_string(path).map_err(|e| format!("loading dyndep '{path}': {e}"))?;
        let parsed = dyndep::parse(&src).map_err(|e| format!("dyndep '{path}': {e}"))?;
        cache.insert(path.to_string(), parsed);
    }
    let ddf = &cache[path];
    // Find the entry keyed by any of the edge's explicit outputs.
    let edge = &mut edges[idx];
    let key = edge
        .outputs
        .iter()
        .find(|o| ddf.entries.contains_key(o.as_str()))
        .cloned();
    let Some(key) = key else {
        return Ok(());
    };
    let entry = &ddf.entries[&key];
    for new_out in &entry.implicit_outputs {
        if producers.contains_key(new_out) {
            return Err(format!("multiple rules generate {new_out}"));
        }
        producers.insert(new_out.clone(), idx);
        edge.implicit_outputs.push(new_out.clone());
    }
    for new_in in &entry.implicit_inputs {
        edge.implicit_inputs.push(new_in.clone());
    }
    if entry.restat {
        edge.bindings.insert("restat".to_string(), "1".to_string());
    }
    Ok(())
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
    if let Some(parent) = std::path::Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        let _ = std::fs::create_dir_all(parent);
    }
}
