//! Making the Dioxus runtime available inside raw JS callbacks.
//!
//! Signals — reading one, writing one, `peek`ing one — resolve through
//! `Runtime::current()`, which reads a thread-local stack that only has anything
//! on it while Dioxus is running our code. Event handlers written in `rsx!` are
//! fine: the runtime pushes a guard before calling them. A callback we hand to
//! the browser ourselves (`scroll`, `focusout`, `setTimeout`, a WebSocket
//! `message`) is called by the browser, with nothing of ours on the stack.
//!
//! Those callbacks nevertheless appear to work, which is the trap. While the app
//! is idle the virtual DOM is parked inside `wait_for_work`, which holds a guard
//! across its await, so a listener firing then finds a runtime and does the
//! right thing. But `wait_for_work` returns *before* taking that guard whenever
//! there are dirty scopes, so through the whole render-and-commit window the
//! stack is empty. A listener that fires in that window panics with "Must be
//! called from inside a Dioxus runtime".
//!
//! Which is a narrow window, and it took a reader clicking through to the
//! Microsoft viewer to hit it: swapping the viewer marks scopes dirty, mounting
//! the iframe moves focus and shifts the scroll position, and those events land
//! in precisely the gap the render opened.
//!
//! So callbacks the browser calls wrap their body in [`enter`]. There is one
//! runtime per page and it outlives everything, so remembering it once at start
//! ([`remember`]) is enough.

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::core::{Runtime, RuntimeGuard};

thread_local! {
    /// The page's runtime, captured by [`remember`].
    static RUNTIME: RefCell<Option<Rc<Runtime>>> = const { RefCell::new(None) };
}

/// Remember the runtime for [`enter`]. Call once, from inside it.
///
/// Quietly does nothing if called from outside a runtime, which would mean the
/// caller moved out of the app's startup path; [`enter`] then behaves as it did
/// before this module existed.
pub fn remember() {
    if let Some(runtime) = Runtime::try_current() {
        RUNTIME.with(|slot| *slot.borrow_mut() = Some(runtime));
    }
}

/// Run `f` with the Dioxus runtime current, so it may touch signals.
///
/// Wrap the body of every callback the *browser* calls that reads or writes
/// Dioxus state. Cheap, and safe to use where a runtime already happens to be
/// current — that case is checked for and left alone.
pub fn enter<R>(f: impl FnOnce() -> R) -> R {
    if Runtime::try_current().is_some() {
        return f();
    }
    // Clone the `Rc` out and drop the borrow: `f` is arbitrary code, and it can
    // reach `remember` through a component that runs while it is on the stack.
    let remembered = RUNTIME.with(|slot| slot.borrow().clone());
    match remembered {
        Some(runtime) => {
            let _guard = RuntimeGuard::new(runtime);
            f()
        }
        // Nothing to enter. Better to run and let the signal call itself panic
        // with its own message than to silently skip the callback.
        None => f(),
    }
}

#[cfg(test)]
mod tests {
    /// With nothing remembered, `enter` still runs the body and returns it.
    /// Callbacks must not be quietly skipped just because the capture missed.
    #[test]
    fn a_body_runs_with_no_runtime_to_enter() {
        assert_eq!(super::enter(|| 7), 7);
        let mut ran = false;
        super::enter(|| ran = true);
        assert!(ran);
    }
}
