//! MIR rewriting: raw-dereference checking with provenance (Tasks B2, B3).
//!
//! `rustc_public` (stable MIR) is read-only, so rewriting drops to a
//! `rustc_driver::Callbacks` that sets `config.override_queries` and wraps
//! the `optimized_mir` query: it clones each local body and, before every
//! raw-pointer dereference, splits the block and injects a call to
//! `cementite::__fec_check_deref_rooted(fault, root)`.
//!
//! **B3 / I10.** The check takes two pointers: the faulting pointer and the
//! pointer at its *derivation root*. A per-body provenance dataflow (the
//! internal-MIR twin of `provenance.rs`) resolves, for each dereferenced
//! pointer, the local holding the pointer it was derived from — the result
//! of `as_mut_ptr`/`as_ptr`/an address-of, propagated through pointer
//! arithmetic. The runtime resolves the owning allocation from the *root*,
//! never the faulting address, so an overflow into an adjacent live
//! allocation is caught instead of silently resolving to the neighbour's
//! capability (trace `rustsec-2021-0003` F10).
//!
//! Modelled on rustc's own `rustc_mir_transform::check_pointers` pass for
//! the block-splitting/injection mechanics.

use std::collections::HashMap;

use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{CRATE_DEF_INDEX, DefId, LocalDefId};
use rustc_index::IndexVec;
use rustc_middle::mir::visit::{PlaceContext, Visitor};
use rustc_middle::mir::{
    BasicBlock, BasicBlockData, BinOp, Body, CallSource, CastKind, Const as MirConst, ConstOperand,
    Local, LocalDecl, Location, Operand, Place, RawPtrKind, Rvalue, SourceInfo, Statement,
    StatementKind, Terminator, TerminatorKind, UnwindAction,
};
use rustc_middle::ty::{self, Mutability, Ty, TyCtxt};
use rustc_span::Spanned;
use std::collections::HashSet;
use thin_vec::ThinVec;

/// Runs the compiler on `args` with the instrumentation pass installed.
/// Returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    let mut callbacks = FecInstrument;
    rustc_driver::run_compiler(args, &mut callbacks);
    0
}

struct FecInstrument;

impl rustc_driver::Callbacks for FecInstrument {
    fn config(&mut self, config: &mut rustc_interface::interface::Config) {
        config.override_queries = Some(|_sess, providers| {
            providers.queries.optimized_mir = fec_optimized_mir;
        });
    }
}

