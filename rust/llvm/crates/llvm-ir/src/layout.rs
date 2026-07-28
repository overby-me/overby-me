//! Type layout: how big a type is and where a struct's fields sit.
//!
//! The rules come from LangRef's data layout section. Everything here answers
//! in bytes except where the name says bits, because the IR speaks bytes and
//! the layout string speaks bits, and mixing the two is how an ABI breaks.

use crate::context::Context;
use crate::types::{TypeId, TypeKind};
use llvm_support::{Align, DataLayout};

/// Where a struct's fields sit and how big the whole thing is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StructLayout {
    pub size_bytes: u64,
    pub align: Align,
    /// Byte offset of each field, in declaration order.
    pub offsets: Vec<u64>,
}

/// A type whose size cannot be a number.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LayoutError {
    /// `void`, `label`, `metadata`, `token` and function types have no size.
    Unsized(&'static str),
    /// A scalable vector's size depends on the runtime vector length.
    Scalable,
    /// An identified struct with no body yet.
    Opaque(String),
}

/// Size in bits, before any alignment padding.
///
/// This is the value `getTypeSizeInBits` returns: an `i1` is one bit, not
/// eight, and an array of them is still one bit per element, which is why the
/// allocation size below is the number almost every caller wants instead.
pub fn size_in_bits(ctx: &Context, layout: &DataLayout, ty: TypeId) -> Result<u64, LayoutError> {
    match ctx.type_kind(ty) {
        TypeKind::Integer(bits) => Ok(u64::from(*bits)),
        TypeKind::Float(semantics) => Ok(u64::from(semantics.bit_width())),
        TypeKind::Pointer { address_space } => {
            Ok(u64::from(layout.pointer_size_bits(*address_space)))
        }
        TypeKind::Array { element, count } => {
            Ok(alloc_size_bytes(ctx, layout, *element)? * 8 * count)
        }
        TypeKind::Vector {
            element,
            count,
            scalable,
        } => {
            if *scalable {
                return Err(LayoutError::Scalable);
            }
            Ok(size_in_bits(ctx, layout, *element)? * count)
        }
        TypeKind::Struct { .. } | TypeKind::NamedStruct(_) => {
            Ok(struct_layout(ctx, layout, ty)?.size_bytes * 8)
        }
        TypeKind::Void => Err(LayoutError::Unsized("void")),
        TypeKind::Label => Err(LayoutError::Unsized("label")),
        TypeKind::Metadata => Err(LayoutError::Unsized("metadata")),
        TypeKind::Token => Err(LayoutError::Unsized("token")),
        TypeKind::X86Amx => Ok(8192),
        TypeKind::Function { .. } => Err(LayoutError::Unsized("function")),
        TypeKind::Target { .. } => Err(LayoutError::Unsized("target extension")),
    }
}

/// Bytes one value occupies in memory, including tail padding: the distance
/// between consecutive elements of an array of this type.
pub fn alloc_size_bytes(
    ctx: &Context,
    layout: &DataLayout,
    ty: TypeId,
) -> Result<u64, LayoutError> {
    let bits = size_in_bits(ctx, layout, ty)?;
    let store = bits.div_ceil(8);
    Ok(abi_align(ctx, layout, ty)?.align_up(store))
}

/// The alignment the ABI requires for this type.
/// A vector's size at `vscale` of one, which is what its alignment is worked
/// out from. A fixed vector's minimum size is its size.
fn minimum_size_in_bits(
    ctx: &Context,
    layout: &DataLayout,
    ty: TypeId,
) -> Result<u64, LayoutError> {
    let TypeKind::Vector { element, count, .. } = ctx.type_kind(ty) else {
        return size_in_bits(ctx, layout, ty);
    };
    Ok(size_in_bits(ctx, layout, *element)? * count)
}

pub fn abi_align(ctx: &Context, layout: &DataLayout, ty: TypeId) -> Result<Align, LayoutError> {
    let bits = match ctx.type_kind(ty) {
        TypeKind::Integer(width) => layout.integer_align(*width).abi_bits,
        TypeKind::Float(semantics) => layout.float_align(semantics.bit_width()).abi_bits,
        TypeKind::Pointer { address_space } => layout.pointer_spec(*address_space).align.abi_bits,
        TypeKind::Array { element, .. } => return abi_align(ctx, layout, *element),
        // A scalable vector has no fixed size and still has an alignment:
        // it is the one the minimum-size vector would have, the target
        // scaling the length rather than the alignment.
        TypeKind::Vector { .. } => {
            let size = minimum_size_in_bits(ctx, layout, ty)?;
            layout
                .vector_align(u32::try_from(size).unwrap_or(u32::MAX))
                .abi_bits
        }
        TypeKind::Struct { .. } | TypeKind::NamedStruct(_) => {
            return struct_align(ctx, layout, ty);
        }
        TypeKind::X86Amx => 8192,
        TypeKind::Void | TypeKind::Label | TypeKind::Metadata | TypeKind::Token => {
            return Ok(Align::ONE);
        }
        TypeKind::Function { .. } => return Ok(Align::ONE),
        TypeKind::Target { .. } => return Err(LayoutError::Unsized("target extension")),
    };
    Ok(Align::from_bytes_rounded_up(u64::from(bits).div_ceil(8)))
}

