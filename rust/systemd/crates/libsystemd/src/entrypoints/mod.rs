pub mod dispatcher;
pub(crate) mod exec_helper;
/// Generated x86_64 syscall-number table and systemd @group syscall-set data for
/// SystemCallFilter=. Only used by the (x86_64-only) seccomp code in exec_helper.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod seccomp_filter_sets;
mod service_manager;

pub use exec_helper::{ExecHelperConfig, glob_match, run_exec_helper, write_utmp_dead_record};
pub use service_manager::{kmsg, run_service_manager, run_user_manager};