/// Our `optimized_mir` provider: take the compiler's optimized body, clone
/// it, instrument it, and hand back the arena-allocated result.
fn fec_optimized_mir(tcx: TyCtxt<'_>, def_id: LocalDefId) -> &Body<'_> {
    let default = (rustc_interface::DEFAULT_QUERY_PROVIDERS
        .queries
        .optimized_mir)(tcx, def_id);
    let Some(fns) = find_fec_fns(tcx) else {
        // cementite not linked / symbols not found: leave the body untouched
        // rather than fail the build.
        return default;
    };
    let mut body = default.clone();
    let injected = instrument_body(tcx, &mut body, &fns);
    if injected > 0 && std::env::var_os("FEC_DEBUG").is_some() {
        eprintln!(
            "fe-c-debug instrumented {} ({} checks)",
            tcx.def_path_str(def_id.to_def_id()),
            injected
        );
    }
    tcx.arena.alloc(body)
}

/// The cementite check entry points the pass injects calls to.
struct FecFns {
    /// `__fec_check_deref_rooted(fault, root)`.
    check: DefId,
    /// `__fec_scope_enter(base, len)`.
    scope_enter: DefId,
    /// `__fec_scope_exit(base)`.
    scope_exit: DefId,
}

/// Resolves cementite's check/scope entry points by walking the crate graph
/// and the cementite crate root's module children.
fn find_fec_fns(tcx: TyCtxt<'_>) -> Option<FecFns> {
    let cnum = tcx
        .crates(())
        .iter()
        .find(|&&c| tcx.crate_name(c).as_str() == "cementite")?;
    let root = DefId {
        krate: *cnum,
        index: CRATE_DEF_INDEX,
    };
    let children = tcx.module_children(root);
    let find = |name: &str| {
        children.iter().find_map(|child| {
            if child.ident.as_str() == name
                && let Res::Def(DefKind::Fn, def_id) = child.res
            {
                Some(def_id)
            } else {
                None
            }
        })
    };
    Some(FecFns {
        check: find("__fec_check_deref_rooted")?,
        scope_enter: find("__fec_scope_enter")?,
        scope_exit: find("__fec_scope_exit")?,
    })
}

/// Injects cementite check/scope calls into `body`: a rooted deref check at
/// every raw access (I10) and stack scope enter/exit hooks for
/// address-taken locals (I8). Returns how many calls were injected.
fn instrument_body<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>, fns: &FecFns) -> usize {
    let check_fn = fns.check;
    // Provenance first, on the un-mutated body: which local holds each
    // pointer's derivation root (I10). Keyed on original locals, which the
    // block-splitting below never renumbers.
    let roots = compute_roots(tcx, body);
    let mut injected = 0usize;

    // Pass 0: stack scope hooks (I8), behind FEC_SCOPE_HOOKS. Register each
    // address-taken local as a stack region and poison it at frame teardown,
    // so a pointer that escapes the scope (as the rusqlite-0128 closure does
    // across FFI) resolves as a dead stack region. Opt-in for now:
    // instrumenting *every* address-taken local is impractical without an
    // escape analysis (most never leave the frame) — see STATUS.
    if std::env::var_os("FEC_SCOPE_HOOKS").is_some() {
        injected += instrument_scopes(tcx, body, fns);
    }

    let basic_blocks = body.basic_blocks.as_mut();
    let local_decls = &mut body.local_decls;

    // Pass 1: pointer-write intrinsic calls. Real unsafe code (and
    // smallvec::insert_many) writes through raw pointers with
    // `ptr::write`/`ptr::copy`, which are Call terminators, not `*p = v`
    // deref places — so check the destination pointer before the call.
    let original_blocks: Vec<BasicBlock> = basic_blocks.indices().collect();
    for block in original_blocks {
        injected += instrument_write_call(tcx, basic_blocks, local_decls, &roots, block, check_fn);
    }

    // Pass 2: direct raw-pointer deref places (`*p`). Reverse order so
    // splitting a block does not disturb the indices of statements we have
    // not visited yet (same discipline as check_pointers).
    for block in basic_blocks.indices().rev() {
        for stmt_index in (0..basic_blocks[block].statements.len()).rev() {
            let location = Location {
                block,
                statement_index: stmt_index,
            };
            let source_info = basic_blocks[block].statements[stmt_index].source_info;

            let mut finder = DerefFinder {
                local_decls,
                found: Vec::new(),
            };
            finder.visit_statement(&basic_blocks[block].statements[stmt_index], location);
            let found = finder.found;

            for fault_local in found {
                // The pointer's derivation root, or itself when propagation
                // found none (unpropagated fallback: resolves the access's
                // own allocation — never a false positive, at worst a false
                // negative when provenance is lost).
                let root_local = roots.get(&fault_local).copied().unwrap_or(fault_local);
                injected += 1;

                let new_block = split_block(basic_blocks, location);
                let bd = &mut basic_blocks[block];

                let u8_ptr = Ty::new_imm_ptr(tcx, tcx.types.u8);
                // Cast the faulting pointer and the root pointer to
                // *const u8, threading each as a distinct SSA value.
                let fault_arg = cast_to_u8_ptr(local_decls, bd, fault_local, u8_ptr, source_info);
                let root_arg = cast_to_u8_ptr(local_decls, bd, root_local, u8_ptr, source_info);

                let ret = local_decls.push(LocalDecl::new(tcx.types.unit, source_info.span));
                bd.terminator = Some(Terminator {
                    source_info,
                    kind: TerminatorKind::Call {
                        func: Operand::function_handle(tcx, check_fn, [], source_info.span),
                        args: Box::new([
                            Spanned {
                                node: Operand::Move(Place::from(fault_arg)),
                                span: source_info.span,
                            },
                            Spanned {
                                node: Operand::Move(Place::from(root_arg)),
                                span: source_info.span,
                            },
                        ]),
                        destination: Place::from(ret),
                        target: Some(new_block),
                        unwind: UnwindAction::Unreachable,
                        call_source: CallSource::Misc,
                        fn_span: source_info.span,
                    },
                    attributes: ThinVec::new(),
                });
            }
        }
    }
    injected
}

