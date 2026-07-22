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

use rustc_hir::def_id::{DefId, LocalDefId};
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
    fn after_crate_root_parsing(
        &mut self,
        compiler: &rustc_interface::interface::Compiler,
        krate: &mut rustc_ast::Crate,
    ) -> rustc_driver::Compilation {
        // Symbol-level injection (I11, A4b): declare the cementite check
        // entry points as `extern "C"` foreign fns in *this* crate, so the
        // MIR pass calls them as bare symbols with no Cargo dependency edge
        // on cementite. cementite is linked once into the final binary
        // (main.rs `-C link-arg`), resolving the symbols — the ASan model.
        inject_fec_decls(&compiler.sess.psess, krate);
        rustc_driver::Compilation::Continue
    }

    fn config(&mut self, config: &mut rustc_interface::interface::Config) {
        config.override_queries = Some(|_sess, providers| {
            providers.queries.optimized_mir = fec_optimized_mir;
        });
    }
}

/// Parses and appends `unsafe extern "C" { fn __fec_*; }` to the crate AST.
fn inject_fec_decls(psess: &rustc_session::parse::ParseSess, krate: &mut rustc_ast::Crate) {
    const DECLS: &str = "unsafe extern \"C\" {\n\
        fn __fec_check_deref_rooted(fault: *const u8, root: *const u8);\n\
        fn __fec_scope_enter(base: *const u8, len: usize);\n\
        fn __fec_scope_exit(base: *const u8);\n\
    }\n";
    let name = rustc_span::FileName::Custom("fe-c-inject".to_string());
    let Ok(mut parser) = rustc_parse::new_parser_from_source_str(
        psess,
        name,
        DECLS.to_string(),
        rustc_parse::lexer::StripTokens::Nothing,
    ) else {
        return;
    };
    if let Ok(Some(item)) = parser.parse_item(
        rustc_parse::parser::ForceCollect::No,
        rustc_parse::parser::AllowConstBlockItems::No,
    ) {
        krate.items.push(item);
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

/// Resolves the check/scope entry points to the *local* `extern "C"` foreign
/// fns injected by `inject_fec_decls` (A4b): a symbol reference, not a Cargo
/// dependency on cementite. Returns `None` before the decls exist (they are
/// injected only in instrument mode).
fn find_fec_fns(tcx: TyCtxt<'_>) -> Option<FecFns> {
    let mut check = None;
    let mut scope_enter = None;
    let mut scope_exit = None;
    for id in tcx.hir_crate_items(()).foreign_items() {
        let def_id = id.owner_id.to_def_id();
        match tcx.item_name(def_id).as_str() {
            "__fec_check_deref_rooted" => check = Some(def_id),
            "__fec_scope_enter" => scope_enter = Some(def_id),
            "__fec_scope_exit" => scope_exit = Some(def_id),
            _ => {}
        }
    }
    Some(FecFns {
        check: check?,
        scope_enter: scope_enter?,
        scope_exit: scope_exit?,
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
    // address-taken local as a stack region and poison it at its lexical
    // death point (drop glue / StorageDead, else frame teardown), so a pointer
    // that escapes the scope (as the rusqlite-0128 closure does across FFI)
    // resolves as a dead stack region. Opt-in for now: instrumenting *every*
    // address-taken local is impractical without an escape analysis (most
    // never leave the frame) — see STATUS.
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

/// Emits stack scope hooks (I8) at **lexical granularity**: `scope_enter`
/// just after a local's address is first taken, and `scope_exit` at that
/// local's lexical death point. The death point is, in order of preference:
/// the local's `Drop { local }` terminator (a `Drop`-type local like the
/// rusqlite `String` has no `StorageDead`, but its drop glue survives
/// optimization); otherwise its `StorageDead(local)` statement. This matters
/// for the rusqlite-0128 shape, where the borrow's target dies at an inner
/// block's end but the callback fires *later in the same frame* — frame
/// granularity (poison only at `Return`) would miss it. A local with neither
/// death marker (an argument, a `Copy` local whose address escaped, or an
/// NRVO return slot live for the whole frame) falls back to `Return`. Returns
/// how many hooks were injected.
fn instrument_scopes<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>, fns: &FecFns) -> usize {
    let sites = address_taken_sites(body);
    if sites.is_empty() {
        return 0;
    }
    let escaping: HashSet<Local> = sites.iter().map(|&(l, _, _)| l).collect();
    let dead = storage_dead_sites(body, &escaping);
    let drops = drop_sites(body, &escaping);
    let typing_env = body.typing_env(tcx);

    // Region sizes need `tcx.layout_of`; resolve them before the mutable
    // borrow. A local whose layout does not resolve gets no enter (and so no
    // exit either — an unregistered region is simply not checked).
    let mut size_of: HashMap<Local, u64> = HashMap::new();
    for &(local, _, _) in &sites {
        let ty = body.local_decls[local].ty;
        if let Ok(layout) = tcx.layout_of(typing_env.as_query_input(ty)) {
            size_of.insert(local, layout.size.bytes());
        }
    }

    let basic_blocks = body.basic_blocks.as_mut();
    let local_decls = &mut body.local_decls;
    let mut injected = 0usize;

    // Terminator exits: a `scope_exit` before each non-cleanup `Drop { local }`.
    // A Drop-type local (the rusqlite `String`) carries no `StorageDead` in
    // optimized MIR, but its drop glue *is* its lexical death point and
    // survives optimization. Done first: it only rewrites terminators and
    // pushes blocks, leaving the statement indices the mid-block pass relies
    // on untouched.
    for (&local, blocks) in &drops {
        if !size_of.contains_key(&local) {
            continue;
        }
        for &block in blocks {
            let ty = local_decls[local].ty;
            let source_info = basic_blocks[block].terminator().source_info;
            inject_scope_before_return(
                tcx,
                basic_blocks,
                local_decls,
                block,
                local,
                ty,
                fns.scope_exit,
                None,
                source_info,
            );
            injected += 1;
        }
    }

    // Mid-block hooks: a `scope_enter` after each address-take and a lexical
    // `scope_exit` before each `StorageDead` (for locals with no drop glue).
    // Grouped by block and applied in descending statement order, so splitting
    // the block at a higher index never disturbs a lower one we have yet to
    // visit.
    enum Hook {
        Enter(u64),
        Exit,
    }
    let mut mid: std::collections::BTreeMap<BasicBlock, Vec<(usize, Local, Hook)>> =
        std::collections::BTreeMap::new();
    for &(local, block, idx) in &sites {
        if let Some(&size) = size_of.get(&local) {
            mid.entry(block)
                .or_default()
                .push((idx + 1, local, Hook::Enter(size)));
        }
    }
    for (&local, locs) in &dead {
        if !size_of.contains_key(&local) || drops.contains_key(&local) {
            continue;
        }
        for &(block, idx) in locs {
            mid.entry(block).or_default().push((idx, local, Hook::Exit));
        }
    }
    for (block, mut hooks) in mid {
        hooks.sort_by_key(|&(at, _, _)| std::cmp::Reverse(at));
        for (at, local, hook) in hooks {
            let ty = local_decls[local].ty;
            let bd = &basic_blocks[block];
            let source_info = bd
                .statements
                .get(at)
                .or_else(|| bd.statements.get(at.saturating_sub(1)))
                .map(|s| s.source_info)
                .unwrap_or_else(|| bd.terminator().source_info);
            let (func, len) = match hook {
                Hook::Enter(size) => (fns.scope_enter, Some(size)),
                Hook::Exit => (fns.scope_exit, None),
            };
            inject_scope_call(
                tcx,
                basic_blocks,
                local_decls,
                block,
                at,
                local,
                ty,
                func,
                len,
                source_info,
            );
            injected += 1;
        }
    }

    // Frame-granularity fallback for locals with no lexical death point (no
    // drop glue and no `StorageDead` — an argument, a `Copy` local whose
    // address escaped, or an NRVO return slot live for the whole frame):
    // poison before every `Return`, where `&raw const local` is still valid.
    let frame_locals: Vec<Local> = escaping
        .iter()
        .copied()
        .filter(|l| size_of.contains_key(l) && !dead.contains_key(l) && !drops.contains_key(l))
        .collect();
    if !frame_locals.is_empty() {
        let return_blocks: Vec<BasicBlock> = basic_blocks
            .indices()
            .filter(|&b| matches!(basic_blocks[b].terminator().kind, TerminatorKind::Return))
            .collect();
        for rb in return_blocks {
            for &local in &frame_locals {
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
    }

    injected
}

/// The `StorageDead(local)` statement locations for each given local — the
/// lexical end of that local's stack storage, where its scope region is
/// poisoned. Optimized MIR retains these for address-escaping locals (the
/// ones that must stay in memory rather than being promoted to SSA), which is
/// what makes lexical-scope granularity available here.
fn storage_dead_sites(
    body: &Body<'_>,
    locals: &HashSet<Local>,
) -> std::collections::BTreeMap<Local, Vec<(BasicBlock, usize)>> {
    let mut sites: std::collections::BTreeMap<Local, Vec<(BasicBlock, usize)>> =
        std::collections::BTreeMap::new();
    for block in body.basic_blocks.indices() {
        for (i, stmt) in body.basic_blocks[block].statements.iter().enumerate() {
            if let StatementKind::StorageDead(local) = stmt.kind
                && locals.contains(&local)
            {
                sites.entry(local).or_default().push((block, i));
            }
        }
    }
    sites
}

/// The non-cleanup blocks that end in `Drop { local }` for each given local —
/// the drop glue of a `Drop`-type local (the rusqlite `String`), which is its
/// lexical death point and survives MIR optimization even when `StorageDead`
/// does not. Cleanup (unwind-path) drops are skipped: the whole frame is
/// unwinding there, and the injected call would carry the wrong unwind action.
fn drop_sites(
    body: &Body<'_>,
    locals: &HashSet<Local>,
) -> std::collections::BTreeMap<Local, Vec<BasicBlock>> {
    let mut sites: std::collections::BTreeMap<Local, Vec<BasicBlock>> =
        std::collections::BTreeMap::new();
    for block in body.basic_blocks.indices() {
        let bd = &body.basic_blocks[block];
        if bd.is_cleanup {
            continue;
        }
        if let TerminatorKind::Drop { place, .. } = &bd.terminator().kind
            && place.projection.is_empty()
            && locals.contains(&place.local)
        {
            sites.entry(place.local).or_default().push(block);
        }
    }
    sites
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
