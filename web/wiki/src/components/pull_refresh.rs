//! Pull-to-refresh: drag down (touch) or over-scroll up (wheel / trackpad) while
//! already at the top of the page to reload the current view, with a spinner
//! animation. The whole document scrolls (there is no inner scroll container),
//! so the gesture is tracked against `window.scrollY`. A refresh bumps the global
//! data version, which the path resolver and app resources depend on, so the
//! visible view refetches without a full reload.

use std::cell::Cell;
use std::rc::Rc;

use dioxus::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::session;

/// Current pull distance in pixels (0 when idle); drives the indicator position.
static PULL_DISTANCE: GlobalSignal<f64> = Signal::global(|| 0.0);
/// True while a refresh is in flight (drives the spinner animation).
static PTR_REFRESHING: GlobalSignal<bool> = Signal::global(|| false);

/// Pull distance (px) needed to arm a refresh on release.
const THRESHOLD: f64 = 70.0;
/// Cap the indicator travel so a long drag does not fling it off-screen.
const MAX_PULL: f64 = 110.0;
/// Resistance applied to the raw drag / scroll delta for a natural rubber feel.
const DAMPING: f64 = 0.5;

fn at_top() -> bool {
    web_sys::window()
        .map(|w| w.scroll_y().unwrap_or(0.0) <= 0.0)
        .unwrap_or(false)
}

/// Set the pull distance only when it actually changes. Touch/wheel handlers fire
/// at 60-120Hz and often write the same value (a repeated 0.0 reset, or the
/// MAX_PULL clamp), each write re-rendering the indicator's subscribers.
fn set_pull(dist: f64) {
    if *PULL_DISTANCE.peek() != dist {
        *PULL_DISTANCE.write() = dist;
    }
}

/// Arm the refresh: show the spinner, bump the data version so resources refetch,
/// and clear the spinner once the animation has settled.
fn trigger_refresh() {
    if *PTR_REFRESHING.peek() {
        return;
    }
    *PTR_REFRESHING.write() = true;
    set_pull(0.0);
    session::bump_data_version();
    wasm_bindgen_futures::spawn_local(async {
        gloo_timers::future::TimeoutFuture::new(900).await;
        *PTR_REFRESHING.write() = false;
    });
}

/// Snap the indicator back if a sub-threshold wheel pull is not continued.
fn schedule_wheel_decay(epoch: Rc<Cell<u32>>, mine: u32) {
    wasm_bindgen_futures::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(280).await;
        if epoch.get() == mine && !*PTR_REFRESHING.peek() {
            set_pull(0.0);
        }
    });
}

/// Attach the window touch / wheel listeners once. Leaks the closures so the
/// listeners live for the app's lifetime.
fn install_listeners() {
    let Some(win) = web_sys::window() else { return };
    // Per-gesture state shared across the touch closures.
    let start_y = Rc::new(Cell::new(0.0f64));
    let active = Rc::new(Cell::new(false));
    let wheel_epoch = Rc::new(Cell::new(0u32));

    // touchstart: arm only when the drag begins at the very top.
    {
        let start_y = start_y.clone();
        let active = active.clone();
        let cb = Closure::wrap(Box::new(move |e: web_sys::Event| {
            let Ok(te) = e.dyn_into::<web_sys::TouchEvent>() else {
                return;
            };
            if !at_top() {
                active.set(false);
                return;
            }
            if let Some(t) = te.touches().get(0) {
                start_y.set(t.client_y() as f64);
                active.set(true);
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        let _ = win.add_event_listener_with_callback("touchstart", cb.as_ref().unchecked_ref());
        cb.forget();
    }

    // touchmove: grow the indicator with the (damped) downward drag.
    {
        let start_y = start_y.clone();
        let active = active.clone();
        let cb = Closure::wrap(Box::new(move |e: web_sys::Event| {
            if !active.get() {
                return;
            }
            let Ok(te) = e.dyn_into::<web_sys::TouchEvent>() else {
                return;
            };
            let Some(t) = te.touches().get(0) else {
                return;
            };
            let dy = t.client_y() as f64 - start_y.get();
            if dy > 0.0 && at_top() {
                set_pull((dy * DAMPING).min(MAX_PULL));
            } else {
                active.set(false);
                set_pull(0.0);
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        let _ = win.add_event_listener_with_callback("touchmove", cb.as_ref().unchecked_ref());
        cb.forget();
    }

    // touchend: refresh if armed, otherwise snap back.
    {
        let active = active.clone();
        let cb = Closure::wrap(Box::new(move |_e: web_sys::Event| {
            if !active.get() {
                return;
            }
            active.set(false);
            if *PULL_DISTANCE.peek() >= THRESHOLD {
                trigger_refresh();
            } else {
                set_pull(0.0);
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        let _ = win.add_event_listener_with_callback("touchend", cb.as_ref().unchecked_ref());
        cb.forget();
    }

    // wheel: over-scrolling up while already at the top (trackpad / mouse).
    {
        let wheel_epoch = wheel_epoch.clone();
        let cb = Closure::wrap(Box::new(move |e: web_sys::Event| {
            if *PTR_REFRESHING.peek() {
                return;
            }
            let Ok(we) = e.dyn_into::<web_sys::WheelEvent>() else {
                return;
            };
            let dy = we.delta_y();
            if dy < 0.0 && at_top() {
                let cur = *PULL_DISTANCE.peek();
                let dist = (cur + (-dy) * DAMPING).min(MAX_PULL);
                set_pull(dist);
                if dist >= THRESHOLD {
                    trigger_refresh();
                } else {
                    let next = wheel_epoch.get().wrapping_add(1);
                    wheel_epoch.set(next);
                    schedule_wheel_decay(wheel_epoch.clone(), next);
                }
            } else if dy > 0.0 {
                set_pull(0.0);
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        let _ = win.add_event_listener_with_callback("wheel", cb.as_ref().unchecked_ref());
        cb.forget();
    }
}

/// A fixed spinner that descends from the top as the user pulls, and spins while
/// the current view refetches. Mount once inside the app shell.
#[component]
pub fn PullToRefresh() -> Element {
    use_hook(install_listeners);

    let pull = PULL_DISTANCE();
    let refreshing = PTR_REFRESHING();
    let armed = pull >= THRESHOLD;
    let pulling = pull > 0.0 && !refreshing;
    let visible = pull > 0.0 || refreshing;
    // Follow the finger while pulling; park at a fixed spot while refreshing.
    let offset = if refreshing { 64.0 } else { pull };
    let opacity = if refreshing {
        1.0
    } else {
        (pull / THRESHOLD).min(1.0)
    };

    let mut cls = String::from("ptr-indicator");
    if refreshing {
        cls.push_str(" refreshing");
    } else if armed {
        cls.push_str(" armed");
    }
    if !pulling {
        // Transition transform only when settling / parking, not while dragging.
        cls.push_str(" settling");
    }
    if !visible {
        cls.push_str(" hidden");
    }

    rsx! {
        div {
            class: "{cls}",
            style: "transform: translateX(-50%) translateY({offset}px); opacity: {opacity};",
            div { class: "ptr-spinner",
                if refreshing {
                    // The M3 Expressive loading indicator, which the spec names
                    // for exactly this: "loading indicators are used in the
                    // pull-to-refresh behavior". Contained, because it sits over
                    // the content it is refreshing rather than in a cleared space.
                    div { class: "spinner-contained",
                        div { class: "spinner spinner-sm" }
                    }
                } else {
                    // Still a gesture, not a wait: the arrow says which way.
                    span { class: "material-icons", "arrow_downward" }
                }
            }
        }
    }
}
