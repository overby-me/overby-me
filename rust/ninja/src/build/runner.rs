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
use std::time::SystemTime;

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
    let result = schedule(state, opts, &plan, &mut status)?;
    if result == 0 && real_count == 0 && !opts.quiet {
        // All edges in the plan turned out to be phony — preserve the
        // legacy "no work to do" message.
    }
    Ok(result)
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
    let explain = opts.explain();

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

    // Initial node dirtiness — kept deliberately minimal:
    //   - source files (no in-edge) referenced as inputs: dirty iff the
    //     file is missing on disk;
    //   - phony outputs whose edge has no inputs ('.FORCE'-style nodes)
    //     are always dirty;
    //   - everything else starts clean and the dispatch-time
    //      check (combined with each completed edge's
    //     dirty-output update) decides whether downstream work fires.
    let mut dirty: HashMap<String, bool> = HashMap::new();
    for edge in &edges {
        for inp in edge
            .inputs
            .iter()
            .chain(&edge.implicit_inputs)
            .chain(&edge.order_only_inputs)
        {
            if !producers.contains_key(inp) {
                dirty
                    .entry(inp.clone())
                    .or_insert_with(|| !std::path::Path::new(inp).exists());
            }
        }
    }
    for &edge_idx in plan.iter() {
        let edge = &edges[edge_idx];
        if edge.is_phony() && edge.inputs.is_empty() && edge.implicit_inputs.is_empty() {
            for o in edge.outputs.iter().chain(&edge.implicit_outputs) {
                dirty.insert(o.clone(), true);
            }
        }
    }

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
                // Re-check dirtiness right before dispatch: an upstream
                // restat-clean completion may have just made this edge
                // unnecessary. If clean, mark its outputs clean so
                // downstream consumers also short-circuit.
                let needs = edge_needs_run(&edge, &dirty);
                if !needs {
                    for o in edge.outputs.iter().chain(&edge.implicit_outputs) {
                        dirty.insert(o.clone(), false);
                    }
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
                // Capture explain lines for dirty inputs of this edge.
                let explain_lines: Vec<String> = if explain {
                    let mut seen = HashSet::new();
                    edge.inputs
                        .iter()
                        .chain(&edge.implicit_inputs)
                        .chain(&edge.order_only_inputs)
                        .filter(|i| *dirty.get(i.as_str()).unwrap_or(&false))
                        .filter(|i| seen.insert((*i).clone()))
                        .map(|i| format!("ninja explain: {i} is dirty"))
                        .collect()
                } else {
                    Vec::new()
                };
                let restat = rule_has_restat(state, &edge);
                let mtimes_before: Vec<Option<SystemTime>> = if restat {
                    edge.outputs
                        .iter()
                        .chain(&edge.implicit_outputs)
                        .map(|o| mtime_of(o))
                        .collect()
                } else {
                    Vec::new()
                };
                let outputs_owned: Vec<String> = edge
                    .outputs
                    .iter()
                    .chain(&edge.implicit_outputs)
                    .cloned()
                    .collect();
                status.build_started(&prepared.shown);
                let tx = tx.clone();
                let prepared_outputs = outputs_owned.clone();
                let prepared_mtimes = mtimes_before.clone();
                let restat_flag = restat;
                let explain_owned = explain_lines.clone();
                thread::spawn(move || {
                    let mut outcome = execute(slot, prepared);
                    outcome.explain = explain_owned;
                    outcome.restat = restat_flag;
                    outcome.outputs_for_restat = prepared_outputs;
                    outcome.mtimes_before = prepared_mtimes;
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
            // Update dirty status for the outputs based on restat.
            if outcome.restat {
                let after: Vec<Option<SystemTime>> = outcome
                    .outputs_for_restat
                    .iter()
                    .map(|o| mtime_of(o))
                    .collect();
                let unchanged = !outcome.outputs_for_restat.is_empty()
                    && after.len() == outcome.mtimes_before.len()
                    && after
                        .iter()
                        .zip(outcome.mtimes_before.iter())
                        .all(|(a, b)| a == b && a.is_some());
                let new_dirty = !unchanged;
                for o in &outcome.outputs_for_restat {
                    dirty.insert(o.clone(), new_dirty);
                }
            } else {
                for o in &outcome.outputs_for_restat {
                    dirty.insert(o.clone(), true);
                }
            }
            // Print any captured explain lines just before the
            // corresponding `[N/T]` status line. Doing it here keeps the
            // pair tightly grouped even when multiple edges complete in
            // the same batch.
            for line in &outcome.explain {
                println!("{line}");
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

/// Decide whether the edge still needs to run *right now*. An edge
/// needs to run if any of its outputs is missing on disk, any of its
/// outputs has been flagged dirty, or any of its inputs is currently
/// flagged dirty.
fn edge_needs_run(edge: &Edge, dirty: &HashMap<String, bool>) -> bool {
    for o in edge.outputs.iter().chain(&edge.implicit_outputs) {
        if !std::path::Path::new(o).exists() {
            return true;
        }
        if *dirty.get(o.as_str()).unwrap_or(&false) {
            return true;
        }
    }
    for i in edge.inputs.iter().chain(&edge.implicit_inputs) {
        if *dirty.get(i.as_str()).unwrap_or(&false) {
            return true;
        }
    }
    false
}

fn rule_has_restat(state: &State, edge: &Edge) -> bool {
    if let Some(v) = edge.bindings.get("restat") {
        return matches!(v.as_str(), "1" | "true");
    }
    if let Some(rule) = state.rules.get(&edge.rule)
        && let Some(v) = rule.bindings.get("restat")
    {
        return matches!(v.as_str(), "1" | "true");
    }
    false
}

fn mtime_of(path: &str) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
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
    explain: Vec<String>,
    restat: bool,
    outputs_for_restat: Vec<String>,
    mtimes_before: Vec<Option<SystemTime>>,
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
                explain: Vec::new(),
                restat: false,
                outputs_for_restat: Vec::new(),
                mtimes_before: Vec::new(),
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
                explain: Vec::new(),
                restat: false,
                outputs_for_restat: Vec::new(),
                mtimes_before: Vec::new(),
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
            explain: Vec::new(),
            restat: false,
            outputs_for_restat: Vec::new(),
            mtimes_before: Vec::new(),
        };
    }

    let raw = output.status.code().unwrap_or(-1);
    if raw == 130 {
        return EdgeOutcome {
            slot,
            shown: p.shown,
            kind: OutcomeKind::Interrupted,
            explain: Vec::new(),
            restat: false,
            outputs_for_restat: Vec::new(),
            mtimes_before: Vec::new(),
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
        explain: Vec::new(),
        restat: false,
        outputs_for_restat: Vec::new(),
        mtimes_before: Vec::new(),
    }
}

fn ensure_parent_dir(path: &str) {
    if let Some(parent) = std::path::Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        let _ = std::fs::create_dir_all(parent);
    }
}
