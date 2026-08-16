//! How a type spells itself inside an intrinsic's name.
//!
//! An overloaded intrinsic carries the types it was instantiated at in its
//! own name, so `llvm.umax` at `i8` is `llvm.umax.i8` and `llvm.memcpy`
//! between two flat pointers with a 64-bit length is
//! `llvm.memcpy.p0.p0.i64`. [`mangled_type`] is the spelling of one such
//! component.
//!
//! Every rule below was read off the assembler rather than reasoned about.
//! `llvm.ssa.copy` is overloaded on a single position that accepts any first
//! class type, so `declare T @llvm.ssa.copy(T)` comes back named
//! `@llvm.ssa.copy.<mangling of T>` and one module answers one question. That
//! is worth saying because three of the answers are not what the spelling
//! suggests: `token` and `label` are both `i0`, `metadata` is `Metadata` with
//! a capital, and a packed struct mangles exactly like the unpacked one, so
//! `<{i8, i32}>` and `{i8, i32}` share a component. `void` is `isVoid`, which
//! `llvm.lifetime.start.isVoid.i64.p0` in upstream's own tests shows as well.
//!
//! A type with no spelling answers `None`, and the caller leaves the name
//! alone rather than inventing one. Two of those are measured: an identified
//! struct written by number has none, because upstream's name for it is empty
//! and the mangling it lands on collides with the one for `{}`; and a
//! function type is not a value, so nothing can be instantiated at it.

use crate::context::Context;
use crate::types::{TypeId, TypeKind};
use llvm_support::FloatSemantics;

/// The component this type contributes to an intrinsic name, or `None` when
/// it has no spelling.
pub fn mangled_type(ctx: &Context, ty: TypeId) -> Option<String> {
    Some(match ctx.type_kind(ty) {
        TypeKind::Void => "isVoid".to_string(),
        // Neither is an integer, and both mangle as the integer of no width.
        TypeKind::Label | TypeKind::Token => "i0".to_string(),
        TypeKind::Metadata => "Metadata".to_string(),
        TypeKind::X86Amx => "x86amx".to_string(),
        TypeKind::Integer(bits) => format!("i{bits}"),
        TypeKind::Float(semantics) => float(*semantics).to_string(),
        TypeKind::Pointer { address_space } => format!("p{address_space}"),
        TypeKind::Array { element, count } => format!("a{count}{}", mangled_type(ctx, *element)?),
        TypeKind::Vector {
            element,
            count,
            scalable,
        } => {
            let prefix = if *scalable { "nxv" } else { "v" };
            format!("{prefix}{count}{}", mangled_type(ctx, *element)?)
        }
        // Packed or not: upstream spells both the same way, so `<{i8, i32}>`
        // and `{i8, i32}` are one component.
        TypeKind::Struct { fields, .. } => {
            let mut text = "sl_".to_string();
            for field in fields {
                text.push_str(&mangled_type(ctx, *field)?);
            }
            text.push('s');
            text
        }
        TypeKind::NamedStruct(id) => {
            let def = ctx.struct_def(*id);
            if def.numbered {
                return None;
            }
            format!("s_{}s", def.name)
        }
        TypeKind::Target { name, types, ints } => {
            let mut text = format!("t{name}");
            for parameter in types {
                text.push('_');
                text.push_str(&mangled_type(ctx, *parameter)?);
            }
            for parameter in ints {
                text.push('_');
                text.push_str(&parameter.to_string());
            }
            text.push('t');
            text
        }
        TypeKind::Function { .. } => return None,
    })
}

/// The name of a float format, which is its width rather than its IR
/// spelling: `half` is `f16` and `bfloat` is `bf16`.
fn float(semantics: FloatSemantics) -> &'static str {
    match semantics {
        FloatSemantics::Half => "f16",
        FloatSemantics::BFloat => "bf16",
        FloatSemantics::Single => "f32",
        FloatSemantics::Double => "f64",
        FloatSemantics::Quad => "f128",
        FloatSemantics::X87DoubleExtended => "f80",
        FloatSemantics::PpcDoubleDouble => "ppcf128",
    }
}

#[cfg(test)]
mod tests {
    use super::mangled_type;
    use crate::context::Context;
    use crate::types::TypeKind;
    use llvm_support::FloatSemantics;

    /// Every pair here is one module upstream answered: `llvm.ssa.copy` at
    /// the type on the left came back named with the component on the right.
    #[test]
    fn a_type_spells_itself_the_way_the_assembler_spelled_it() {
        let mut ctx = Context::new();
        let i1 = ctx.int_type(1);
        let i32_ = ctx.int_type(32);
        let i8_ = ctx.int_type(8);
        let p0 = ctx.pointer_type(0);
        let p3 = ctx.pointer_type(3);
        let mut cases: Vec<(crate::types::TypeId, &str)> = vec![(i1, "i1"), (p0, "p0")];
        for (bits, wanted) in [(7u32, "i7"), (1024, "i1024")] {
            let ty = ctx.int_type(bits);
            cases.push((ty, wanted));
        }
        for (semantics, wanted) in [
            (FloatSemantics::Half, "f16"),
            (FloatSemantics::BFloat, "bf16"),
            (FloatSemantics::Single, "f32"),
            (FloatSemantics::Double, "f64"),
            (FloatSemantics::X87DoubleExtended, "f80"),
            (FloatSemantics::Quad, "f128"),
            (FloatSemantics::PpcDoubleDouble, "ppcf128"),
        ] {
            let ty = ctx.float_type(semantics);
            cases.push((ty, wanted));
        }
        let more = [
            (ctx.pointer_type(270), "p270"),
            (ctx.vector_type(i32_, 4, false), "v4i32"),
            (ctx.vector_type(p0, 2, false), "v2p0"),
            (ctx.vector_type(p3, 4, false), "v4p3"),
            (ctx.vector_type(i1, 1, false), "v1i1"),
            (ctx.array_type(i32_, 4), "a4i32"),
            (ctx.array_type(i8_, 0), "a0i8"),
            (ctx.void_type(), "isVoid"),
            (ctx.token_type(), "i0"),
            (ctx.label_type(), "i0"),
            (ctx.metadata_type(), "Metadata"),
            (ctx.intern_type(TypeKind::X86Amx), "x86amx"),
        ];
        cases.extend(more);
        for (ty, wanted) in cases {
            assert_eq!(mangled_type(&ctx, ty).as_deref(), Some(wanted));
        }
    }

