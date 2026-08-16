//! Initializer list size computation for global arrays.
//!
//! This module computes array dimensions and struct element counts from
//! initializer lists, handling:
//! - Brace-wrapped string literals for char/wchar_t/char16_t arrays
//! - Index designators (`[N] = val`) for sparse initialization
//! - Flat initialization of struct arrays (field-by-field without braces)
//! - String literal special cases that consume entire char array fields

use super::lower::Lowerer;
use crate::common::types::{CType, IrType, StructLayout};
use crate::frontend::parser::ast::{Designator, Expr, Initializer, InitializerItem};

impl Lowerer {
    /// Compute the effective array size from an initializer list with potential designators.
    /// Returns the minimum array size needed to hold all designated (and positional) elements.
    /// For char arrays, handles brace-wrapped string literals: char c[] = {"hello"} -> size = 6
    pub(super) fn compute_init_list_array_size_for_char_array(
        &self,
        items: &[InitializerItem],
        base_ty: IrType,
    ) -> usize {
        // Special case: char c[] = {"hello"} - single brace-wrapped string literal
        if (base_ty == IrType::I8 || base_ty == IrType::U8)
            && items.len() == 1
            && items[0].designators.is_empty()
        {
            if let Initializer::Expr(Expr::StringLiteral(s, _)) = &items[0].init {
                return s.chars().count() + 1; // +1 for null terminator
            }
            if let Initializer::Expr(
                Expr::WideStringLiteral(s, _) | Expr::Char16StringLiteral(s, _),
            ) = &items[0].init
            {
                return s.chars().count() + 1; // wide/char16 string to char array
            }
        }
        // Special case: wchar_t w[] = {L"hello"} - single brace-wrapped wide string literal
        if (base_ty == IrType::I32 || base_ty == IrType::U32)
            && items.len() == 1
            && items[0].designators.is_empty()
        {
            if let Initializer::Expr(Expr::WideStringLiteral(s, _)) = &items[0].init {
                return s.chars().count() + 1; // +1 for null terminator (count in wchar_t elements)
            }
            if let Initializer::Expr(Expr::StringLiteral(s, _)) = &items[0].init {
                return s.len() + 1; // narrow string to wchar_t array (each byte is an element)
            }
        }
        // Special case: char16_t c[] = {u"hello"} - single brace-wrapped char16_t string literal
        if (base_ty == IrType::I16 || base_ty == IrType::U16)
            && items.len() == 1
            && items[0].designators.is_empty()
        {
            if let Initializer::Expr(Expr::Char16StringLiteral(s, _)) = &items[0].init {
                return s.chars().count() + 1; // +1 for null terminator (count in char16_t elements)
            }
            if let Initializer::Expr(Expr::WideStringLiteral(s, _) | Expr::StringLiteral(s, _)) =
                &items[0].init
            {
                return s.chars().count() + 1; // string to char16_t array
            }
        }
        self.compute_init_list_array_size(items)
    }

    /// Compute the effective array size from an initializer list with potential designators.
    /// Returns the minimum array size needed to hold all designated (and positional) elements.
    pub(super) fn compute_init_list_array_size(&self, items: &[InitializerItem]) -> usize {
        let mut max_idx = 0usize;
        let mut current_idx = 0usize;
        for item in items {
            if let Some(Designator::Index(ref idx_expr)) = item.designators.first() {
                if let Some(idx) = self.eval_const_expr_for_designator(idx_expr) {
                    current_idx = idx;
                }
            }
            if current_idx >= max_idx {
                max_idx = current_idx + 1;
            }
            // Only advance to next element if this item does NOT have a field
            // designator (e.g., [0].field = val should NOT advance the index,
            // since multiple items may target different fields of the same element)
            let has_field_designator = item
                .designators
                .iter()
                .any(|d| matches!(d, Designator::Field(_)));
            if !has_field_designator {
                current_idx += 1;
            }
        }
        // For non-designated cases, each item is one element so use items.len().
        // For designated cases, max_idx already accounts for the correct count.
        let has_any_designator = items.iter().any(|item| !item.designators.is_empty());
        if has_any_designator {
            max_idx
        } else {
            max_idx.max(items.len())
        }
    }

