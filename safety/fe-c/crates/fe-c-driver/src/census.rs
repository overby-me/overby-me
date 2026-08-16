//! The visitation census (Task A5): a read-only inventory of every place a
//! later checking pass will need to visit, proving I1 (total visitation)
//! before any rewriting exists.

use rustc_public::CrateDef;
use rustc_public::mir::visit::{Location, PlaceContext};
use rustc_public::mir::{
    Body, CastKind, MirVisitor, Place, ProjectionElem, Rvalue, Terminator, TerminatorKind,
};
use rustc_public::ty::{Abi, RigidTy, Ty, TyKind};

/// Per-crate census totals.
#[derive(Default, Debug)]
pub struct Census {
    /// MIR bodies visited.
    pub bodies: u64,
    /// Locals whose type is a raw pointer (`*const T` / `*mut T`).
    pub raw_ptr_locals: u64,
    /// Locals whose type is a reference (`&T` / `&mut T`).
    pub ref_locals: u64,
    /// Dereference projections encountered (reads and writes).
    pub derefs: u64,
    /// Dereferences specifically through a raw pointer.
    pub raw_derefs: u64,
    /// `&*p` / `&mut *p` reborrows and pointer-to-reference casts:
    /// the raw->safe boundary (instrumentation point 1).
    pub raw_to_safe_casts: u64,
    /// Pointer<->integer casts (provenance-losing).
    pub ptr_int_casts: u64,
    /// Call terminators to functions across an FFI (`extern` ABI) edge.
    pub ffi_calls: u64,
    /// Bodies whose analysis panicked and were skipped. Must stay
    /// observable: a non-zero value means the census under-counts.
    pub skipped_bodies: u64,
    /// Raw dereferences whose provenance the B1 dataflow traced to a
    /// derivation root (I10).
    pub rooted_derefs: u64,
    /// Raw dereferences where provenance propagation was lost.
    pub lost_derefs: u64,
}

/// Runs the census over every local MIR body and reports it.
pub fn run() -> Result<(), String> {
    let mut census = Census::default();
    let krate = rustc_public::local_crate().name;
    let prov_fn_filter = std::env::var("FEC_PROV_FN").ok().filter(|s| !s.is_empty());

    for item in rustc_public::all_local_items() {
        // The census must never break the compilation it rides on: any
        // panic while fetching or walking one body is contained, that body
        // is counted as skipped, and the rest proceed (I1 stays honest —
        // an under-count is visible, not silent).
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let body = item.body()?;
            let mut sub = Census::default();
            let mut visitor = CensusVisitor {
                census: &mut sub,
                body: &body,
            };
            visitor.visit_body(&body);
            sub.bodies = 1;

            // B1 capability propagation (I10): trace each raw deref to a
            // derivation root or record the loss.
            let (prov, _facts) = crate::provenance::analyze(&body);
            sub.rooted_derefs = prov.rooted_derefs;
            sub.lost_derefs = prov.lost_derefs;

            // Per-function provenance dump for spot-checking a specific
            // function (e.g. FEC_PROV_FN=insert_many): print the derivation
            // roots that reach a write in any matching body.
            if let Some(want) = prov_fn_filter.as_deref() {
                let name = item.name();
                if name.contains(want) && prov.rooted_writes > 0 {
                    println!(
                        "fe-c-prov fn={name} rooted_writes={} write_roots={:?}",
                        prov.rooted_writes, prov.write_roots
                    );
                }
            }
            Some(sub)
        }));
        match outcome {
            Ok(Some(sub)) => census.add(&sub),
            Ok(None) => {} // no MIR body (foreign / intrinsic)
            Err(_) => census.skipped_bodies += 1,
        }
    }

    report(&krate, &census);
    Ok(())
}

impl Census {
    fn add(&mut self, o: &Census) {
        self.bodies += o.bodies;
        self.raw_ptr_locals += o.raw_ptr_locals;
        self.ref_locals += o.ref_locals;
        self.derefs += o.derefs;
        self.raw_derefs += o.raw_derefs;
        self.raw_to_safe_casts += o.raw_to_safe_casts;
        self.ptr_int_casts += o.ptr_int_casts;
        self.ffi_calls += o.ffi_calls;
        self.rooted_derefs += o.rooted_derefs;
        self.lost_derefs += o.lost_derefs;
    }
}

struct CensusVisitor<'a> {
    census: &'a mut Census,
    body: &'a Body,
}

