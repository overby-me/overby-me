//! The target schema's entity subset. The DDL is GENERATED from the domain
//! types (`wiki_domain_types::DDL`) so the Rust types are the single source of
//! truth and cannot drift from the SQL, replacing the previously hand-authored
//! `schema.sql`. The tests prove the generated DDL runs and enforces its shape
//! on BOTH engines (real SQLite via rusqlite, and Turso via the `turso` crate).

/// The entity-subset DDL, generated from `wiki-domain-types`.
pub const ENTITY_SCHEMA: &str = wiki_domain_types::DDL;