/// If `block` ends in a pointer-write intrinsic call
/// (`ptr::write`/`copy`/`copy_nonoverlapping`/`write_bytes`), injects a
/// `__fec_check_deref_rooted(dst, root)` on the destination pointer *before*
/// the call: the original call moves into a fresh block, and this block's
/// terminator becomes the check, targeting it.
fn instrument_write_call<'tcx>(
    tcx: TyCtxt<'tcx>,
    basic_blocks: &mut IndexVec<BasicBlock, BasicBlockData<'tcx>>,
    local_decls: &mut IndexVec<Local, LocalDecl<'tcx>>,
    roots: &HashMap<Local, Local>,
    block: BasicBlock,
    check_fn: DefId,
) -> usize {
    let term = basic_blocks[block].terminator();
    let source_info = term.source_info;
    let TerminatorKind::Call { func, args, .. } = &term.kind else {
        return 0;
    };
    let Some(name) = callee_name(tcx, local_decls, func) else {
        return 0;
    };
    let Some(dest_idx) = ptr_write_dest_arg(&name) else {
        return 0;
    };
    let Some(dst_place) = args.get(dest_idx).and_then(|a| a.node.place()) else {
        return 0;
    };
    if !dst_place.projection.is_empty() || !is_raw_ptr_local(local_decls, dst_place.local) {
        return 0;
    }
    let fault_local = dst_place.local;
    let root_local = roots.get(&fault_local).copied().unwrap_or(fault_local);

    // Move the original write call into a fresh block.
    let orig = basic_blocks[block].terminator.take();
    let is_cleanup = basic_blocks[block].is_cleanup;
    let new_block = basic_blocks.push(BasicBlockData::new_stmts(Vec::new(), orig, is_cleanup));

    let u8_ptr = Ty::new_imm_ptr(tcx, tcx.types.u8);
    let bd = &mut basic_blocks[block];
    let fault_arg = cast_to_u8_ptr(local_decls, bd, fault_local, u8_ptr, source_info);
    let root_arg = cast_to_u8_ptr(local_decls, bd, root_local, u8_ptr, source_info);
    let ret = local_decls.push(LocalDecl::new(tcx.types.unit, source_info.span));

    bd.terminator = Some(Terminator {
        source_info,
        kind: TerminatorKind::Call {
            func: Operand::function_handle(tcx, check_fn, [], source_info.span),
            args: Box::new([
                Spanned {
                    node: Operand::Move(Place::from(fault_arg)),
                    span: source_info.span,
                },
                Spanned {
                    node: Operand::Move(Place::from(root_arg)),
                    span: source_info.span,
                },
            ]),
            destination: Place::from(ret),
            target: Some(new_block),
            unwind: UnwindAction::Unreachable,
            call_source: CallSource::Misc,
            fn_span: source_info.span,
        },
        attributes: ThinVec::new(),
    });
    1
}

/// If a callee writes through a pointer argument, the index of that
/// destination argument. `ptr::write*`/`write_bytes` write arg 0;
/// `ptr::copy`/`copy_nonoverlapping` write arg 1 (`copy(src, dst, count)`).
fn ptr_write_dest_arg(name: &str) -> Option<usize> {
    match name {
        "write" | "write_unaligned" | "write_volatile" | "write_bytes" => Some(0),
        "copy" | "copy_nonoverlapping" => Some(1),
        _ => None,
    }
}

/// Appends a `tmp = ptr_local as *const u8` cast to `bd` and returns the
/// temp local.
fn cast_to_u8_ptr<'tcx>(
    local_decls: &mut IndexVec<Local, LocalDecl<'tcx>>,
    bd: &mut BasicBlockData<'tcx>,
    ptr_local: Local,
    u8_ptr: Ty<'tcx>,
    source_info: rustc_middle::mir::SourceInfo,
) -> Local {
    let tmp = local_decls.push(LocalDecl::new(u8_ptr, source_info.span));
    bd.statements.push(Statement::new(
        source_info,
        StatementKind::Assign(Box::new((
            Place::from(tmp),
            Rvalue::Cast(
                CastKind::PtrToPtr,
                Operand::Copy(Place::from(ptr_local)),
                u8_ptr,
            ),
        ))),
    ));
    tmp
}

// ---- stack scope hooks (I8) -----------------------------------------------