    /// Compute the number of scalar initializer items needed to flat-initialize a CType.
    /// For scalar types: 1. For arrays: element_count * scalars_per_element.
    /// For structs/unions: sum of scalar counts of all fields (union uses max field count).
    fn flat_scalar_count(&self, ty: &CType) -> usize {
        match ty {
            CType::Array(elem_ty, Some(size)) => *size * self.flat_scalar_count(elem_ty),
            CType::Array(_, None) => {
                // Unsized array treated as 0 for counting purposes
                0
            }
            CType::Struct(key) | CType::Union(key) => {
                if let Some(layout) = self.types.borrow_struct_layouts().get(&**key) {
                    self.flat_scalar_count_for_layout(layout)
                } else {
                    1
                }
            }
            // All scalar types (int, float, pointer, etc.) consume 1 initializer item
            _ => 1,
        }
    }

    /// Compute the flat scalar count for a struct/union layout.
    /// For structs: sum of flat scalar counts of all fields.
    /// For unions: the maximum flat scalar count among fields (since only one is initialized).
    fn flat_scalar_count_for_layout(&self, layout: &StructLayout) -> usize {
        if layout.fields.is_empty() {
            return 0;
        }
        if layout.is_union {
            // Union: only one field is initialized, use max for sizing purposes
            layout
                .fields
                .iter()
                .map(|f| self.flat_scalar_count(&f.ty))
                .max()
                .unwrap_or(1)
        } else {
            // Struct: all fields are initialized sequentially
            layout
                .fields
                .iter()
                .map(|f| self.flat_scalar_count(&f.ty))
                .sum()
        }
    }

