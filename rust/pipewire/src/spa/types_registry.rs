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
// SPA_MEDIA_TYPE_* → short name (spa/param/format-types.h::spa_type_media_type[])
// ---------------------------------------------------------------------------
pub const MEDIA_TYPE_TABLE: EnumTable = &[
    (0, "unknown"),
    (1, "audio"),
    (2, "video"),
    (3, "image"),
    (4, "binary"),
    (5, "stream"),
    (6, "application"),
];

// ---------------------------------------------------------------------------
// SPA_MEDIA_SUBTYPE_* → short name (spa_type_media_subtype[])
// ---------------------------------------------------------------------------
pub const MEDIA_SUBTYPE_TABLE: EnumTable = &[
    (0, "unknown"),
    // generic
    (1, "raw"),
    (2, "dsp"),
    (3, "iec958"),
    (4, "dsd"),
    // audio
    (0x10001, "mp3"),
    (0x10002, "aac"),
    (0x10003, "vorbis"),
    (0x10004, "wma"),
    (0x10005, "ra"),
    (0x10006, "sbc"),
    (0x10007, "adpcm"),
    (0x10008, "g723"),
    (0x10009, "g726"),
    (0x1000a, "g729"),
    (0x1000b, "amr"),
    (0x1000c, "gsm"),
    (0x1000d, "alac"),
    (0x1000e, "flac"),
    (0x1000f, "ape"),
    (0x10010, "opus"),
    (0x10011, "ac3"),
    (0x10012, "eac3"),
    (0x10013, "truehd"),
    (0x10014, "dts"),
    (0x10015, "mpegh"),
    // video
    (0x20001, "h264"),
    (0x20002, "mjpg"),
    (0x20003, "dv"),
    (0x20004, "mpegts"),
    (0x20005, "h263"),
    (0x20006, "mpeg1"),
    (0x20007, "mpeg2"),
    (0x20008, "mpeg4"),
    (0x20009, "xvid"),
    (0x2000a, "vc1"),
    (0x2000b, "vp8"),
    (0x2000c, "vp9"),
    (0x2000d, "bayer"),
    // image
    (0x30001, "jpeg"),
    // stream
    (0x40001, "midi"),
    // application
    (0x50001, "control"),
];

// ---------------------------------------------------------------------------
// SPA_AUDIO_FORMAT_* → short name (spa_type_audio_format[]). Mirrors the
// "raw-types.h" table — names dropped the underscore (S16_LE → "S16LE").
// ---------------------------------------------------------------------------
pub const AUDIO_FORMAT_TABLE: EnumTable = &[
    (0, "UNKNOWN"),
    (1, "ENCODED"),
    // interleaved (0x100..)
    (0x101, "S8"),
    (0x102, "U8"),
    (0x103, "S16LE"),
    (0x104, "S16BE"),
    (0x105, "U16LE"),
    (0x106, "U16BE"),
    (0x107, "S24_32LE"),
    (0x108, "S24_32BE"),
    (0x109, "U24_32LE"),
    (0x10a, "U24_32BE"),
    (0x10b, "S32LE"),
    (0x10c, "S32BE"),
    (0x10d, "U32LE"),
    (0x10e, "U32BE"),
    (0x10f, "S24LE"),
    (0x110, "S24BE"),
    (0x111, "U24LE"),
    (0x112, "U24BE"),
    (0x113, "S20LE"),
    (0x114, "S20BE"),
    (0x115, "U20LE"),
    (0x116, "U20BE"),
    (0x117, "S18LE"),
    (0x118, "S18BE"),
    (0x119, "U18LE"),
    (0x11a, "U18BE"),
    (0x11b, "F32LE"),
    (0x11c, "F32BE"),
    (0x11d, "F64LE"),
    (0x11e, "F64BE"),
    (0x11f, "ULAW"),
    (0x120, "ALAW"),
    // planar (0x200..)
    (0x201, "U8P"),
    (0x202, "S16P"),
    (0x203, "S24_32P"),
    (0x204, "S32P"),
    (0x205, "S24P"),
    (0x206, "F32P"),
    (0x207, "F64P"),
    (0x208, "S8P"),
];

// ---------------------------------------------------------------------------
// SPA_AUDIO_CHANNEL_* → short name (spa_type_audio_channel[]).
// ---------------------------------------------------------------------------
pub const AUDIO_CHANNEL_TABLE: EnumTable = &[
    (0, "UNK"),
    (1, "NA"),
    (2, "MONO"),
    (3, "FL"),
    (4, "FR"),
    (5, "FC"),
    (6, "LFE"),
    (7, "SL"),
    (8, "SR"),
    (9, "FLC"),
    (10, "FRC"),
    (11, "RC"),
    (12, "RL"),
    (13, "RR"),
    (14, "TC"),
    (15, "TFL"),
    (16, "TFC"),
    (17, "TFR"),
    (18, "TRL"),
    (19, "TRC"),
    (20, "TRR"),
    (21, "RLC"),
    (22, "RRC"),
    (23, "FLW"),
    (24, "FRW"),
    (25, "LFE2"),
    (26, "FLH"),
    (27, "FCH"),
    (28, "FRH"),
    (29, "TFLC"),
    (30, "TFRC"),
    (31, "TSL"),
    (32, "TSR"),
    (33, "LLFE"),
    (34, "RLFE"),
    (35, "BC"),
    (36, "BLC"),
    (37, "BRC"),
    // Aux channels (0x1000..)
    (0x1000, "AUX0"),
    (0x1001, "AUX1"),
    (0x1002, "AUX2"),
    (0x1003, "AUX3"),
    (0x1004, "AUX4"),
    (0x1005, "AUX5"),
    (0x1006, "AUX6"),
    (0x1007, "AUX7"),
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

/// `SPA_TYPE_OBJECT_Format` (spa_format enum): subset covering audio/raw
/// — the fields the rich-daemon Port EnumFormat exercises. For Id-typed
/// values we plug in the matching enum table so values like
/// `SPA_AUDIO_FORMAT_F32P` render as "F32P".
const FORMAT_PROPS: &[(u32, PropInfo)] = &[
    (
        1,
        PropInfo {
            name: "mediaType",
            enum_table: Some(MEDIA_TYPE_TABLE),
        },
    ),
    (
        2,
        PropInfo {
            name: "mediaSubtype",
            enum_table: Some(MEDIA_SUBTYPE_TABLE),
        },
    ),
    (
        0x10001,
        PropInfo {
            name: "format",
            enum_table: Some(AUDIO_FORMAT_TABLE),
        },
    ),
    (
        0x10002,
        PropInfo {
            name: "flags",
            enum_table: None,
        },
    ),
    (
        0x10003,
        PropInfo {
            name: "rate",
            enum_table: None,
        },
    ),
    (
        0x10004,
        PropInfo {
            name: "channels",
            enum_table: None,
        },
    ),
    (
        0x10005,
        PropInfo {
            name: "position",
            enum_table: Some(AUDIO_CHANNEL_TABLE),
        },
    ),
];

/// Map an object-type id to its property table, or `None` for an unknown
/// object type (caller should fall back to "id-<hex>" rendering).
pub fn object_prop_table(object_type: u32) -> Option<&'static [(u32, PropInfo)]> {
    match object_type {
        SPA_TYPE_OBJECT_ParamIO => Some(PARAM_IO_PROPS),
        SPA_TYPE_OBJECT_Format => Some(FORMAT_PROPS),
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