/// The alignment the target would choose if the ABI left it free, which is
/// what an `alloca` with no explicit alignment gets.
pub fn preferred_align(
    ctx: &Context,
    layout: &DataLayout,
    ty: TypeId,
) -> Result<Align, LayoutError> {
    let bits = match ctx.type_kind(ty) {
        TypeKind::Integer(width) => layout.integer_align(*width).preferred_bits,
        TypeKind::Float(semantics) => layout.float_align(semantics.bit_width()).preferred_bits,
        TypeKind::Pointer { address_space } => {
            layout.pointer_spec(*address_space).align.preferred_bits
        }
        TypeKind::Array { element, .. } => return preferred_align(ctx, layout, *element),
        TypeKind::Vector { .. } => {
            let size = minimum_size_in_bits(ctx, layout, ty)?;
            layout
                .vector_align(u32::try_from(size).unwrap_or(u32::MAX))
                .preferred_bits
        }
        // A struct's fields decide its ABI alignment, and the layout's
        // aggregate preference can ask for more: with the default `a:0:64`,
        // an alloca of `{ i8 }` is eight-aligned even though nothing in it
        // needs to be.
        TypeKind::Struct { .. } | TypeKind::NamedStruct(_) => {
            let from_fields = struct_align(ctx, layout, ty)?;
            let preferred =
                Align::from_bits(layout.aggregate_align().preferred_bits).unwrap_or(Align::ONE);
            return Ok(from_fields.max(preferred));
        }
        TypeKind::X86Amx => 8192,
        _ => return abi_align(ctx, layout, ty),
    };
    Ok(Align::from_bytes_rounded_up(u64::from(bits).div_ceil(8)))
}

/// Field offsets, total size and alignment of a struct.
///
/// A packed struct takes byte alignment for every field and for itself; an
/// unpacked one aligns each field to its own ABI alignment, then rounds the
/// whole thing up so an array of the struct keeps every element aligned.
/// A struct's own alignment, which is the strictest its fields ask for and
/// at least whatever the layout asks of an aggregate.
///
/// This never asks a field how large it is, so a struct of scalable vectors
/// has an alignment where it has no fixed size: `{ <vscale x 1 x i32> }` is
/// four-aligned and takes however many bytes the vector length decides.
pub fn struct_align(ctx: &Context, layout: &DataLayout, ty: TypeId) -> Result<Align, LayoutError> {
    let (fields, packed) = struct_fields(ctx, ty)?;
    if packed {
        return Ok(Align::ONE);
    }
    let mut align = Align::from_bits(layout.aggregate_align().abi_bits).unwrap_or(Align::ONE);
    for field in &fields {
        align = align.max(abi_align(ctx, layout, *field)?);
    }
    Ok(align)
}

/// The fields a struct type holds, whether it was written out or named.
fn struct_fields(ctx: &Context, ty: TypeId) -> Result<(Vec<TypeId>, bool), LayoutError> {
    match ctx.type_kind(ty) {
        TypeKind::Struct { fields, packed } => Ok((fields.clone(), *packed)),
        TypeKind::NamedStruct(id) => {
            let def = ctx.struct_def(*id);
            match &def.fields {
                Some(fields) => Ok((fields.clone(), def.packed)),
                None => Err(LayoutError::Opaque(def.name.clone())),
            }
        }
        _ => Err(LayoutError::Unsized("not a struct")),
    }
}

