//! Capability propagation dataflow (Task B1, invariant I10).
//!
//! A read-only, intraprocedural forward dataflow that answers the question
//! the checker will need at every dereference: *where was this pointer
//! derived?* Capabilities are resolved at **derivation roots** (a call that
//! mints a pointer from a bounded source — `as_mut_ptr`, `as_ptr`,
//! `Vec::as_mut_ptr`, …; a raw ref/address-of), propagated through pointer
//! arithmetic (`Offset`, `ptr::add`/`offset`/`wrapping_*`) and moves/casts,
//! and **compared at the dereference** — never re-resolved from the
//! faulting address (I10, trace rustsec-2021-0003 F10).
//!
//! B1 does not rewrite; it records, for every raw-pointer dereference,
//! whether propagation reached a known root or was lost. The worked
//! acceptance is `smallvec::insert_many`: the overflowing write must trace
//! back to the `as_mut_ptr()` derivation root, not to whatever allocation
//! the faulting address lands in.

use std::collections::HashMap;

use rustc_public::CrateDef;
use rustc_public::mir::{
    Body, LocalDecl, Operand, Place, ProjectionElem, Rvalue, StatementKind, TerminatorKind,
};
use rustc_public::ty::{RigidTy, TyKind};

/// Where a pointer value's provenance came from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Prov {
    /// Resolved to a derivation root (allocation/cast/FFI/scope entry). The
    /// string names the root for the report.
    Root(String),
    /// Propagation was lost (int->ptr round-trip, opaque call, union). The
    /// checker must fall back to a table resolve here (v0.5 policy).
    Lost,
}

/// One raw-pointer dereference and the provenance that reached it.
#[derive(Debug)]
pub struct DerefFact {
    /// Whether the access is a write.
    pub is_write: bool,
    /// The resolved provenance of the pointer being dereferenced.
    pub prov: Prov,
}

/// Result of the analysis for one body.
#[derive(Default, Debug)]
pub struct BodyProvenance {
    /// Raw dereferences whose provenance reached a derivation root.
    pub rooted_derefs: u64,
    /// Raw dereferences where propagation was lost.
    pub lost_derefs: u64,
    /// Raw *writes* through a pointer that reached a derivation root. The
    /// smallvec-0003 overflow is exactly one of these.
    pub rooted_writes: u64,
    /// The distinct root descriptions that fed a *write*, for the report
    /// (e.g. `as_mut_ptr`).
    pub write_roots: Vec<String>,
}

/// Runs the propagation dataflow over one body to a bounded fixpoint and
/// returns the per-deref provenance facts.
pub fn analyze(body: &Body) -> (BodyProvenance, Vec<DerefFact>) {
    let locals = body.locals();
    let mut state: HashMap<usize, Prov> = HashMap::new();

    // Fixpoint over the whole body so pointers derived inside loops (as in
    // insert_many) settle. The lattice is tiny (Root(name) / Lost), and a
    // conflicting join goes to Lost, so iteration terminates quickly; the
    // bound is a backstop.
    let max_rounds = 8;
    for _ in 0..max_rounds {
        let mut changed = false;
        for block in &body.blocks {
            for stmt in &block.statements {
                if let StatementKind::Assign(place, rvalue) = &stmt.kind {
                    changed |= assign(&mut state, locals, place, rvalue);
                }
            }
            if let TerminatorKind::Call {
                func,
                args,
                destination,
                ..
            } = &block.terminator.kind
            {
                changed |= call(&mut state, locals, func, args, destination);
            }
        }
        if !changed {
            break;
        }
    }

    // Collect the per-dereference facts under the settled state.
    let mut facts = Vec::new();
    for block in &body.blocks {
        for stmt in &block.statements {
            if let StatementKind::Assign(dst, rvalue) = &stmt.kind {
                collect_place_derefs(&state, locals, dst, true, &mut facts);
                for_each_read_place(rvalue, &mut |p| {
                    collect_place_derefs(&state, locals, p, false, &mut facts)
                });
            }
        }
        // Writes through raw pointers usually happen via intrinsic *calls*
        // (`ptr::write`, `ptr::copy*`), not a bare `*p = v`: real unsafe code
        // (smallvec::insert_many) writes with `ptr::write(cur, element)`.
        // Resolve the destination argument's provenance at the call site.
        if let TerminatorKind::Call { func, args, .. } = &block.terminator.kind
            && let Some(name) = callee_name(func)
            && let Some(dest_idx) = ptr_write_dest_arg(&name)
            && let Some(arg) = args.get(dest_idx)
            && let Some(prov) = operand_prov(&state, arg)
        {
            facts.push(DerefFact {
                is_write: true,
                prov,
            });
        }
    }

    let mut summary = BodyProvenance::default();
    for f in &facts {
        match &f.prov {
            Prov::Root(name) => {
                summary.rooted_derefs += 1;
                if f.is_write {
                    summary.rooted_writes += 1;
                    if !summary.write_roots.contains(name) {
                        summary.write_roots.push(name.clone());
                    }
                }
            }
            Prov::Lost => summary.lost_derefs += 1,
        }
    }
    (summary, facts)
}

