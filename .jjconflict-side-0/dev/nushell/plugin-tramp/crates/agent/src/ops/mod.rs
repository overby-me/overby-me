//! RPC operation handlers for the tramp-agent.
//!
//! Each submodule implements one category of RPC methods:
//!
//! | Module    | Methods                                                     |
//! |-----------|-------------------------------------------------------------|
//! | `file`    | `file.stat`, `file.stat_batch`, `file.truename`,           |
//! |           | `file.read`, `file.read_range`, `file.write`,              |
//! |           | `file.write_range`, `file.size`, `file.copy`,              |
//! |           | `file.rename`, `file.delete`, `file.set_modes`             |
//! | `dir`     | `dir.list`, `dir.create`, `dir.remove`                      |
//! | `process` | `process.run`, `process.start`, `process.read`,            |
//! |           | `process.write`, `process.kill`                             |
//! | `pty`     | `process.start_pty`, `process.resize`                       |
//! | `system`  | `system.info`, `system.getenv`, `system.statvfs`            |
//! | `batch`   | `batch` (multiple ops in one round-trip)                    |
//! | `watch`   | `watch.add`, `watch.remove`, `watch.list`                   |

pub mod batch;
pub mod dir;
pub mod file;
pub mod process;
pub mod pty;
pub mod system;
pub mod watch;