    /// Compute the number of struct elements in a flat initializer list for an unsized
    /// array of structs. Handles both braced (each item is one struct) and flat (items
    /// fill struct fields sequentially) initialization styles, as well as [idx] designators.
    /// E.g., struct {int a; char b;} x[] = {1, 'c', 2, 'd'} -> 2 elements
    ///       struct {int a; char b;} x[] = {{1,'c'}, {2,'d'}} -> 2 elements
    ///       struct {int a; char b;} x[] = {[2] = {1,'c'}} -> 3 elements (indices 0,1,2)
    pub(super) fn compute_struct_array_init_count(
        &self,
        items: &[InitializerItem],
        layout: &StructLayout,
    ) -> usize {
        // Use flat scalar count: accounts for array fields consuming multiple items
        let flat_count = self.flat_scalar_count_for_layout(layout);
        if flat_count == 0 {
            return items.len();
        }

        // Build per-field flat scalar counts for mapping fields_consumed to field index
        let field_scalar_counts: Vec<usize> = layout
            .fields
            .iter()
            .map(|f| self.flat_scalar_count(&f.ty))
            .collect();

        let mut max_idx = 0usize;
        let mut current_idx = 0usize;
        let mut fields_consumed = 0usize;

        for item in items {
            // Check for [idx] designator (array index)
            if let Some(Designator::Index(ref idx_expr)) = item.designators.first() {
                if let Some(idx) = self.eval_const_expr_for_designator(idx_expr) {
                    current_idx = idx;
                    fields_consumed = 0; // Reset field counter for new struct element
                }
            }

            // Check if this item starts a new struct element (braced or designator)
            let is_braced = matches!(item.init, Initializer::List(_));
            let has_field_designator = item
                .designators
                .iter()
                .any(|d| matches!(d, Designator::Field(_)));

            if is_braced || has_field_designator {
                // Each braced item or field-designated item is one struct element
                // (or part of the current struct element if field-designated)
                if has_field_designator {
                    // .field = val stays in current struct element
                } else {
                    // Braced list is one complete struct element
                    if fields_consumed > 0 {
                        // We were in the middle of a flat init - advance to next
                        current_idx += 1;
                        fields_consumed = 0;
                    }
                }
                if current_idx >= max_idx {
                    max_idx = current_idx + 1;
                }
                if !has_field_designator {
                    current_idx += 1;
                }
            } else {
                // Check if this expression produces a whole struct/union value
                // (e.g., a global struct variable used as an initializer element).
                // If so, it occupies one complete array element, not one scalar field slot.
                let is_struct_value = if let Initializer::Expr(e) = &item.init {
                    self.struct_value_size(e).is_some()
                } else {
                    false
                };

                if is_struct_value {
                    // Struct-valued expression: finish any partial element, then count
                    // this as one complete struct element
                    if fields_consumed > 0 {
                        // We were in the middle of a flat init - finish it
                        if current_idx >= max_idx {
                            max_idx = current_idx + 1;
                        }
                        current_idx += 1;
                        fields_consumed = 0;
                    }
                    if current_idx >= max_idx {
                        max_idx = current_idx + 1;
                    }
                    current_idx += 1;
                } else {
                    // Flat init: determine how many scalar slots this item consumes.
                    // A string literal initializing a char/wchar_t array field fills the
                    // entire array, not just one scalar slot.
                    let slots = if self.flat_init_item_is_string_for_char_array(
                        &item.init,
                        fields_consumed,
                        &field_scalar_counts,
                        layout,
                    ) {
                        // String literal fills the entire current char array field
                        self.remaining_scalars_in_current_field(
                            fields_consumed,
                            &field_scalar_counts,
                        )
                    } else {
                        1
                    };
                    fields_consumed += slots;
                    if fields_consumed >= flat_count {
                        // Completed one struct element
                        if current_idx >= max_idx {
                            max_idx = current_idx + 1;
                        }
                        current_idx += 1;
                        fields_consumed = 0;
                    }
                }
            }
        }

        // If there are remaining fields consumed, count the partial struct element
        if fields_consumed > 0 && current_idx >= max_idx {
            max_idx = current_idx + 1;
        }

        max_idx
    }

    /// Check if a flat initializer item is a string literal that initializes a char array field.
    fn flat_init_item_is_string_for_char_array(
        &self,
        init: &Initializer,
        fields_consumed: usize,
        field_scalar_counts: &[usize],
        layout: &StructLayout,
    ) -> bool {
        let is_string = matches!(
            init,
            Initializer::Expr(
                Expr::StringLiteral(_, _)
                    | Expr::WideStringLiteral(_, _)
                    | Expr::Char16StringLiteral(_, _)
            )
        );
        if !is_string {
            return false;
        }
        // Find which field index corresponds to fields_consumed
        let mut remaining = fields_consumed;
        for (fi, &count) in field_scalar_counts.iter().enumerate() {
            if remaining < count {
                // This is the field at index fi, check if it's a char/wchar_t array
                return Self::is_char_array_type(&layout.fields[fi].ty);
            }
            remaining -= count;
        }
        false
    }

    /// Compute how many scalar slots remain in the current field given a flat position.
    fn remaining_scalars_in_current_field(
        &self,
        fields_consumed: usize,
        field_scalar_counts: &[usize],
    ) -> usize {
        let mut remaining = fields_consumed;
        for &count in field_scalar_counts {
            if remaining < count {
                return count - remaining;
            }
            remaining -= count;
        }
        1
    }

    /// Check if a CType is a char or wchar_t array (eligible for string literal initialization).
    fn is_char_array_type(ty: &CType) -> bool {
        match ty {
            CType::Array(elem, _) => {
                matches!(
                    elem.as_ref(),
                    CType::Char | CType::UChar | CType::Int | CType::UInt
                )
            }
            _ => false,
        }
    }
}
