// SPA type registry — short-name tables and per-object property tables.
//
// Mirrors the `spa_type_info[]` graphs from `spa/include/spa/**/type-info.h`.
// Used by `pw-dump` (and other tools) to render PODs as JSON: each Object
// POD has a type id (e.g. `SPA_TYPE_OBJECT_ParamIO`) that maps to a list of
// property descriptors, and each property has a short JSON key plus an
// optional enum table for `SPA_TYPE_Id`-typed values.

#![allow(dead_code)]
#![allow(non_upper_case_globals)]

// SPA_TYPE_OBJECT_* ids (from spa/utils/type.h::enum spa_types).
pub const SPA_TYPE_OBJECT_PropInfo: u32 = 0x40001;
pub const SPA_TYPE_OBJECT_Props: u32 = 0x40002;
pub const SPA_TYPE_OBJECT_Format: u32 = 0x40003;
pub const SPA_TYPE_OBJECT_ParamBuffers: u32 = 0x40004;
pub const SPA_TYPE_OBJECT_ParamMeta: u32 = 0x40005;
pub const SPA_TYPE_OBJECT_ParamIO: u32 = 0x40006;
pub const SPA_TYPE_OBJECT_ParamProfile: u32 = 0x40007;
pub const SPA_TYPE_OBJECT_ParamPortConfig: u32 = 0x40008;
pub const SPA_TYPE_OBJECT_ParamRoute: u32 = 0x40009;
pub const SPA_TYPE_OBJECT_Profiler: u32 = 0x4000a;
pub const SPA_TYPE_OBJECT_ParamLatency: u32 = 0x4000b;
pub const SPA_TYPE_OBJECT_ParamProcessLatency: u32 = 0x4000c;
pub const SPA_TYPE_OBJECT_ParamTag: u32 = 0x4000d;
pub const SPA_TYPE_OBJECT_PeerParam: u32 = 0x4000e;
pub const SPA_TYPE_OBJECT_ParamDict: u32 = 0x4000f;

/// `(value, short_name)` pairs forming an enum table for an `Id`-typed
/// property value. Lookups are linear (tables are small).
pub type EnumTable = &'static [(u32, &'static str)];

/// Description of one property inside a SPA object: the JSON key it
/// renders to, and (for `Id`-typed properties) the enum table to translate
/// the value into a short name.
pub struct PropInfo {
    pub name: &'static str,
    pub enum_table: Option<EnumTable>,
}

const PROP_NULL: PropInfo = PropInfo {
    name: "",
    enum_table: None,
};

pub fn lookup_enum(table: EnumTable, id: u32) -> Option<&'static str> {
    table.iter().find_map(|(v, n)| (*v == id).then_some(*n))
}

// ---------------------------------------------------------------------------
// SPA_PARAM_* → short name (e.g. SPA_PARAM_IO=7 → "IO")
// (spa/include/spa/param/param-types.h::spa_type_param[])
// ---------------------------------------------------------------------------
pub const PARAM_TABLE: EnumTable = &[
    (0, "Invalid"),
    (1, "PropInfo"),
    (2, "Props"),
    (3, "EnumFormat"),
    (4, "Format"),
    (5, "Buffers"),
    (6, "Meta"),
    (7, "IO"),
    (8, "EnumProfile"),
    (9, "Profile"),
    (10, "EnumPortConfig"),
    (11, "PortConfig"),
    (12, "EnumRoute"),
    (13, "Route"),
    (14, "Control"),
    (15, "Latency"),
    (16, "ProcessLatency"),
    (17, "Tag"),
    (18, "PeerEnumFormat"),
    (20, "Capability"),
    (21, "PeerCapability"),
];

pub fn param_short_name(id: u32) -> Option<&'static str> {
    lookup_enum(PARAM_TABLE, id)
}

// ---------------------------------------------------------------------------
// SPA_IO_* → short name (spa/node/type-info.h::spa_type_io[])
// ---------------------------------------------------------------------------
pub const IO_TABLE: EnumTable = &[
    (0, "Invalid"),
    (1, "Buffers"),
    (2, "Range"),
    (3, "Clock"),
    (4, "Latency"),
    (5, "Control"),
    (6, "Notify"),
    (7, "Position"),
    (8, "RateMatch"),
    (9, "Memory"),
    (10, "AsyncBuffers"),
];

// ---------------------------------------------------------------------------
// Per-object property tables. Tables map a property key to its JSON name
// and (for Id-typed properties) the enum table for the value.
// ---------------------------------------------------------------------------

/// `SPA_TYPE_OBJECT_ParamIO` (spa_param_io enum): id, size.
const PARAM_IO_PROPS: &[(u32, PropInfo)] = &[
    (
        1,
        PropInfo {
            name: "id",
            enum_table: Some(IO_TABLE),
        },
    ),
    (
        2,
        PropInfo {
            name: "size",
            enum_table: None,
        },
    ),
];

/// Map an object-type id to its property table, or `None` for an unknown
/// object type (caller should fall back to "id-<hex>" rendering).
pub fn object_prop_table(object_type: u32) -> Option<&'static [(u32, PropInfo)]> {
    match object_type {
        SPA_TYPE_OBJECT_ParamIO => Some(PARAM_IO_PROPS),
        _ => None,
    }
}

pub fn lookup_object_prop(object_type: u32, key: u32) -> Option<&'static PropInfo> {
    object_prop_table(object_type)
        .and_then(|t| t.iter().find_map(|(k, p)| (*k == key).then_some(p)))
}

#[allow(unused)]
pub fn null_prop() -> &'static PropInfo {
    &PROP_NULL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_names() {
        assert_eq!(param_short_name(7), Some("IO"));
        assert_eq!(param_short_name(3), Some("EnumFormat"));
        assert_eq!(param_short_name(999), None);
    }

    #[test]
    fn io_names() {
        assert_eq!(lookup_enum(IO_TABLE, 3), Some("Clock"));
        assert_eq!(lookup_enum(IO_TABLE, 7), Some("Position"));
        assert_eq!(lookup_enum(IO_TABLE, 999), None);
    }

    #[test]
    fn param_io_props() {
        let p = lookup_object_prop(SPA_TYPE_OBJECT_ParamIO, 1).unwrap();
        assert_eq!(p.name, "id");
        assert!(p.enum_table.is_some());
        let q = lookup_object_prop(SPA_TYPE_OBJECT_ParamIO, 2).unwrap();
        assert_eq!(q.name, "size");
    }
}
