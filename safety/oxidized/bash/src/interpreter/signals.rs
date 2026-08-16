use super::*;

/// Global pending signal flags (indexed by signal number, 0-64)
#[allow(clippy::declare_interior_mutable_const)]
static PENDING_SIGNALS: [AtomicBool; 65] = {
    const INIT: AtomicBool = AtomicBool::new(false);
    [INIT; 65]
};

/// C-level signal handler that sets the pending flag
extern "C" fn shell_signal_handler(signum: libc::c_int) {
    if (signum as usize) < PENDING_SIGNALS.len() {
        PENDING_SIGNALS[signum as usize].store(true, Ordering::SeqCst);
    }
}

/// Install a signal handler that marks the signal as pending
pub fn install_signal_handler(signum: i32) {
    unsafe {
        libc::signal(
            signum,
            shell_signal_handler as *const () as libc::sighandler_t,
        );
    }
}

/// Check and clear a pending signal, returns true if it was pending
pub fn take_pending_signal(signum: i32) -> bool {
    if (signum as usize) < PENDING_SIGNALS.len() {
        PENDING_SIGNALS[signum as usize].swap(false, Ordering::SeqCst)
    } else {
        false
    }
}
