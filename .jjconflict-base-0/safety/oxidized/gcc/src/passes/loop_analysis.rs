//! Shared loop analysis utilities for optimization passes.
//!
//! Provides natural loop detection, loop body computation, and preheader
//! identification used by LICM and induction variable strength reduction.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::analysis;

/// A natural loop identified by its header block and the set of blocks in the loop body.
pub struct NaturalLoop {
    /// The header block index - the target of the back edge
    pub header: usize,
    /// All block indices that form the loop body (includes the header)
    pub body: FxHashSet<usize>,
}

/// Find all natural loops in the CFG.
///
/// A natural loop is defined by a back edge (tail -> header) where the header
/// dominates the tail. The loop body is the set of blocks that can reach the
/// tail without going through the header.
pub fn find_natural_loops(
    num_blocks: usize,
    preds: &analysis::FlatAdj,
    succs: &analysis::FlatAdj,
    idom: &[usize],
) -> Vec<NaturalLoop> {
    let mut loops = Vec::new();

    // Build dominance relation: does block `a` dominate block `b`?
    // We check by walking idom chain from b upward.
    let dominates = |a: usize, mut b: usize| -> bool {
        loop {
            if b == a {
                return true;
            }
            if b == idom[b] || idom[b] == usize::MAX {
                return false;
            }
            b = idom[b];
        }
    };

    // Find back edges: an edge (tail -> header) where header dominates tail
    for tail in 0..num_blocks {
        for &header in succs.row(tail) {
            let header = header as usize;
            if dominates(header, tail) {
                // Found a back edge: tail -> header
                // Compute the natural loop body
                let body = compute_loop_body(header, tail, preds);
                loops.push(NaturalLoop { header, body });
            }
        }
    }

    loops
}

/// Merge natural loops that share the same header block.
///
/// Multiple back edges targeting the same header produce separate NaturalLoop
/// entries, each with a partial body. We must take the union of all bodies
/// for the same header to ensure analysis covers ALL blocks in the loop.
pub fn merge_loops_by_header(loops: Vec<NaturalLoop>) -> Vec<NaturalLoop> {
    let mut header_map: FxHashMap<usize, FxHashSet<usize>> = FxHashMap::default();
    for nl in loops {
        header_map.entry(nl.header).or_default().extend(nl.body);
    }

    header_map
        .into_iter()
        .map(|(header, body)| NaturalLoop { header, body })
        .collect()
}

/// Compute the body of a natural loop given a back edge (tail -> header).
/// Uses a reverse walk from the tail, adding all blocks that can reach the
/// tail without going through the header.
fn compute_loop_body(header: usize, tail: usize, preds: &analysis::FlatAdj) -> FxHashSet<usize> {
    let mut body = FxHashSet::default();
    body.insert(header);

    if header == tail {
        // Self-loop
        return body;
    }

    // Walk backwards from tail, adding predecessors
    let mut worklist = vec![tail];
    body.insert(tail);

    while let Some(block) = worklist.pop() {
        for &pred in preds.row(block) {
            let pred = pred as usize;
            if !body.contains(&pred) {
                body.insert(pred);
                worklist.push(pred);
            }
        }
    }

    body
}

/// Find a suitable preheader block for a loop.
/// The preheader must be the single predecessor of the header that is not
/// part of the loop body. Returns None if no unique preheader exists.
pub fn find_preheader(
    header: usize,
    loop_body: &FxHashSet<usize>,
    preds: &analysis::FlatAdj,
) -> Option<usize> {
    let outside_preds: Vec<usize> = preds
        .row(header)
        .iter()
        .map(|&p| p as usize)
        .filter(|p| !loop_body.contains(p))
        .collect();

    if outside_preds.len() != 1 {
        return None;
    }

    Some(outside_preds[0])
}