/// Emits stack scope hooks (I8) at **frame granularity**: `scope_enter` just
/// after a local's address is first taken, and `scope_exit` before every
/// `Return` (frame teardown). Optimized MIR strips `StorageLive`/`Dead`, so
/// lexical-scope granularity is not available here; frame granularity
/// catches use-after-function-return (a pointer that escapes a frame and is
/// dereferenced after it returns). Returns how many hooks were injected.
fn instrument_scopes<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>, fns: &FecFns) -> usize {
    let sites = address_taken_sites(body);
    if sites.is_empty() {
        return 0;
    }
    let typing_env = body.typing_env(tcx);
    let basic_blocks = body.basic_blocks.as_mut();
    let local_decls = &mut body.local_decls;
    let mut injected = 0usize;

    // scope_exit before every Return, for each escaping local (processed
    // before enters, since enters below add blocks). One exit call per
    // (return-block, local); chained through fresh blocks.
    let return_blocks: Vec<BasicBlock> = basic_blocks
        .indices()
        .filter(|&b| matches!(basic_blocks[b].terminator().kind, TerminatorKind::Return))
        .collect();
    for rb in return_blocks {
        for &(local, _, _) in &sites {
            let ty = local_decls[local].ty;
            let source_info = basic_blocks[rb].terminator().source_info;
            inject_scope_before_return(
                tcx,
                basic_blocks,
                local_decls,
                rb,
                local,
                ty,
                fns.scope_exit,
                None,
                source_info,
            );
            injected += 1;
        }
    }

    // scope_enter just after each address-taking statement. Reverse so
    // splits do not disturb earlier indices.
    let mut by_block: std::collections::BTreeMap<BasicBlock, Vec<(Local, usize)>> =
        std::collections::BTreeMap::new();
    for &(local, block, idx) in &sites {
        by_block.entry(block).or_default().push((local, idx));
    }
    for (block, mut entries) in by_block {
        entries.sort_by_key(|&(_, idx)| std::cmp::Reverse(idx));
        for (local, idx) in entries {
            let ty = local_decls[local].ty;
            let Ok(layout) = tcx.layout_of(typing_env.as_query_input(ty)) else {
                continue;
            };
            let size = layout.size.bytes();
            let source_info = basic_blocks[block].statements[idx].source_info;
            inject_scope_call(
                tcx,
                basic_blocks,
                local_decls,
                block,
                idx + 1,
                local,
                ty,
                fns.scope_enter,
                Some(size),
                source_info,
            );
            injected += 1;
        }
    }
    injected
}

/// The first address-taking site `(local, block, statement_index)` for each
/// local whose address is taken (via `&raw`/`&`) — candidates for escaping
/// their stack scope.
fn address_taken_sites(body: &Body<'_>) -> Vec<(Local, BasicBlock, usize)> {
    let mut seen: HashSet<Local> = HashSet::new();
    let mut sites = Vec::new();
    for block in body.basic_blocks.indices() {
        for (i, stmt) in body.basic_blocks[block].statements.iter().enumerate() {
            if let StatementKind::Assign(boxed) = &stmt.kind {
                let place = match &boxed.1 {
                    Rvalue::RawPtr(_, p) | Rvalue::Ref(_, _, p) => p,
                    _ => continue,
                };
                if place.projection.is_empty() && seen.insert(place.local) {
                    sites.push((place.local, block, i));
                }
            }
        }
    }
    sites
}

/// Injects `scope_exit(&raw local)` immediately before `block`'s `Return`
/// terminator: the original terminator moves to a fresh block that the
/// injected call targets.
#[allow(clippy::too_many_arguments)]
fn inject_scope_before_return<'tcx>(
    tcx: TyCtxt<'tcx>,
    basic_blocks: &mut IndexVec<BasicBlock, BasicBlockData<'tcx>>,
    local_decls: &mut IndexVec<Local, LocalDecl<'tcx>>,
    block: BasicBlock,
    local: Local,
    ty: Ty<'tcx>,
    func: DefId,
    len: Option<u64>,
    source_info: SourceInfo,
) {
    let orig = basic_blocks[block].terminator.take();
    let is_cleanup = basic_blocks[block].is_cleanup;
    let ret_block = basic_blocks.push(BasicBlockData::new_stmts(Vec::new(), orig, is_cleanup));
    emit_scope_terminator(
        tcx,
        basic_blocks,
        local_decls,
        block,
        ret_block,
        local,
        ty,
        func,
        len,
        source_info,
    );
}