pub fn struct_layout(
    ctx: &Context,
    layout: &DataLayout,
    ty: TypeId,
) -> Result<StructLayout, LayoutError> {
    let (fields, packed) = struct_fields(ctx, ty)?;

    let mut offset = 0u64;
    let mut align = if packed {
        Align::ONE
    } else {
        Align::from_bits(layout.aggregate_align().abi_bits).unwrap_or(Align::ONE)
    };
    let mut offsets = Vec::with_capacity(fields.len());
    for field in &fields {
        let field_align = if packed {
            Align::ONE
        } else {
            abi_align(ctx, layout, *field)?
        };
        offset = field_align.align_up(offset);
        offsets.push(offset);
        offset += alloc_size_bytes(ctx, layout, *field)?;
        align = align.max(field_align);
    }
    Ok(StructLayout {
        size_bytes: align.align_up(offset),
        align,
        offsets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use llvm_support::FloatSemantics;

    const X86_64_LINUX: &str =
        "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128";

    fn layout() -> DataLayout {
        DataLayout::parse(X86_64_LINUX).unwrap()
    }

    #[test]
    fn primitive_sizes_and_alignments() {
        let mut ctx = Context::new();
        let dl = layout();
        let i1 = ctx.int_type(1);
        let i8 = ctx.int_type(8);
        let i32 = ctx.int_type(32);
        let i64 = ctx.int_type(64);
        let i128 = ctx.int_type(128);
        let f64 = ctx.float_type(FloatSemantics::Double);
        let ptr = ctx.pointer_type(0);

        assert_eq!(size_in_bits(&ctx, &dl, i1).unwrap(), 1);
        assert_eq!(alloc_size_bytes(&ctx, &dl, i1).unwrap(), 1);
        assert_eq!(alloc_size_bytes(&ctx, &dl, i8).unwrap(), 1);
        assert_eq!(alloc_size_bytes(&ctx, &dl, i32).unwrap(), 4);
        assert_eq!(alloc_size_bytes(&ctx, &dl, i64).unwrap(), 8);
        assert_eq!(alloc_size_bytes(&ctx, &dl, f64).unwrap(), 8);
        assert_eq!(alloc_size_bytes(&ctx, &dl, ptr).unwrap(), 8);
        // The i128:128 entry is why this is 16-aligned on x86-64 and not 8.
        assert_eq!(abi_align(&ctx, &dl, i128).unwrap().bytes(), 16);
        assert_eq!(alloc_size_bytes(&ctx, &dl, i128).unwrap(), 16);
    }

    #[test]
    fn structs_pad_between_fields_and_at_the_end() {
        let mut ctx = Context::new();
        let dl = layout();
        let i8 = ctx.int_type(8);
        let i32 = ctx.int_type(32);
        let i64 = ctx.int_type(64);

        // { i8, i32 } is 8 bytes: one byte, three of padding, four more.
        let small = ctx.struct_type(vec![i8, i32], false);
        let small_layout = struct_layout(&ctx, &dl, small).unwrap();
        assert_eq!(small_layout.offsets, vec![0, 4]);
        assert_eq!(small_layout.size_bytes, 8);
        assert_eq!(small_layout.align.bytes(), 4);

        // { i8, i64 } needs eight-byte alignment, so the second field is at 8.
        let wide = ctx.struct_type(vec![i8, i64], false);
        let wide_layout = struct_layout(&ctx, &dl, wide).unwrap();
        assert_eq!(wide_layout.offsets, vec![0, 8]);
        assert_eq!(wide_layout.size_bytes, 16);

        // { i64, i8 } is the same size, with the padding at the end instead.
        let trailing = ctx.struct_type(vec![i64, i8], false);
        let trailing_layout = struct_layout(&ctx, &dl, trailing).unwrap();
        assert_eq!(trailing_layout.offsets, vec![0, 8]);
        assert_eq!(trailing_layout.size_bytes, 16);
    }

    #[test]
    fn packed_structs_have_no_padding() {
        let mut ctx = Context::new();
        let dl = layout();
        let i8 = ctx.int_type(8);
        let i64 = ctx.int_type(64);
        let packed = ctx.struct_type(vec![i8, i64], true);
        let packed_layout = struct_layout(&ctx, &dl, packed).unwrap();
        assert_eq!(packed_layout.offsets, vec![0, 1]);
        assert_eq!(packed_layout.size_bytes, 9);
        assert_eq!(packed_layout.align.bytes(), 1);
    }

    #[test]
    fn arrays_and_vectors() {
        let mut ctx = Context::new();
        let dl = layout();
        let i8 = ctx.int_type(8);
        let i32 = ctx.int_type(32);
        let i1 = ctx.int_type(1);

        let bytes = ctx.array_type(i8, 13);
        assert_eq!(alloc_size_bytes(&ctx, &dl, bytes).unwrap(), 13);
        assert_eq!(abi_align(&ctx, &dl, bytes).unwrap().bytes(), 1);

        let words = ctx.array_type(i32, 4);
        assert_eq!(alloc_size_bytes(&ctx, &dl, words).unwrap(), 16);

        // An array of i1 stores one byte per element, unlike a vector of i1.
        let bits = ctx.array_type(i1, 8);
        assert_eq!(alloc_size_bytes(&ctx, &dl, bits).unwrap(), 8);
        let bit_vector = ctx.vector_type(i1, 8, false);
        assert_eq!(size_in_bits(&ctx, &dl, bit_vector).unwrap(), 8);
        assert_eq!(alloc_size_bytes(&ctx, &dl, bit_vector).unwrap(), 1);

        let vector = ctx.vector_type(i32, 4, false);
        assert_eq!(alloc_size_bytes(&ctx, &dl, vector).unwrap(), 16);
        assert_eq!(abi_align(&ctx, &dl, vector).unwrap().bytes(), 16);
    }

    #[test]
    fn unsized_and_opaque_types_report_why() {
        let mut ctx = Context::new();
        let dl = layout();
        let void = ctx.void_type();
        assert_eq!(
            size_in_bits(&ctx, &dl, void),
            Err(LayoutError::Unsized("void"))
        );
        let i32 = ctx.int_type(32);
        let scalable = ctx.vector_type(i32, 4, true);
        assert_eq!(
            size_in_bits(&ctx, &dl, scalable),
            Err(LayoutError::Scalable)
        );
        let opaque = ctx.named_struct("Opaque");
        let opaque_ty = ctx.named_struct_type(opaque);
        assert_eq!(
            size_in_bits(&ctx, &dl, opaque_ty),
            Err(LayoutError::Opaque("Opaque".to_string()))
        );
    }
}
