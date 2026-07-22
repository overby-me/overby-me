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
    AggregateKind, BasicBlock, BasicBlockData, BinOp, Body, CallSource, CastKind,
    Const as MirConst, ConstOperand, Local, LocalDecl, Location, Operand, Place, ProjectionElem,
    RawPtrKind, Rvalue, SourceInfo, Statement, StatementKind, Terminator, TerminatorKind,
    UnwindAction,
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
        fn __fec_check_dealloc_reachable(fault: *const u8, root: *const u8, read_line: usize);\n\
        fn __fec_ensure(fault: *const u8, root: *const u8, size: usize, mint_line: usize);\n\
        fn __fec_check_extent(fault: *const u8, root: *const u8, size: usize);\n\
        fn __fec_scope_enter(base: *const u8, len: usize, site: usize);\n\
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
    /// `__fec_check_dealloc_reachable(fault, root)` — case-mode re-check.
    dealloc_reachable: DefId,
    /// `__fec_ensure(fault, root, size)` — raw→safe cast check (point 1).
    ensure: DefId,
    /// `__fec_check_extent(fault, root, size)` — projected-deref extent check.
    check_extent: DefId,
    /// `__fec_scope_enter(base, len, site)`.
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
    let mut dealloc_reachable = None;
    let mut ensure = None;
    let mut check_extent = None;
    let mut scope_enter = None;
    let mut scope_exit = None;
    for id in tcx.hir_crate_items(()).foreign_items() {
        let def_id = id.owner_id.to_def_id();
        match tcx.item_name(def_id).as_str() {
            "__fec_check_deref_rooted" => check = Some(def_id),
            "__fec_check_dealloc_reachable" => dealloc_reachable = Some(def_id),
            "__fec_ensure" => ensure = Some(def_id),
            "__fec_check_extent" => check_extent = Some(def_id),
            "__fec_scope_enter" => scope_enter = Some(def_id),
            "__fec_scope_exit" => scope_exit = Some(def_id),
            _ => {}
        }
    }
    Some(FecFns {
        check: check?,
        dealloc_reachable: dealloc_reachable?,
        ensure: ensure?,
        check_extent: check_extent?,
        scope_enter: scope_enter?,
        scope_exit: scope_exit?,
    })
}

