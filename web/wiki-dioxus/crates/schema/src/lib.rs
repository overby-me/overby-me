//! The target schema's entity subset as an executable artifact. See
//! `schema.sql` for provenance and provisional status; the tests prove the
//! dialect claim on BOTH engines (real SQLite via rusqlite, and the decided
//! Turso Database via the `turso` crate).

/// The entity-subset DDL, verbatim from `schema.sql`.
pub const ENTITY_SCHEMA: &str = include_str!("../schema.sql");
