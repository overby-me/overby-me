// SPA type IDs and POD value enum.

#![allow(non_upper_case_globals)]

// Basic POD type IDs (from spa/utils/type.h).
pub const SPA_TYPE_None: u32 = 1;
pub const SPA_TYPE_Bool: u32 = 2;
pub const SPA_TYPE_Id: u32 = 3;
pub const SPA_TYPE_Int: u32 = 4;
pub const SPA_TYPE_Long: u32 = 5;
pub const SPA_TYPE_Float: u32 = 6;
pub const SPA_TYPE_Double: u32 = 7;
pub const SPA_TYPE_String: u32 = 8;
pub const SPA_TYPE_Bytes: u32 = 9;
pub const SPA_TYPE_Rectangle: u32 = 10;
pub const SPA_TYPE_Fraction: u32 = 11;
pub const SPA_TYPE_Bitmap: u32 = 12;
pub const SPA_TYPE_Array: u32 = 13;
pub const SPA_TYPE_Struct: u32 = 14;
pub const SPA_TYPE_Object: u32 = 15;
pub const SPA_TYPE_Sequence: u32 = 16;
pub const SPA_TYPE_Pointer: u32 = 17;
pub const SPA_TYPE_Fd: u32 = 18;
pub const SPA_TYPE_Choice: u32 = 19;
pub const SPA_TYPE_Pod: u32 = 20;

// Choice types (from spa/pod/pod.h::enum spa_choice_type).
pub const SPA_CHOICE_None: u32 = 0;
pub const SPA_CHOICE_Range: u32 = 1;
pub const SPA_CHOICE_Step: u32 = 2;
pub const SPA_CHOICE_Enum: u32 = 3;
pub const SPA_CHOICE_Flags: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rectangle {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fraction {
    pub num: u32,
    pub denom: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    None,
    Bool(bool),
    Id(u32),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
    Bytes(Vec<u8>),
    Rectangle(Rectangle),
    Fraction(Fraction),
    Bitmap(Vec<u8>),
    /// All array elements share the same `child_type` and fixed `child_size`.
    Array {
        child_type: u32,
        child_size: u32,
        elements: Vec<Value>,
    },
    Struct(Vec<Value>),
    Object {
        object_type: u32,
        id: u32,
        properties: Vec<Property>,
    },
    /// Sequence of (offset, type, payload) control entries.
    Sequence {
        unit: u32,
        controls: Vec<Control>,
    },
    /// Pointer's value is meaningless across processes; preserved for
    /// round-trip but normally zeroed when sending.
    Pointer {
        ptype: u32,
        ptr: u64,
    },
    Fd(i64),
    Choice {
        choice_type: u32,
        flags: u32,
        child_type: u32,
        child_size: u32,
        elements: Vec<Value>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub key: u32,
    pub flags: u32,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Control {
    pub offset: u32,
    pub ctrl_type: u32,
    pub value: Value,
}

/// Round `n` up to the nearest 8-byte boundary (POD alignment).
pub fn align8(n: usize) -> usize {
    (n + 7) & !7
}
