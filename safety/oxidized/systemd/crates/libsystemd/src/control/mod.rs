//! This module provides the control access similar to systemctl from systemd. It uses the jsonrpc 2.0 spec.

#[allow(clippy::module_inception)]
mod control;
pub mod jsonrpc2;
pub mod unit_properties;
pub mod varlink;

pub use control::*;