/// Whether a local's declared type is a raw pointer.
fn is_raw_ptr(locals: &[LocalDecl], local: usize) -> bool {
    locals
        .get(local)
        .is_some_and(|d| matches!(d.ty.kind(), TyKind::RigidTy(RigidTy::RawPtr(..))))
}

/// Joins a new provenance into a local's slot, returning whether it changed.
fn join(state: &mut HashMap<usize, Prov>, local: usize, incoming: Prov) -> bool {
    match state.get(&local) {
        Some(existing) if *existing == incoming => false,
        Some(Prov::Lost) => false, // already bottom
        Some(_) => {
            // Conflicting roots meet at Lost.
            state.insert(local, Prov::Lost);
            true
        }
        None => {
            state.insert(local, incoming);
            true
        }
    }
}

/// Provenance of an operand's pointer value, if it is a bare pointer local.
fn operand_prov(state: &HashMap<usize, Prov>, op: &Operand) -> Option<Prov> {
    let place = match op {
        Operand::Copy(p) | Operand::Move(p) => p,
        Operand::Constant(_) | Operand::RuntimeChecks(_) => return Some(Prov::Lost),
    };
    if place.projection.is_empty() {
        state.get(&place.local).cloned()
    } else {
        // A projection off a pointer (deref/field) is a fresh value whose
        // provenance we do not track through memory in v0.
        Some(Prov::Lost)
    }
}

/// Handles a statement assignment; returns whether state changed.
fn assign(
    state: &mut HashMap<usize, Prov>,
    locals: &[LocalDecl],
    place: &Place,
    rvalue: &Rvalue,
) -> bool {
    // Only track provenance flowing *into* a bare raw-pointer local.
    if !place.projection.is_empty() || !is_raw_ptr(locals, place.local) {
        return false;
    }
    let dst = place.local;

    match rvalue {
        // Pointer arithmetic: the result inherits the base's provenance.
        Rvalue::BinaryOp(op, lhs, _rhs) if is_offset_op(op) => match operand_prov(state, lhs) {
            Some(p) => join(state, dst, p),
            None => false,
        },
        // A raw reference/address-of a place is a derivation root: the
        // pointer is minted here from that place's allocation.
        Rvalue::AddressOf(_, src) => {
            let root = root_for_addressof(src);
            join(state, dst, Prov::Root(root))
        }
        // Move/copy of another pointer, and pointer<->pointer casts:
        // inherit provenance.
        Rvalue::Use(op, _) | Rvalue::Cast(_, op, _) => match operand_prov(state, op) {
            Some(p) => join(state, dst, p),
            None => false,
        },
        _ => false,
    }
}

/// Handles a call terminator; returns whether state changed.
fn call(
    state: &mut HashMap<usize, Prov>,
    locals: &[LocalDecl],
    func: &Operand,
    args: &[Operand],
    destination: &Place,
) -> bool {
    if !destination.projection.is_empty() || !is_raw_ptr(locals, destination.local) {
        return false;
    }
    let dst = destination.local;
    let Some(name) = callee_name(func) else {
        return join(state, dst, Prov::Lost);
    };

    // Pointer-arithmetic intrinsics/methods propagate the receiver's prov.
    if is_ptr_arith_call(&name) {
        if let Some(first) = args.first()
            && let Some(p) = operand_prov(state, first)
        {
            return join(state, dst, p);
        }
        return join(state, dst, Prov::Lost);
    }

    // Known pointer-minting sources are derivation roots.
    if let Some(root) = root_for_call(&name) {
        return join(state, dst, Prov::Root(root));
    }

    // Any other call returning a raw pointer: provenance unknown.
    join(state, dst, Prov::Lost)
}