/// Injects a scope hook call at `block`/`at`: splits the block so the
/// original tail becomes the call's target, then builds the hook terminator.
#[allow(clippy::too_many_arguments)]
fn inject_scope_call<'tcx>(
    tcx: TyCtxt<'tcx>,
    basic_blocks: &mut IndexVec<BasicBlock, BasicBlockData<'tcx>>,
    local_decls: &mut IndexVec<Local, LocalDecl<'tcx>>,
    block: BasicBlock,
    at: usize,
    local: Local,
    ty: Ty<'tcx>,
    func: DefId,
    len: Option<u64>,
    source_info: SourceInfo,
) {
    let new_block = split_block(
        basic_blocks,
        Location {
            block,
            statement_index: at,
        },
    );
    emit_scope_terminator(
        tcx,
        basic_blocks,
        local_decls,
        block,
        new_block,
        local,
        ty,
        func,
        len,
        source_info,
    );
}

/// Builds `func(&raw const local as *const u8, [len])` as `block`'s
/// terminator, targeting `target`. `&raw const local` is cast to `*const u8`
/// and threaded through.
#[allow(clippy::too_many_arguments)]
fn emit_scope_terminator<'tcx>(
    tcx: TyCtxt<'tcx>,
    basic_blocks: &mut IndexVec<BasicBlock, BasicBlockData<'tcx>>,
    local_decls: &mut IndexVec<Local, LocalDecl<'tcx>>,
    block: BasicBlock,
    target: BasicBlock,
    local: Local,
    ty: Ty<'tcx>,
    func: DefId,
    len: Option<u64>,
    source_info: SourceInfo,
) {
    let bd = &mut basic_blocks[block];

    // _addr = &raw const local : *const ty
    let raw_ty = Ty::new_ptr(tcx, ty, Mutability::Not);
    let addr = local_decls.push(LocalDecl::new(raw_ty, source_info.span));
    bd.statements.push(Statement::new(
        source_info,
        StatementKind::Assign(Box::new((
            Place::from(addr),
            Rvalue::RawPtr(RawPtrKind::Const, Place::from(local)),
        ))),
    ));
    let u8_ptr = Ty::new_imm_ptr(tcx, tcx.types.u8);
    let addr_u8 = cast_to_u8_ptr(local_decls, bd, addr, u8_ptr, source_info);

    let mut args = vec![Spanned {
        node: Operand::Move(Place::from(addr_u8)),
        span: source_info.span,
    }];
    if let Some(len) = len {
        args.push(Spanned {
            node: Operand::Constant(Box::new(ConstOperand {
                span: source_info.span,
                user_ty: None,
                const_: MirConst::from_usize(tcx, len),
            })),
            span: source_info.span,
        });
    }

    let ret = local_decls.push(LocalDecl::new(tcx.types.unit, source_info.span));
    bd.terminator = Some(Terminator {
        source_info,
        kind: TerminatorKind::Call {
            func: Operand::function_handle(tcx, func, [], source_info.span),
            args: args.into_boxed_slice(),
            destination: Place::from(ret),
            target: Some(target),
            unwind: UnwindAction::Unreachable,
            call_source: CallSource::Misc,
            fn_span: source_info.span,
        },
        attributes: ThinVec::new(),
    });
}

/// Collects the base locals of raw-pointer dereferences in a statement
/// (indirect places whose base local is a raw pointer).
struct DerefFinder<'a, 'tcx> {
    local_decls: &'a IndexVec<Local, LocalDecl<'tcx>>,
    found: Vec<Local>,
}

impl<'tcx> Visitor<'tcx> for DerefFinder<'_, 'tcx> {
    fn visit_place(&mut self, place: &Place<'tcx>, context: PlaceContext, location: Location) {
        if place.is_indirect() && is_raw_ptr_local(self.local_decls, place.local) {
            self.found.push(place.local);
        }
        self.super_place(place, context, location);
    }
}

/// Splits `location.block` at `location.statement_index`, moving the tail
/// statements and the terminator into a fresh block, and returns it.
fn split_block(
    basic_blocks: &mut IndexVec<BasicBlock, BasicBlockData<'_>>,
    location: Location,
) -> BasicBlock {
    let block_data = &mut basic_blocks[location.block];
    let new_block = BasicBlockData::new_stmts(
        block_data.statements.split_off(location.statement_index),
        block_data.terminator.take(),
        block_data.is_cleanup,
    );
    basic_blocks.push(new_block)
}

