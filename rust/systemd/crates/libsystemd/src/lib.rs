#![allow(clippy::result_large_err)]
#![allow(clippy::large_enum_variant)]

//! `libsystemd` is the core library for rust-systemd, providing shared
//! functionality used by the service manager (`systemd`), control tool
//! (`systemctl`), and all other rust-systemd components.
//!
//! It contains:
//! - Unit file parsing (INI-style with systemd extensions)
//! - Dependency graph engine (topological sort, cycle detection)
//! - sd_notify protocol implementation
//! - Socket activation support
//! - Platform abstractions (cgroups, eventfd, subreaper, etc.)
//! - Service lifecycle management
//! - Configuration loading
//! - Control interface (JSON-RPC 2.0)

pub mod calendar_spec;
pub mod config;
pub mod control;
pub mod dbus_server;
pub mod dbus_wait;
pub mod entrypoints;
pub mod fd_store;
pub mod generators;
pub mod hwdb;
pub mod journal;
pub mod kmsg_log;
pub mod link_config;
pub mod lock_ext;
pub mod logging;
pub mod notification_handler;
pub mod path_watcher;
pub mod platform;
pub mod runtime_info;
pub mod services;
pub mod shutdown;
pub mod signal_handler;
pub mod socket_activation;
pub mod sockets;
pub mod specifier;
pub mod timer_scheduler;
pub mod tpm2;
pub mod unit_name;
pub mod units;
pub mod watchdog;

#[cfg(test)]
mod tests;
