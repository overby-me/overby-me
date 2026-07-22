//! MIR rewriting: instrumentation point 0 (Task B2).
//!
//! `rustc_public` (stable MIR) is read-only, so rewriting drops to a
//! `rustc_driver::Callbacks` that sets `config.override_queries` and wraps
//! the `optimized_mir` query: it clones each local body, injects a call to
//! `cementite::__fec_check_deref(ptr)` before every raw-pointer
//! dereference, and returns the rewritten body — so codegen emits the
//! instrumented version.
//!
//! Modelled on rustc's own `rustc_mir_transform::check_pointers` pass
//! (which inserts alignment/null checks the same way): find indirect
//! raw-pointer places, `split_block` before the access, and make the first
//! half's terminator the injected `Call`, targeting the second half.
//!
//! B2 threads the pointer through as a distinct SSA value (cast to
//! `*const u8`) and the check counts executions and rejects null; B3
//! upgrades the check to a bounds/liveness comparison against the
//! propagated capability (I10).

use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{CRATE_DEF_INDEX, DefId, LocalDefId};
use rustc_index::IndexVec;
use rustc_middle::mir::visit::{PlaceContext, Visitor};
use rustc_middle::mir::{
    BasicBlock, BasicBlockData, Body, CallSource, CastKind, Local, LocalDecl, Location, Operand,
    Place, Rvalue, Statement, StatementKind, Terminator, TerminatorKind, UnwindAction,
};
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::Spanned;
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
    let Some(check_fn) = find_check_fn(tcx) else {
        // cementite not linked / symbol not found: leave the body untouched
        // rather than fail the build.
        return default;
    };
    let mut body = default.clone();
    instrument_body(tcx, &mut body, check_fn);
    tcx.arena.alloc(body)
}

/// Resolves `cementite::__fec_check_deref` to its `DefId` by walking the
/// crate graph and the cementite crate root's module children.
fn find_check_fn(tcx: TyCtxt<'_>) -> Option<DefId> {
    let cnum = tcx
        .crates(())
        .iter()
        .find(|&&c| tcx.crate_name(c).as_str() == "cementite")?;
    let root = DefId {
        krate: *cnum,
        index: CRATE_DEF_INDEX,
    };
    tcx.module_children(root).iter().find_map(|child| {
        if child.ident.as_str() == "__fec_check_deref"
            && let Res::Def(DefKind::Fn, def_id) = child.res
        {
            Some(def_id)
        } else {
            None
        }
    })
}

/// Injects a `__fec_check_deref(ptr)` call before every raw-pointer
/// dereference in `body`.
fn instrument_body<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>, check_fn: DefId) {
    let basic_blocks = body.basic_blocks.as_mut();
    let local_decls = &mut body.local_decls;

    // Reverse order so splitting a block does not disturb the indices of
    // statements we have not visited yet (same discipline as
    // check_pointers).
    for block in basic_blocks.indices().rev() {
        for stmt_index in (0..basic_blocks[block].statements.len()).rev() {
            let location = Location {
                block,
                statement_index: stmt_index,
            };
            let source_info = basic_blocks[block].statements[stmt_index].source_info;

            let mut finder = DerefFinder {
                tcx,
                local_decls,
                found: Vec::new(),
            };
            finder.visit_statement(&basic_blocks[block].statements[stmt_index], location);
            let found = finder.found;

            for pointer in found {
                let new_block = split_block(basic_blocks, location);
                let bd = &mut basic_blocks[block];

                // Cast the found pointer to *const u8 and thread it through
                // as a distinct SSA value (B3 will carry a capability here).
                let u8_ptr = Ty::new_imm_ptr(tcx, tcx.types.u8);
                let cast_local = local_decls.push(LocalDecl::new(u8_ptr, source_info.span));
                bd.statements.push(Statement::new(
                    source_info,
                    StatementKind::Assign(Box::new((
                        Place::from(cast_local),
                        Rvalue::Cast(CastKind::PtrToPtr, Operand::Copy(pointer), u8_ptr),
                    ))),
                ));

                // Destination for the (unit-returning) check call.
                let ret = local_decls.push(LocalDecl::new(tcx.types.unit, source_info.span));

                bd.terminator = Some(Terminator {
                    source_info,
                    kind: TerminatorKind::Call {
                        func: Operand::function_handle(tcx, check_fn, [], source_info.span),
                        args: Box::new([Spanned {
                            node: Operand::Move(Place::from(cast_local)),
                            span: source_info.span,
                        }]),
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
}

/// Collects the base pointer places of raw-pointer dereferences in a
/// statement (indirect places whose base local is a raw pointer).
struct DerefFinder<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    local_decls: &'a IndexVec<Local, LocalDecl<'tcx>>,
    found: Vec<Place<'tcx>>,
}

impl<'tcx> Visitor<'tcx> for DerefFinder<'_, 'tcx> {
    fn visit_place(&mut self, place: &Place<'tcx>, context: PlaceContext, location: Location) {
        if place.is_indirect() {
            let base = Place::from(place.local);
            let base_ty = base.ty(self.local_decls, self.tcx).ty;
            if let ty::RawPtr(..) = base_ty.kind() {
                self.found.push(base);
            }
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
