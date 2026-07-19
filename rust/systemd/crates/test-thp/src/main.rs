//! test-thp — verify the process's transparent-huge-page (THP) disable state.
//!
//! A Rust port of upstream `src/test/test-thp.c`. TEST-07-PID1.thp-disable runs
//! it under `systemd-run [-p MemoryTHP=...]` to confirm the manager applied the
//! matching `prctl(PR_SET_THP_DISABLE, ...)` in the service's exec setup.
//!
//! Usage: test-thp <no-disable|disable|madvise>
//!   Exit 0 on the expected state, 77 (skip) when the kernel does not support
//!   the requested mode, and 1 on mismatch.
use std::process::ExitCode;

const PR_GET_THP_DISABLE: libc::c_int = 42;
/// Bit 0 of the PR_GET_THP_DISABLE result: THPs disabled for the process.
const PR_THP_DISABLE: libc::c_long = 1;
/// Bit 1: THPs disabled except where explicitly madvised.
const PR_THP_DISABLE_EXCEPT_ADVISED: libc::c_long = 1 << 1;
/// systemd test-suite "skip" exit code.
const EXIT_TEST_SKIP: u8 = 77;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Invalid number of args passed to the test {}", args.len());
        return ExitCode::FAILURE;
    }
    let mode = args[1].as_str();

    let r = unsafe { libc::prctl(PR_GET_THP_DISABLE, 0, 0, 0, 0) } as libc::c_long;

    match mode {
        // THPs should not be disabled.
        "no-disable" => {
            if r != 0 {
                eprintln!("THPs disabled for the process r = {r}");
                return ExitCode::FAILURE;
            }
        }
        // THPs should be completely disabled.
        "disable" => {
            if r == 0 {
                eprintln!("Disabling THPs completely for the process not supported");
                return ExitCode::from(EXIT_TEST_SKIP);
            }
            if r != PR_THP_DISABLE {
                eprintln!("THPs not completely disabled for the process r = {r}");
                return ExitCode::FAILURE;
            }
        }
        // THPs should be enabled only on a madvise basis.
        "madvise" => {
            if r == 0 {
                eprintln!("Disabling THPs except for madvise not supported");
                return ExitCode::from(EXIT_TEST_SKIP);
            }
            if r != (PR_THP_DISABLE | PR_THP_DISABLE_EXCEPT_ADVISED) {
                eprintln!("THPs (except madvise) not completely disabled for the process r = {r}");
                return ExitCode::FAILURE;
            }
        }
        other => {
            eprintln!("Invalid mode: {other} (expected: no-disable, disable, or madvise)");
            return ExitCode::FAILURE;
        }
    }

    ExitCode::SUCCESS
}
