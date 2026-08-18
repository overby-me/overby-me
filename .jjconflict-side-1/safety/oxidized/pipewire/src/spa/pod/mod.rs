// SPA POD: the Plain Old Data binary format used by every PipeWire IPC
// message and parameter. Aims for byte-exact compatibility with the C
// `spa_pod_*` API in `spa/include/spa/pod/`.

// Public API is exercised by unit tests + the pod-encode helper; later
// phases (protocol-native client) will use it directly.
#![allow(dead_code)]
//
// Wire format: every POD is `[size: u32][type: u32][payload: size bytes][padding to 8 bytes]`.
//
// The payload layout is type-specific (see SPA_TYPE_* in
// `spa/include/spa/utils/type.h`):
//   None      → empty
//   Bool      → i32 (0 or non-zero), 4 bytes pad to 8
//   Id        → u32, 4 bytes pad
//   Int       → i32, 4 bytes pad
//   Long      → i64
//   Float     → f32 le, 4 bytes pad
//   Double    → f64 le
//   String    → null-terminated bytes (size includes the NUL)
//   Bytes     → raw bytes
//   Rectangle → (u32 width, u32 height)
//   Fraction  → (u32 num, u32 denom)
//   Bitmap    → raw bytes
//   Array     → (struct spa_pod child header)(elements)
//   Struct    → 0+ inner PODs concatenated
//   Object    → (u32 type, u32 id) then properties
//   Sequence  → (u32 unit, u32 pad) then control entries
//   Pointer   → (u32 type, u32 pad, u64 ptr) — the pointer value isn't
//               sent over the wire
//   Fd        → i64
//   Choice    → (u32 choice_type, u32 flags, struct spa_pod child)(elements)

pub mod builder;
pub mod parser;
pub mod types;

// Re-exports kept commented for now — fully resolved by their absolute
// paths in the small set of internal callers; consumers from outside the
// crate root will go through these once we add a library target.
// pub use builder::Builder;
// pub use parser::Parser;
// pub use types::*;

#[cfg(test)]
mod tests;
