//! Journal core data structures and storage.
//!
//! This module provides the journal entry model (`entry`) and the
//! on-disk storage engine (`storage`) used by both `systemd-journald`
//! (for writing) and `journalctl` (for reading).
//!
//! ## Key types
//!
//! - [`entry::JournalEntry`] — a single structured log entry with
//!   key-value fields, timestamps, and sequence number.
//! - [`entry::FieldMatch`] — filter criteria for matching entries
//!   (exact field match, priority, timestamp range).
//! - [`entry::from_export_format`] — parse entries from the systemd
//!   journal export format (`systemd-journal-export(5)`).
//! - [`entry::parse_export_entries`] — parse all entries from export
//!   format text.
//! - [`storage::JournalStorage`] — multi-file append-only storage with
//!   rotation and vacuuming.
//! - [`storage::JournalReader`] — filtered, seekable iterator over
//!   stored entries.
//! - [`storage::SeekPosition`] — where to start reading (head, tail,
//!   cursor, timestamp, sequence number).
//! - [`storage::StorageConfig`] — configuration for storage limits.
//! - [`storage::parse_cursor_realtime`] / [`storage::parse_cursor_boot_id`]
//!   — helpers for extracting fields from cursor strings.

pub mod c_journal;
pub mod entry;
pub mod storage;