// ---- provenance dataflow on internal MIR (I10) ----------------------------

/// Computes, for each raw-pointer local, the local holding its derivation
/// root. The internal-MIR twin of `provenance.rs`: roots are pointer-minting
/// calls (`as_mut_ptr`, `as_ptr`, …) and address-of; provenance propagates
/// through pointer arithmetic (`Offset`, `ptr::add`/`offset`/`wrapping_*`)
/// and moves/casts to a bounded fixpoint.
fn compute_roots<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> HashMap<Local, Local> {
    let mut root_of: HashMap<Local, Local> = HashMap::new();
    let decls = &body.local_decls;

    for _ in 0..8 {
        let mut changed = false;
        for bb in body.basic_blocks.iter() {
            for stmt in &bb.statements {
                if let StatementKind::Assign(boxed) = &stmt.kind {
                    let (place, rvalue) = &**boxed;
                    if !place.projection.is_empty() || !is_raw_ptr_local(decls, place.local) {
                        continue;
                    }
                    let dst = place.local;
                    let incoming = match rvalue {
                        // Address-of mints a pointer from that place's
                        // allocation: dst is itself a root.
                        Rvalue::RawPtr(..) | Rvalue::Ref(..) => Some(dst),
                        Rvalue::Use(op, _) | Rvalue::Cast(_, op, _) => operand_root(&root_of, op),
                        Rvalue::BinaryOp(BinOp::Offset, boxed) => operand_root(&root_of, &boxed.0),
                        _ => None,
                    };
                    changed |= update(&mut root_of, dst, incoming);
                }
            }
            if let TerminatorKind::Call {
                func,
                args,
                destination,
                ..
            } = &bb.terminator().kind
                && destination.projection.is_empty()
                && is_raw_ptr_local(decls, destination.local)
            {
                let dst = destination.local;
                let name = callee_name(tcx, decls, func);
                let incoming = match name.as_deref() {
                    Some(n) if is_ptr_arith(n) => {
                        args.first().and_then(|a| operand_root(&root_of, &a.node))
                    }
                    Some(n) if is_root_source(n) => Some(dst),
                    _ => None,
                };
                changed |= update(&mut root_of, dst, incoming);
            }
        }
        if !changed {
            break;
        }
    }
    root_of
}

/// Merges `incoming` into `root_of[dst]`, returning whether it changed. A
/// conflicting root drops the entry (unknown), matching the join-to-lost of
/// the stable-MIR analysis.
fn update(root_of: &mut HashMap<Local, Local>, dst: Local, incoming: Option<Local>) -> bool {
    match incoming {
        None => false,
        Some(r) => match root_of.get(&dst) {
            Some(&existing) if existing == r => false,
            Some(_) => root_of.remove(&dst).is_some(),
            None => {
                root_of.insert(dst, r);
                true
            }
        },
    }
}

/// Root of an operand's pointer value, if it is a bare pointer local.
fn operand_root(root_of: &HashMap<Local, Local>, op: &Operand<'_>) -> Option<Local> {
    match op {
        Operand::Copy(p) | Operand::Move(p) if p.projection.is_empty() => {
            root_of.get(&p.local).copied()
        }
        _ => None,
    }
}

/// The last path segment of a call's callee, if it is a direct `FnDef`.
fn callee_name<'tcx>(
    tcx: TyCtxt<'tcx>,
    decls: &IndexVec<Local, LocalDecl<'tcx>>,
    func: &Operand<'tcx>,
) -> Option<String> {
    match func.ty(decls, tcx).kind() {
        ty::FnDef(def_id, _) => Some(tcx.item_name(*def_id).to_string()),
        _ => None,
    }
}

fn is_ptr_arith(name: &str) -> bool {
    matches!(
        name,
        "add" | "offset" | "sub" | "wrapping_add" | "wrapping_offset" | "wrapping_sub"
    )
}

fn is_root_source(name: &str) -> bool {
    matches!(
        name,
        "as_mut_ptr" | "as_ptr" | "as_mut_slice" | "as_slice" | "into_raw" | "as_non_null_ptr"
    )
}

fn is_raw_ptr_local(decls: &IndexVec<Local, LocalDecl<'_>>, local: Local) -> bool {
    matches!(decls[local].ty.kind(), ty::RawPtr(..))
}