fn is_offset_op(op: &rustc_public::mir::BinOp) -> bool {
    matches!(op, rustc_public::mir::BinOp::Offset)
}

/// Names a derivation root for an address-of of `src`.
fn root_for_addressof(src: &Place) -> String {
    if src
        .projection
        .iter()
        .any(|e| matches!(e, ProjectionElem::Deref))
    {
        "addr_of(deref)".to_string()
    } else {
        format!("addr_of(local {})", src.local)
    }
}

/// The demangled-ish tail of a callee's path, for matching known sources.
fn callee_name(func: &Operand) -> Option<String> {
    let ty = match func {
        Operand::Constant(c) => c.ty(),
        Operand::Copy(_) | Operand::Move(_) | Operand::RuntimeChecks(_) => return None,
    };
    match ty.kind() {
        TyKind::RigidTy(RigidTy::FnDef(def, _)) => Some(def.name()),
        _ => None,
    }
}

/// Whether a callee is pointer arithmetic that propagates provenance.
fn is_ptr_arith_call(name: &str) -> bool {
    const ARITH: [&str; 6] = [
        "add",
        "offset",
        "sub",
        "wrapping_add",
        "wrapping_offset",
        "wrapping_sub",
    ];
    let tail = name.rsplit("::").next().unwrap_or(name);
    ARITH.contains(&tail)
}

/// If a callee writes through a pointer argument, the index of that
/// destination argument. `ptr::write*`/`write_bytes` write arg 0;
/// `ptr::copy`/`copy_nonoverlapping` write arg 1 (`copy(src, dst, count)`).
fn ptr_write_dest_arg(name: &str) -> Option<usize> {
    let tail = name.rsplit("::").next().unwrap_or(name);
    match tail {
        "write" | "write_unaligned" | "write_volatile" | "write_bytes" | "replace" => Some(0),
        "copy" | "copy_nonoverlapping" => Some(1),
        _ => None,
    }
}

/// Whether a callee mints a pointer from a bounded source (a derivation
/// root), returning the root name to report.
fn root_for_call(name: &str) -> Option<String> {
    const SOURCES: [&str; 6] = [
        "as_mut_ptr",
        "as_ptr",
        "as_mut_slice",
        "as_slice",
        "into_raw",
        "as_non_null_ptr",
    ];
    let tail = name.rsplit("::").next().unwrap_or(name);
    SOURCES.contains(&tail).then(|| tail.to_string())
}

/// Emits deref facts for every raw dereference in `place`.
fn collect_place_derefs(
    state: &HashMap<usize, Prov>,
    locals: &[LocalDecl],
    place: &Place,
    is_write: bool,
    out: &mut Vec<DerefFact>,
) {
    // A deref appears as the first projection off the base local.
    if place
        .projection
        .first()
        .is_some_and(|e| matches!(e, ProjectionElem::Deref))
        && is_raw_ptr(locals, place.local)
    {
        let prov = state.get(&place.local).cloned().unwrap_or(Prov::Lost);
        out.push(DerefFact { is_write, prov });
    }
}

/// Invokes `f` for each place read by an rvalue (for classifying read
/// dereferences).
fn for_each_read_place(rvalue: &Rvalue, f: &mut impl FnMut(&Place)) {
    let op = |o: &Operand, f: &mut dyn FnMut(&Place)| {
        if let Operand::Copy(p) | Operand::Move(p) = o {
            f(p);
        }
    };
    match rvalue {
        Rvalue::Use(o, _) | Rvalue::Cast(_, o, _) | Rvalue::UnaryOp(_, o) => op(o, f),
        Rvalue::BinaryOp(_, a, b) => {
            op(a, f);
            op(b, f);
        }
        Rvalue::Ref(_, _, p)
        | Rvalue::AddressOf(_, p)
        | Rvalue::Len(p)
        | Rvalue::Discriminant(p) => f(p),
        _ => {}
    }
}