impl CensusVisitor<'_> {
    /// Classifies a local's type as raw pointer / reference for the local
    /// inventory.
    fn classify_local(&mut self, ty: Ty) {
        match ty.kind() {
            TyKind::RigidTy(RigidTy::RawPtr(..)) => self.census.raw_ptr_locals += 1,
            TyKind::RigidTy(RigidTy::Ref(..)) => self.census.ref_locals += 1,
            _ => {}
        }
    }

    /// Whether a place's base local is a raw pointer, for deref
    /// classification.
    fn base_is_raw_ptr(&self, place: &Place) -> bool {
        self.body
            .locals()
            .get(place.local)
            .is_some_and(|d| matches!(d.ty.kind(), TyKind::RigidTy(RigidTy::RawPtr(..))))
    }
}

impl MirVisitor for CensusVisitor<'_> {
    fn visit_body(&mut self, body: &Body) {
        for decl in body.locals() {
            self.classify_local(decl.ty);
        }
        self.super_body(body);
    }

    fn visit_place(&mut self, place: &Place, ptx: PlaceContext, loc: Location) {
        let raw_base = self.base_is_raw_ptr(place);
        for elem in &place.projection {
            if matches!(elem, ProjectionElem::Deref) {
                self.census.derefs += 1;
                if raw_base {
                    self.census.raw_derefs += 1;
                }
            }
        }
        self.super_place(place, ptx, loc);
    }

    fn visit_rvalue(&mut self, rvalue: &Rvalue, loc: Location) {
        match rvalue {
            // &*p / &mut *p reborrows: a reference produced from a place
            // that derefs a raw pointer is the raw->safe boundary.
            Rvalue::Ref(_, _, place) | Rvalue::AddressOf(_, place) => {
                if place
                    .projection
                    .iter()
                    .any(|e| matches!(e, ProjectionElem::Deref))
                    && self.base_is_raw_ptr(place)
                    && matches!(rvalue, Rvalue::Ref(..))
                {
                    self.census.raw_to_safe_casts += 1;
                }
            }
            Rvalue::Cast(kind, _op, target) => match kind {
                CastKind::PtrToPtr | CastKind::PointerCoercion(..) => {
                    if matches!(target.kind(), TyKind::RigidTy(RigidTy::Ref(..))) {
                        self.census.raw_to_safe_casts += 1;
                    }
                }
                CastKind::PointerExposeAddress | CastKind::PointerWithExposedProvenance => {
                    self.census.ptr_int_casts += 1;
                }
                _ => {}
            },
            _ => {}
        }
        self.super_rvalue(rvalue, loc);
    }

    fn visit_terminator(&mut self, term: &Terminator, loc: Location) {
        if let TerminatorKind::Call { func, .. } = &term.kind
            && let Ok(callee_ty) = func.ty(self.body.locals())
            && is_foreign_abi(callee_ty)
        {
            self.census.ffi_calls += 1;
        }
        self.super_terminator(term, loc);
    }
}

/// Whether a callee's function type uses a non-Rust ABI (an FFI edge). The
/// signature — including its ABI — is read straight off the `FnDef`/`FnPtr`
/// type; resolving a monomorphic `Instance` here would panic on the many
/// unresolvable generic callees real crates contain.
fn is_foreign_abi(ty: Ty) -> bool {
    match ty.kind().fn_sig() {
        Some(sig) => !matches!(sig.value.abi, Abi::Rust | Abi::RustCall),
        None => false,
    }
}

fn report(krate: &str, census: &Census) {
    let json = format!(
        "{{\"crate\":\"{}\",\"bodies\":{},\"raw_ptr_locals\":{},\"ref_locals\":{},\
\"derefs\":{},\"raw_derefs\":{},\"raw_to_safe_casts\":{},\"ptr_int_casts\":{},\
\"ffi_calls\":{},\"skipped_bodies\":{},\"rooted_derefs\":{},\"lost_derefs\":{}}}",
        krate,
        census.bodies,
        census.raw_ptr_locals,
        census.ref_locals,
        census.derefs,
        census.raw_derefs,
        census.raw_to_safe_casts,
        census.ptr_int_casts,
        census.ffi_calls,
        census.skipped_bodies,
        census.rooted_derefs,
        census.lost_derefs,
    );

    // FEC_CENSUS_DIR: one file per crate (for multi-crate builds like a
    // serde tree). FEC_CENSUS_OUT: a single file. Otherwise: stderr.
    if let Some(dir) = std::env::var_os("FEC_CENSUS_DIR") {
        let path = std::path::Path::new(&dir).join(format!("{krate}.json"));
        let _ = std::fs::create_dir_all(&dir);
        if std::fs::write(&path, &json).is_ok() {
            return;
        }
    }
    match std::env::var("FEC_CENSUS_OUT") {
        Ok(path) => {
            if let Err(e) = std::fs::write(&path, &json) {
                eprintln!("fe-c-driver: could not write census to {path}: {e}");
                eprintln!("{json}");
            }
        }
        Err(_) => eprintln!("fe-c-census {json}"),
    }
}