    /// The composite spellings, which nest: `[2 x [3 x float]]` is `a2a3f32`
    /// and `{i32, {float, i8}}` is `sl_i32sl_f32i8ss`.
    #[test]
    fn a_composite_type_spells_what_it_contains() {
        let mut ctx = Context::new();
        let i8_ = ctx.int_type(8);
        let i32_ = ctx.int_type(32);
        let i64_ = ctx.int_type(64);
        let f32_ = ctx.float_type(FloatSemantics::Single);
        let inner = ctx.array_type(f32_, 3);
        let outer = ctx.array_type(inner, 2);
        assert_eq!(mangled_type(&ctx, outer).as_deref(), Some("a2a3f32"));
        let nested = ctx.struct_type(vec![f32_, i8_], false);
        let wrapping = ctx.struct_type(vec![i32_, nested], false);
        assert_eq!(
            mangled_type(&ctx, wrapping).as_deref(),
            Some("sl_i32sl_f32i8ss")
        );
        let empty = ctx.struct_type(vec![], false);
        assert_eq!(mangled_type(&ctx, empty).as_deref(), Some("sl_s"));
        // Packed and unpacked share a spelling.
        let packed = ctx.struct_type(vec![i8_, i32_], true);
        let plain = ctx.struct_type(vec![i8_, i32_], false);
        assert_eq!(mangled_type(&ctx, packed).as_deref(), Some("sl_i8i32s"));
        assert_eq!(mangled_type(&ctx, plain).as_deref(), Some("sl_i8i32s"));
        let scalable = ctx.vector_type(i64_, 2, true);
        assert_eq!(mangled_type(&ctx, scalable).as_deref(), Some("nxv2i64"));
    }

    /// A target extension type carries its own name and then its parameters,
    /// types before integers, each behind an underscore.
    #[test]
    fn a_target_type_spells_its_parameters() {
        let mut ctx = Context::new();
        let i8_ = ctx.int_type(8);
        let i16_ = ctx.int_type(16);
        let target = |ctx: &mut Context, name: &str, types: Vec<crate::types::TypeId>, ints| {
            ctx.intern_type(TypeKind::Target {
                name: name.to_string(),
                types,
                ints,
            })
        };
        let bare = target(&mut ctx, "aarch64.svcount", vec![], vec![]);
        assert_eq!(
            mangled_type(&ctx, bare).as_deref(),
            Some("taarch64.svcountt")
        );
        let element = ctx.vector_type(i8_, 8, true);
        let tuple = target(&mut ctx, "riscv.vector.tuple", vec![element], vec![2]);
        assert_eq!(
            mangled_type(&ctx, tuple).as_deref(),
            Some("triscv.vector.tuple_nxv8i8_2t")
        );
        let mixed = target(&mut ctx, "t", vec![i8_, i16_], vec![3, 4, 5]);
        assert_eq!(
            mangled_type(&ctx, mixed).as_deref(),
            Some("tt_i8_i16_3_4_5t")
        );
        let empty = target(&mut ctx, "t", vec![], vec![]);
        assert_eq!(mangled_type(&ctx, empty).as_deref(), Some("ttt"));
    }

    /// An identified struct spells its name, and one written by number has
    /// none to spell.
    #[test]
    fn an_identified_struct_spells_its_name_or_nothing() {
        let mut ctx = Context::new();
        let i32_ = ctx.int_type(32);
        let f32_ = ctx.float_type(FloatSemantics::Single);
        let id = ctx.named_struct("foo");
        ctx.set_struct_body(id, vec![i32_, f32_], false);
        let ty = ctx.named_struct_type(id);
        assert_eq!(mangled_type(&ctx, ty).as_deref(), Some("s_foos"));

        let numbered = ctx.named_struct("0");
        ctx.set_struct_body(numbered, vec![i32_], false);
        ctx.set_struct_numbered(numbered);
        let ty = ctx.named_struct_type(numbered);
        assert_eq!(mangled_type(&ctx, ty), None);
    }

    /// A function type is not a value, so nothing instantiates at it.
    #[test]
    fn a_function_type_has_no_spelling() {
        let mut ctx = Context::new();
        let i32_ = ctx.int_type(32);
        let ty = ctx.function_type(i32_, vec![i32_], false);
        assert_eq!(mangled_type(&ctx, ty), None);
        assert!(matches!(ctx.type_kind(ty), TypeKind::Function { .. }));
    }
}
