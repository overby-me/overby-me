//! Pull-to-refresh: drag down (touch) or over-scroll up (wheel / trackpad) while
//! already at the top of the page to reload the current view, with a spinner
//! animation. The gesture is tracked against the document's scroll position, and
//! declined outright when it begins inside a box that scrolls on its own (see
//! [`inside_own_scroller`], which is the correction to this module having been
//! written when there were none). A refresh bumps the global
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

/// Whether the window is scrolled to the very top.
///
/// Shared with the feed, which uses it to decide whether new arrivals can be
/// spliced in without moving anything the reader is looking at.
pub(crate) fn at_top() -> bool {
    web_sys::window()
        .map(|_| crate::scroll_host::scroll_top() <= 0.0)
        .unwrap_or(false)
}

/// Whether the gesture began inside an element that scrolls on its own.
///
/// This module was written when the document was the only scroller, which is what
/// let it decide everything from `window.scrollY`. It is not true any more: a
/// spreadsheet, a wide table and a page-view document are all `overflow: auto`
/// boxes. Panning inside one while the page happened to be at the top armed a
/// refresh, so dragging a sheet around reloaded the view under the reader's
/// finger.
///
/// ANY scrollable ancestor disqualifies the gesture, not only one already
/// scrolled away from its top. A pannable surface owns the drags that begin on
/// it, and a sheet still at its origin is exactly where someone's first pan is a
/// downward one.
///
/// Walks to `body`, since the document scroller is the one this module is for.
fn inside_own_scroller(target: Option<web_sys::EventTarget>) -> bool {
    let Some(win) = web_sys::window() else {
        return false;
    };
    let mut node = target.and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    while let Some(el) = node {
        let tag = el.tag_name().to_lowercase();
        if tag == "body" || tag == "html" {
            return false;
        }
        // Overflowing AND allowed to scroll. Either test alone is wrong: a box
        // that merely could scroll but has nothing to scroll steals nothing, and
        // `overflow: hidden` content that exceeds its box is not draggable.
        let overflows =
            el.scroll_height() > el.client_height() || el.scroll_width() > el.client_width();
        if overflows {
            if let Ok(Some(style)) = win.get_computed_style(&el) {
                let scrollable = |axis: &str| {
                    matches!(
                        style.get_property_value(axis).unwrap_or_default().as_str(),
                        "auto" | "scroll" | "overlay"
                    )
                };
                if scrollable("overflow-y") || scrollable("overflow-x") {
                    return true;
                }
            }
        }
        node = el.parent_element();
    }
    false
}

/// Set the pull distance only when it actually changes. Touch/wheel handlers fire
/// at 60-120Hz and often write the same value (a repeated 0.0 reset, or the
/// MAX_PULL clamp), each write re-rendering the indicator's subscribers.
fn set_pull(dist: f64) {
    #[expect(
        clippy::float_cmp,
        reason = "guards re-render on the exact same value; both sides are the same untransformed f64, so bit equality is the intended test"
    )]
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
            // The browser calls this, so the runtime has to be put back before
            // touching a signal (see `crate::runtime`).
            crate::runtime::enter(|| {
                let Ok(te) = e.dyn_into::<web_sys::TouchEvent>() else {
                    return;
                };
                if !at_top() || inside_own_scroller(te.target()) {
                    active.set(false);
                    return;
                }
                if let Some(t) = te.touches().get(0) {
                    start_y.set(t.client_y() as f64);
                    active.set(true);
                }
            });
        }) as Box<dyn FnMut(web_sys::Event)>);
        let _ = win.add_event_listener_with_callback("touchstart", cb.as_ref().unchecked_ref());
        cb.forget();
    }

    // touchmove: grow the indicator with the (damped) downward drag.
    {
        let start_y = start_y.clone();
        let active = active.clone();
        let cb = Closure::wrap(Box::new(move |e: web_sys::Event| {
            // The browser calls this, so the runtime has to be put back before
            // touching a signal (see `crate::runtime`).
            crate::runtime::enter(|| {
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
            });
        }) as Box<dyn FnMut(web_sys::Event)>);
        let _ = win.add_event_listener_with_callback("touchmove", cb.as_ref().unchecked_ref());
        cb.forget();
    }

    // touchend: refresh if armed, otherwise snap back.
    {
        let active = active.clone();
        let cb = Closure::wrap(Box::new(move |_e: web_sys::Event| {
            // The browser calls this, so the runtime has to be put back before
            // touching a signal (see `crate::runtime`).
            crate::runtime::enter(|| {
                if !active.get() {
                    return;
                }
                active.set(false);
                if *PULL_DISTANCE.peek() >= THRESHOLD {
                    trigger_refresh();
                } else {
                    set_pull(0.0);
                }
            });
        }) as Box<dyn FnMut(web_sys::Event)>);
        let _ = win.add_event_listener_with_callback("touchend", cb.as_ref().unchecked_ref());
        cb.forget();
    }

    // wheel: over-scrolling up while already at the top (trackpad / mouse).
    {
        let wheel_epoch = wheel_epoch.clone();
        let cb = Closure::wrap(Box::new(move |e: web_sys::Event| {
            // The browser calls this, so the runtime has to be put back before
            // touching a signal (see `crate::runtime`).
            crate::runtime::enter(|| {
                if *PTR_REFRESHING.peek() {
                    return;
                }
                let Ok(we) = e.dyn_into::<web_sys::WheelEvent>() else {
                    return;
                };
                let dy = we.delta_y();
                // Same exclusion as the touch path: a trackpad swipe over a
                // spreadsheet is that spreadsheet's, even at the top of the page.
                if inside_own_scroller(we.target()) {
                    return;
                }
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
            });
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

    // The indicator grows with the pull and is at full size exactly at the
    // threshold, which is what says "let go now" — the job the flipped arrow
    // used to do, done by the component itself.
    let scale = match refreshing {
        true => 1.0,
        false => 0.55 + 0.45 * (pull / THRESHOLD).min(1.0),
    };

    rsx! {
        div {
            class: "{cls}",
            style: "transform: translateX(-50%) translateY({offset}px); opacity: {opacity};",
            // The M3 Expressive loading indicator, which the spec names for
            // exactly this: "loading indicators are used in the pull-to-refresh
            // behavior". Contained, because it sits over the content it is
            // refreshing rather than in a cleared space.
            //
            // The SAME component for the whole gesture. It used to be an arrow
            // in a circle while dragging and this while refreshing, which is two
            // different components for one gesture — and the arrow was the older
            // pattern the loading indicator replaces.
            div {
                class: "spinner-contained ptr-spinner",
                style: "transform: scale({scale:.3});",
                div { class: "spinner spinner-sm" }
            }
        }
    }
}