/// Injects cementite check/scope calls into `body`: a rooted deref check at
/// every raw access (I10) and stack scope enter/exit hooks for
/// address-taken locals (I8). Returns how many calls were injected.
///
/// `FEC_MODE=through` selects **through** mode (Task C1): the exhaustive mode
/// that also checks **safe-pointer** dereferences (`*r` where `r: &T`), which
/// `case` elides as vetted at the cast (the one bolded row in the both-modes
/// table). In `case` mode a safe deref is instead re-checked *only* when it is
/// **dealloc-reachable** — it follows a possibly-freeing call — via the
/// temporal-only heap re-check (point 4, I6; Task C2, trace `rustsec-2021-0130`
/// §3.4). Raw dereferences are checked in both modes.
fn instrument_body<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>, fns: &FecFns) -> usize {
    let check_fn = fns.check;
    let through = matches!(std::env::var("FEC_MODE").as_deref(), Ok("through"));
    // In `case` mode, safe-pointer derefs are elided except those reachable
    // from a (conservatively, any) call — where an intervening heap free could
    // have invalidated the reference. `through` checks them all, so it needs no
    // such set.
    let dealloc_reachable = if through {
        HashSet::new()
    } else {
        dealloc_reachable_blocks(body)
    };
    // Provenance first, on the un-mutated body: which local holds each
    // pointer's derivation root (I10). Keyed on original locals, which the
    // block-splitting below never renumbers.
    let roots = compute_roots(tcx, body);
    let mut injected = 0usize;

    // Pass 0: stack scope hooks (I8). Register each *escaping* address-taken
    // local as a stack region and poison it at its lexical death point (drop
    // glue / StorageDead, else frame teardown), so a pointer that escapes the
    // scope (as the rusqlite-0128 closure does across FFI) resolves as a dead
    // stack region. Default-on: the escape analysis (`escaping_locals`) keeps
    // this to the locals whose address is laundered out of the frame, so a
    // raw-pointer-heavy crate like hashbrown gets few or no hooks and neither
    // times out nor false-aborts.
    injected += instrument_scopes(tcx, body, fns);

    // Pass 0.5 (both modes): raw->safe cast ensures (point 1, §3.1). Validate a
    // `&*p` reference's extent is in-bounds and live *at the cast*, resolving
    // from the raw pointer's derivation root (I10). This is what lets `case`
    // soundly elide the reference's later dereferences; `through` needs it too,
    // because the reborrow's own local is a reference (not tracked by
    // `compute_roots`), so the later deref check would resolve the faulting
    // address — off the end for an OOB cast — and find nothing.
    injected += instrument_cast_ensures(tcx, body, &roots, fns.ensure);

    // For the projected-deref extent check (point 0): the accessed subobject's
    // layout size. Computed before the body is split into mutable borrows.
    let typing_env = body.typing_env(tcx);

    let basic_blocks = body.basic_blocks.as_mut();
    let local_decls = &mut body.local_decls;

    // Pass 1: pointer-write intrinsic calls. Real unsafe code (and
    // smallvec::insert_many) writes through raw pointers with
    // `ptr::write`/`ptr::copy`, which are Call terminators, not `*p = v`
    // deref places — so check the destination pointer before the call.
    let original_blocks: Vec<BasicBlock> = basic_blocks.indices().collect();
    for block in original_blocks {
        injected += instrument_write_call(
            tcx,
            basic_blocks,
            local_decls,
            &roots,
            block,
            fns,
            typing_env,
        );
    }

    // Pass 2: direct dereference places (`*p`). In `through` mode this covers
    // safe references too (`*r`, `r: &T`); otherwise only raw pointers.
    // Reverse order so splitting a block does not disturb the indices of
    // statements we have not visited yet (same discipline as check_pointers).
    for block in basic_blocks.indices().rev() {
        for stmt_index in (0..basic_blocks[block].statements.len()).rev() {
            let location = Location {
                block,
                statement_index: stmt_index,
            };
            let source_info = basic_blocks[block].statements[stmt_index].source_info;

            // In `case` mode, only re-check safe derefs in dealloc-reachable
            // blocks; `through` checks them everywhere.
            let check_safe = through || dealloc_reachable.contains(&block);
            let mut finder = DerefFinder {
                local_decls,
                check_safe,
                found: Vec::new(),
            };
            finder.visit_statement(&basic_blocks[block].statements[stmt_index], location);
            let found = finder.found;

            for (place, is_raw) in found {
                let base_local = place.local;
                // The pointer's derivation root, or itself when propagation
                // found none (unpropagated fallback: resolves the access's
                // own allocation — never a false positive, at worst a false
                // negative when provenance is lost).
                let root_local = roots.get(&base_local).copied().unwrap_or(base_local);
                injected += 1;

                // Raw derefs, and every deref in `through` mode, get the full
                // check; a `case`-mode safe deref gets the temporal-only heap
                // re-check (it aborts on a dead heap allocation but passes a
                // dead stack scope, which `case` elides).
                let check = if is_raw || through {
                    check_fn
                } else {
                    fns.dealloc_reachable
                };

                let projected = is_simple_projected_place(&place);
                // A raw deref (either mode) or a `through` safe deref gets the
                // extent check over the whole accessed place (spatial + temporal)
                // when its layout size is known — the fault is `p` for a
                // whole-object `*p` and `p + offset` for a projected `(*p).f`,
                // and the check verifies `[fault, fault + size)`, so an access
                // that *starts* in bounds but runs past the end of a mis-cast
                // allocation is caught, not just one whose start is off the end.
                // Case-mode safe derefs keep the temporal-only re-check (their
                // spatial extent was vetted at the cast ensure); an unsized
                // access (`layout_of` fails) keeps the single-address base check.
                let extent_size = if check == check_fn {
                    let ty = place.ty(local_decls, tcx).ty;
                    tcx.layout_of(typing_env.as_query_input(ty))
                        .ok()
                        .map(|l| l.size.bytes())
                } else {
                    None
                };
                let actual_check = if extent_size.is_some() {
                    fns.check_extent
                } else {
                    check
                };

                let new_block = split_block(basic_blocks, location);
                let bd = &mut basic_blocks[block];

                let u8_ptr = Ty::new_imm_ptr(tcx, tcx.types.u8);
                // Fault on the *accessed* address: for a simple projected place
                // `(*p).f` that is `&raw const place` = `p + offset(f)` (a
                // tighter check than the base `p`); for a whole-object `*p` it
                // is just `p`. The root pointer is threaded as a distinct value.
                let fault_arg = if projected {
                    raw_const_place_as_u8(tcx, local_decls, bd, &place, u8_ptr, source_info)
                } else {
                    cast_to_u8_ptr(local_decls, bd, base_local, u8_ptr, source_info)
                };
                let root_arg = cast_to_u8_ptr(local_decls, bd, root_local, u8_ptr, source_info);

                let ret = local_decls.push(LocalDecl::new(tcx.types.unit, source_info.span));
                let mut args = vec![
                    Spanned {
                        node: Operand::Move(Place::from(fault_arg)),
                        span: source_info.span,
                    },
                    Spanned {
                        node: Operand::Move(Place::from(root_arg)),
                        span: source_info.span,
                    },
                ];
                // The case-mode dealloc-reachable re-check also carries the
                // source line of this dereference, so a heap use-after-free
                // report can name where the dangling reference was read
                // (`read_at`) — half of the both-sites debuggability rule
                // (§2). Precisely naming the *free* line needs the deferred
                // `nofree` callgraph: under the conservative "any call frees"
                // reachability, an innocent intervening call would be named.
                if actual_check == fns.dealloc_reachable {
                    let read_line = tcx
                        .sess
                        .source_map()
                        .lookup_char_pos(source_info.span.lo())
                        .line as u64;
                    args.push(Spanned {
                        node: Operand::Constant(Box::new(ConstOperand {
                            span: source_info.span,
                            user_ty: None,
                            const_: MirConst::from_usize(tcx, read_line),
                        })),
                        span: source_info.span,
                    });
                } else if let Some(size) = extent_size {
                    args.push(Spanned {
                        node: Operand::Constant(Box::new(ConstOperand {
                            span: source_info.span,
                            user_ty: None,
                            const_: MirConst::from_usize(tcx, size),
                        })),
                        span: source_info.span,
                    });
                }
                bd.terminator = Some(Terminator {
                    source_info,
                    kind: TerminatorKind::Call {
                        func: Operand::function_handle(tcx, actual_check, [], source_info.span),
                        args: args.into_boxed_slice(),
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
/// (`ptr::write`/`copy`/`copy_nonoverlapping`/`write_bytes`), injects a check on
/// the destination pointer *before* the call: the original call moves into a
/// fresh block, and this block's terminator becomes the check, targeting it.
///
/// The check is `__fec_check_extent(dst, root, size)` when the written extent's
/// byte size is known — `count * size_of::<T>()`, with `count` = 1 for the
/// single-element writes and the intrinsic's `count` argument for
/// `copy`/`copy_nonoverlapping`/`write_bytes` — so a `ptr::copy` whose
/// destination starts in bounds but whose extent overruns the end of a mis-cast
/// allocation is caught, not only one whose start is off the end. It falls back
/// to the single-address `__fec_check_deref_rooted(dst, root)` when the element
/// type is unsized (no static `size_of`).
fn instrument_write_call<'tcx>(
    tcx: TyCtxt<'tcx>,
    basic_blocks: &mut IndexVec<BasicBlock, BasicBlockData<'tcx>>,
    local_decls: &mut IndexVec<Local, LocalDecl<'tcx>>,
    roots: &HashMap<Local, Local>,
    block: BasicBlock,
    fns: &FecFns,
    typing_env: ty::TypingEnv<'tcx>,
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

    // The written extent's element size (`size_of::<T>()` for a `*mut T`
    // destination), if statically known — None for an unsized element.
    let elem_size = local_decls[fault_local]
        .ty
        .builtin_deref(true)
        .and_then(|pointee| tcx.layout_of(typing_env.as_query_input(pointee)).ok())
        .map(|layout| layout.size.bytes());
    // The `count` operand for the count-taking intrinsics, captured before the
    // terminator is moved (it is one of the call's own arguments, live here).
    let count_op = ptr_write_count_arg(&name).and_then(|ci| args.get(ci).map(|a| a.node.clone()));

    // Move the original write call into a fresh block.
    let orig = basic_blocks[block].terminator.take();
    let is_cleanup = basic_blocks[block].is_cleanup;
    let new_block = basic_blocks.push(BasicBlockData::new_stmts(Vec::new(), orig, is_cleanup));

    let u8_ptr = Ty::new_imm_ptr(tcx, tcx.types.u8);
    let bd = &mut basic_blocks[block];
    let fault_arg = cast_to_u8_ptr(local_decls, bd, fault_local, u8_ptr, source_info);
    let root_arg = cast_to_u8_ptr(local_decls, bd, root_local, u8_ptr, source_info);

    let mut args_out = vec![
        Spanned {
            node: Operand::Move(Place::from(fault_arg)),
            span: source_info.span,
        },
        Spanned {
            node: Operand::Move(Place::from(root_arg)),
            span: source_info.span,
        },
    ];
    // Extent check when the element size is known; otherwise the base check.
    let check = match elem_size {
        Some(sz) => {
            let size_operand = match count_op {
                // `size = count * size_of::<T>()` (materialized when non-trivial).
                Some(count) if sz != 1 => {
                    let size_local =
                        local_decls.push(LocalDecl::new(tcx.types.usize, source_info.span));
                    let elem_const = Operand::Constant(Box::new(ConstOperand {
                        span: source_info.span,
                        user_ty: None,
                        const_: MirConst::from_usize(tcx, sz),
                    }));
                    bd.statements.push(Statement::new(
                        source_info,
                        StatementKind::Assign(Box::new((
                            Place::from(size_local),
                            Rvalue::BinaryOp(BinOp::Mul, Box::new((count, elem_const))),
                        ))),
                    ));
                    Operand::Move(Place::from(size_local))
                }
                // `size = count` (element size 1).
                Some(count) => count,
                // Single-element write: `size = size_of::<T>()`.
                None => Operand::Constant(Box::new(ConstOperand {
                    span: source_info.span,
                    user_ty: None,
                    const_: MirConst::from_usize(tcx, sz),
                })),
            };
            args_out.push(Spanned {
                node: size_operand,
                span: source_info.span,
            });
            fns.check_extent
        }
        None => fns.check,
    };
    let ret = local_decls.push(LocalDecl::new(tcx.types.unit, source_info.span));

    bd.terminator = Some(Terminator {
        source_info,
        kind: TerminatorKind::Call {
            func: Operand::function_handle(tcx, check, [], source_info.span),
            args: args_out.into_boxed_slice(),
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

/// The index of the element-`count` argument for the count-taking write
/// intrinsics (`copy(src, dst, count)`, `write_bytes(dst, val, count)`), which
/// write `count * size_of::<T>()` bytes. `None` for the single-element writes.
fn ptr_write_count_arg(name: &str) -> Option<usize> {
    match name {
        "write_bytes" | "copy" | "copy_nonoverlapping" => Some(2),
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

/// Injects `__fec_ensure(ref_ptr, root, size)` before every reborrow of a raw
/// pointer into a safe reference — a raw->safe cast (point 1, §3.1). Both the
/// whole-object `_ref = &*raw` and a **field reborrow** `_ref = &(*raw).f` are
/// handled: a `Deref` of a raw-pointer local followed by only `Field`/`Downcast`
/// projections. `ref_ptr` is the address of the reborrowed place (`p` for `&*p`,
/// `p + offset(f)` for `&(*p).f`), materialized as `&raw const place`, and
/// `size` is that place's layout size — so a field reborrow is bounds-checked
/// against the *field's* extent. Covering field reborrows closes a `case`-mode
/// soundness gap: `case` elides the reference's later dereferences, so an
/// unvetted field reborrow off the end of a mis-cast base (`base.add(k) as
/// *const Struct` then `&(*p).f`) would never be spatially checked (I1 — a
/// missing visit, not a policy elision). A projection with a *further* `Deref`
/// or an `Index` changes the derivation root, so those are left to the deref
/// checks. Returns how many ensures were injected.
fn instrument_cast_ensures<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &mut Body<'tcx>,
    roots: &HashMap<Local, Local>,
    ensure_fn: DefId,
) -> usize {
    let typing_env = body.typing_env(tcx);
    let basic_blocks = body.basic_blocks.as_mut();
    let local_decls = &mut body.local_decls;
    let mut injected = 0usize;

    for block in basic_blocks.indices().rev() {
        for idx in (0..basic_blocks[block].statements.len()).rev() {
            // Read the cast site (immutable) before splitting/injecting.
            let Some((place, raw, size, source_info)) = ({
                let stmt = &basic_blocks[block].statements[idx];
                if let StatementKind::Assign(boxed) = &stmt.kind
                    && let Rvalue::Ref(_, _, place) = &boxed.1
                    && is_reborrow_of_raw(local_decls, place)
                    && let Ok(layout) =
                        tcx.layout_of(typing_env.as_query_input(place.ty(local_decls, tcx).ty))
                {
                    Some((*place, place.local, layout.size.bytes(), stmt.source_info))
                } else {
                    None
                }
            }) else {
                continue;
            };

            let root = roots.get(&raw).copied().unwrap_or(raw);
            let new_block = split_block(
                basic_blocks,
                Location {
                    block,
                    statement_index: idx,
                },
            );
            let bd = &mut basic_blocks[block];
            let u8_ptr = Ty::new_imm_ptr(tcx, tcx.types.u8);
            // The reborrowed place's address: `p` for `&*p`, `p + offset(f)`
            // for `&(*p).f`. `&raw const place` computes it uniformly.
            let fault_arg =
                raw_const_place_as_u8(tcx, local_decls, bd, &place, u8_ptr, source_info);
            let root_arg = cast_to_u8_ptr(local_decls, bd, root, u8_ptr, source_info);
            // The mint site: the source line of this raw->safe cast, recorded on
            // the resolved allocation so a later heap use-after-free names where
            // the dangling reference was born (`minted_at`, trace -0130).
            let mint_line = tcx
                .sess
                .source_map()
                .lookup_char_pos(source_info.span.lo())
                .line as u64;
            let ret = local_decls.push(LocalDecl::new(tcx.types.unit, source_info.span));
            bd.terminator = Some(Terminator {
                source_info,
                kind: TerminatorKind::Call {
                    func: Operand::function_handle(tcx, ensure_fn, [], source_info.span),
                    args: Box::new([
                        Spanned {
                            node: Operand::Move(Place::from(fault_arg)),
                            span: source_info.span,
                        },
                        Spanned {
                            node: Operand::Move(Place::from(root_arg)),
                            span: source_info.span,
                        },
                        Spanned {
                            node: Operand::Constant(Box::new(ConstOperand {
                                span: source_info.span,
                                user_ty: None,
                                const_: MirConst::from_usize(tcx, size),
                            })),
                            span: source_info.span,
                        },
                        Spanned {
                            node: Operand::Constant(Box::new(ConstOperand {
                                span: source_info.span,
                                user_ty: None,
                                const_: MirConst::from_usize(tcx, mint_line),
                            })),
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
            injected += 1;
        }
    }
    injected
}

/// True if `place` is a reborrow eligible for a raw->safe cast ensure: a
/// `Deref` of a raw-pointer local, followed by only `Field`/`Downcast`
/// projections (a whole-object `&*p` or a field reborrow `&(*p).f`). A further
/// `Deref` or an `Index` after the first would change the derivation root, so
/// those are left to the deref checks.
fn is_reborrow_of_raw<'tcx>(
    local_decls: &IndexVec<Local, LocalDecl<'tcx>>,
    place: &Place<'tcx>,
) -> bool {
    if place.projection.is_empty()
        || !matches!(place.projection[0], ProjectionElem::Deref)
        || !is_raw_ptr_local(local_decls, place.local)
    {
        return false;
    }
    place.projection[1..]
        .iter()
        .all(|p| matches!(p, ProjectionElem::Field(..) | ProjectionElem::Downcast(..)))
}

/// Emits `_p = &raw const place; _u8 = _p as *const u8` and returns the u8
/// pointer local — the address of `place` itself (`p` for `*p`, `p + offset(f)`
/// for `(*p).f`), so a field reborrow is bounds-checked against the field's
/// extent, not the whole object's.
fn raw_const_place_as_u8<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &mut IndexVec<Local, LocalDecl<'tcx>>,
    bd: &mut BasicBlockData<'tcx>,
    place: &Place<'tcx>,
    u8_ptr: Ty<'tcx>,
    source_info: SourceInfo,
) -> Local {
    let elem_ty = place.ty(local_decls, tcx).ty;
    let raw_ty = Ty::new_ptr(tcx, elem_ty, Mutability::Not);
    let addr = local_decls.push(LocalDecl::new(raw_ty, source_info.span));
    bd.statements.push(Statement::new(
        source_info,
        StatementKind::Assign(Box::new((
            Place::from(addr),
            Rvalue::RawPtr(RawPtrKind::Const, *place),
        ))),
    ));
    cast_to_u8_ptr(local_decls, bd, addr, u8_ptr, source_info)
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
    let escapes = escaping_locals(tcx, body);
    if escapes.is_empty() {
        return 0;
    }
    let sites: Vec<(Local, BasicBlock, usize)> = address_taken_sites(body)
        .into_iter()
        .filter(|&(l, _, _)| escapes.contains_key(&l))
        .collect();
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
                &[],
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
            // scope_enter carries [region size, escape site line]; scope_exit
            // takes only the base.
            let (func, extra): (DefId, Vec<u64>) = match hook {
                Hook::Enter(size) => (fns.scope_enter, vec![size, u64::from(escapes[&local])]),
                Hook::Exit => (fns.scope_exit, Vec::new()),
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
                &extra,
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
                    &[],
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

/// The address-taken locals whose address genuinely *escapes* the frame, and
/// so are worth a scope hook. A forward taint tracks, for each pointer local,
/// the stack locals whose address it may hold: seeded at address-of and
/// propagated through pointer-preserving copies, casts, reborrows and
/// `offset`. A local escapes when a tainted pointer reaches an escape sink:
/// a **cast to an integer** (laundering a stack address into a `static` or a
/// heap closure), or a **pointer argument to a foreign `extern "C"` call**
/// (handing it straight out to C, as the rusqlite-0128 closure box is — I9 /
/// trace F6). A `&mut x` merely handed to a Rust method that keeps it in-frame
/// (the hashbrown common case) reaches neither, so default-on scope hooks stay
/// cheap and false-positive-free.
///
/// Returns each escaping local mapped to the **source line of its escape
/// sink** (the `SiteId` a use-after-scope report names as `escaped_at`, so the
/// developer sees where the address was handed out, not just the callback the
/// abort fires in — trace F7).
fn escaping_locals<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> HashMap<Local, u32> {
    let mut taint: HashMap<Local, HashSet<Local>> = HashMap::new();
    for bb in body.basic_blocks.iter() {
        for stmt in &bb.statements {
            if let StatementKind::Assign(boxed) = &stmt.kind {
                let (dst, rv) = &**boxed;
                if dst.projection.is_empty()
                    && let Rvalue::Ref(_, _, p) | Rvalue::RawPtr(_, p) = rv
                    && p.projection.is_empty()
                {
                    taint.entry(dst.local).or_default().insert(p.local);
                }
            }
        }
    }
    if taint.is_empty() {
        return HashMap::new();
    }

    // Propagate through pointer-preserving moves/casts/offsets to a fixpoint.
    for _ in 0..8 {
        let mut changed = false;
        for bb in body.basic_blocks.iter() {
            for stmt in &bb.statements {
                if let StatementKind::Assign(boxed) = &stmt.kind {
                    let (dst, rv) = &**boxed;
                    if !dst.projection.is_empty() {
                        continue;
                    }
                    let src = match rv {
                        Rvalue::Use(op, _) => op.place(),
                        Rvalue::Cast(_, op, ty)
                            if matches!(ty.kind(), ty::RawPtr(..) | ty::Ref(..)) =>
                        {
                            op.place()
                        }
                        Rvalue::BinaryOp(BinOp::Offset, boxed) => boxed.0.place(),
                        // Reborrow `&*base` / `&raw *base`: a new reference to
                        // the same target, the form `&local as *const T`
                        // lowers to. Preserves the address, so propagate taint.
                        Rvalue::Ref(_, _, p) | Rvalue::RawPtr(_, p)
                            if p.projection.len() == 1
                                && matches!(p.projection[0], ProjectionElem::Deref) =>
                        {
                            Some(Place::from(p.local))
                        }
                        _ => None,
                    };
                    if let Some(src) = src
                        && let Some(seed) = taint.get(&src.local).cloned()
                    {
                        let entry = taint.entry(dst.local).or_default();
                        for l in seed {
                            changed |= entry.insert(l);
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    let mut escaping: HashMap<Local, u32> = HashMap::new();
    let sm = tcx.sess.source_map();
    for bb in body.basic_blocks.iter() {
        for stmt in &bb.statements {
            if let StatementKind::Assign(boxed) = &stmt.kind {
                let (_, rv) = &**boxed;
                let line = || sm.lookup_char_pos(stmt.source_info.span.lo()).line as u32;
                // Sink: a pointer-to-integer cast launders the address out.
                if let Rvalue::Cast(_, op, ty) = rv
                    && ty.is_integral()
                    && let Some(p) = op.place()
                    && let Some(seed) = taint.get(&p.local)
                {
                    let l0 = line();
                    for &l in seed {
                        escaping.entry(l).or_insert(l0);
                    }
                }
                // Sink: a pointer captured by-move into a closure. The closure
                // is heap-boxed and outlives the frame (rusqlite hands such a
                // box to SQLite via `create_scalar_function`), carrying the
                // stack address out with it.
                if let Rvalue::Aggregate(kind, operands) = rv
                    && matches!(**kind, AggregateKind::Closure(..))
                {
                    for op in operands.iter() {
                        if let Some(p) = op.place()
                            && let Some(seed) = taint.get(&p.local)
                        {
                            let l0 = line();
                            for &l in seed {
                                escaping.entry(l).or_insert(l0);
                            }
                        }
                    }
                }
            }
        }
        // Sink: a pointer argument to a foreign `extern "C"` call — the
        // outbound FFI escape (I9 / trace F6) the rusqlite closure box takes.
        if let TerminatorKind::Call { func, args, .. } = &bb.terminator().kind
            && is_foreign_call(tcx, &body.local_decls, func)
        {
            let line = sm
                .lookup_char_pos(bb.terminator().source_info.span.lo())
                .line as u32;
            for arg in args.iter() {
                if let Some(p) = arg.node.place()
                    && let Some(seed) = taint.get(&p.local)
                {
                    for &l in seed {
                        escaping.entry(l).or_insert(line);
                    }
                }
            }
        }
    }
    escaping
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
    extra: &[u64],
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
        extra,
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
    extra: &[u64],
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
        extra,
        source_info,
    );
}

/// Builds `func(&raw const local as *const u8, extra…)` as `block`'s
/// terminator, targeting `target`. `&raw const local` is cast to `*const u8`
/// and threaded through; each `extra` value is appended as a `usize` constant
/// argument (`[len, site]` for `scope_enter`, none for `scope_exit`).
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
    extra: &[u64],
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
    for &val in extra {
        args.push(Spanned {
            node: Operand::Constant(Box::new(ConstOperand {
                span: source_info.span,
                user_ty: None,
                const_: MirConst::from_usize(tcx, val),
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

/// The basic blocks reachable (following CFG edges) from the target of some
/// `Call` terminator — the blocks that execute *after* a call. In `case` mode
/// a safe dereference here is re-checked (point 4, I6, trace §3.4): a preceding
/// call may have freed the referent. Conservative — every call is treated as
/// possibly-freeing; a precise `nofree` callgraph would prune this (the abort
/// is already gated to dead *heap* allocations at runtime, so the imprecision
/// costs extra checks, never false positives).
fn dealloc_reachable_blocks(body: &Body<'_>) -> HashSet<BasicBlock> {
    let mut reachable: HashSet<BasicBlock> = HashSet::new();
    let mut worklist: Vec<BasicBlock> = Vec::new();
    for block in body.basic_blocks.indices() {
        if let TerminatorKind::Call {
            target: Some(t), ..
        } = &body.basic_blocks[block].terminator().kind
            && reachable.insert(*t)
        {
            worklist.push(*t);
        }
    }
    while let Some(b) = worklist.pop() {
        for succ in body.basic_blocks[b].terminator().successors() {
            if reachable.insert(succ) {
                worklist.push(succ);
            }
        }
    }
    reachable
}

/// Collects the dereference places in a statement (indirect places), each
/// tagged `is_raw`. Always collects raw-pointer bases; safe-reference bases
/// (`*r`, `r: &T`) only when `check_safe` (through mode, or a dealloc-reachable
/// block in case mode). The full place (not just its base local) is kept so the
/// check can fault on the *accessed* address — `p + offset(f)` for `(*p).f` —
/// not merely the base `p`.
struct DerefFinder<'a, 'tcx> {
    local_decls: &'a IndexVec<Local, LocalDecl<'tcx>>,
    check_safe: bool,
    found: Vec<(Place<'tcx>, bool)>,
}

impl<'tcx> Visitor<'tcx> for DerefFinder<'_, 'tcx> {
    fn visit_place(&mut self, place: &Place<'tcx>, context: PlaceContext, location: Location) {
        if place.is_indirect() {
            if is_raw_ptr_local(self.local_decls, place.local) {
                self.found.push((*place, true));
            } else if self.check_safe && is_ref_local(self.local_decls, place.local) {
                self.found.push((*place, false));
            }
        }
        self.super_place(place, context, location);
    }
}

/// True if `place` is a simple projected dereference — a single leading `Deref`
/// followed by field/index/downcast projections and **no further `Deref`**
/// (`(*p).f`, `(*p)[i]`, `(*p).f.g`). For these the *accessed* address is
/// `&raw const place` (`p + offset`), which is a tighter fault than the base
/// pointer `p`: it catches a direct projected access off the end of a mis-cast
/// base, which the base check would miss (a point-0 precision gap). A nested
/// `Deref` would change the derivation root, so those keep the base fault.
fn is_simple_projected_place(place: &Place<'_>) -> bool {
    place.projection.len() > 1
        && matches!(place.projection[0], ProjectionElem::Deref)
        && place.projection[1..]
            .iter()
            .all(|p| !matches!(p, ProjectionElem::Deref))
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

/// Whether a direct call targets a foreign (`extern "C"`) function — an
/// outbound FFI edge where a pointer argument leaves Rust's control (I9 /
/// trace F6). Indirect calls through a function pointer (the shape C uses to
/// re-enter a Rust trampoline) are not foreign items and are not matched here.
fn is_foreign_call<'tcx>(
    tcx: TyCtxt<'tcx>,
    decls: &IndexVec<Local, LocalDecl<'tcx>>,
    func: &Operand<'tcx>,
) -> bool {
    match func.ty(decls, tcx).kind() {
        ty::FnDef(def_id, _) => tcx.is_foreign_item(*def_id),
        _ => false,
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

/// Whether a local is a safe reference (`&T` / `&mut T`) — a deref through it
/// is a safe-pointer dereference, checked only in `through` mode.
fn is_ref_local(decls: &IndexVec<Local, LocalDecl<'_>>, local: Local) -> bool {
    matches!(decls[local].ty.kind(), ty::Ref(..))
}
