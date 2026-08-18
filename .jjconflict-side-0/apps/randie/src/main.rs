//! Randsim in the browser.
//!
//! The page is one canvas and one panel. The canvas is drawn by
//! [`view::View`] on every animation frame and knows nothing about Dioxus; the
//! panel is Dioxus and samples the simulation a few times a second. Input goes
//! the other way: the canvas's pointer events and a window-level key listener
//! reach into the same `View`.

mod camera;
mod panel;
mod render;
mod view;

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::*;
use dioxus::web::WebEventExt;
use randie_sim::BlockType;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::panel::{Panel, Readout};
use crate::view::{View, start_animation_loop};

const MAIN_CSS: Asset = asset!("/assets/main.css");

/// How often the panel is refreshed, in milliseconds. The simulation runs at
/// the animation frame rate; re-rendering the DOM that often would be sixty
/// diffs a second to move a few digits.
const READOUT_MS: u32 = 100;

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        document::Stylesheet { href: MAIN_CSS }
        Room {}
    }
}

#[component]
fn Room() -> Element {
    let mut state: Signal<Option<Rc<RefCell<View>>>> = use_signal(|| None);
    let mut readout = use_signal(Readout::default);
    let mut running = use_signal(|| true);
    let mut speed = use_signal(|| 1_u32);
    let mut kind = use_signal(|| BlockType::Wall);

    // Sample the simulation for the panel. Everything the panel shows is a
    // number the canvas already has; this is the only place the two meet.
    use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(READOUT_MS).await;

            let view = state.read().clone();
            if let Some(view) = view {
                readout.set(view.borrow().readout());
            }
        }
    });

    let onmounted = move |event: MountedEvent| {
        spawn(async move {
            let Some(element) = event.data().try_as_web_event() else {
                return;
            };
            let Ok(canvas) = element.dyn_into::<HtmlCanvasElement>() else {
                return;
            };
            let Some(view) = View::new(canvas) else {
                return;
            };

            let view = Rc::new(RefCell::new(view));
            state.set(Some(Rc::clone(&view)));

            listen_for_wheel(&view);
            listen_for_keys(&view, running, speed, kind);

            start_animation_loop(view);
        });
    };

    rsx! {
        div { class: "app",
            canvas {
                id: "room",
                onmounted,
                // Drawing walls with the left button, moving the view with
                // either of the others.
                onpointerdown: move |event| {
                    let at = event.data().client_coordinates();
                    let secondary = !matches!(
                        event.data().trigger_button(),
                        Some(dioxus::html::input_data::MouseButton::Primary),
                    );
                    if let Some(view) = state.read().clone() {
                        let mut view = view.borrow_mut();
                        if secondary {
                            view.begin_pan(at.x, at.y);
                        } else {
                            let point = view.to_room(at.x, at.y);
                            view.begin_paint(point);
                        }
                    }
                },
                onpointermove: move |event| {
                    let at = event.data().client_coordinates();
                    if let Some(view) = state.read().clone() {
                        let mut view = view.borrow_mut();
                        let point = view.to_room(at.x, at.y);
                        view.track_cursor(point);
                        view.paint(point);
                        view.pan(at.x, at.y);
                    }
                },
                onpointerup: move |_| {
                    if let Some(view) = state.read().clone() {
                        let mut view = view.borrow_mut();
                        view.end_paint();
                        view.end_pan();
                    }
                },
                onpointerleave: move |_| {
                    if let Some(view) = state.read().clone() {
                        let mut view = view.borrow_mut();
                        view.end_paint();
                        view.end_pan();
                    }
                },
                // Right-dragging the view should not open a menu over it.
                oncontextmenu: move |event| event.prevent_default(),
            }

            Panel {
                readout: readout(),
                running: running(),
                speed: speed(),
                kind: kind(),
                on_run: move |()| {
                    let now = !running();
                    running.set(now);
                    if let Some(view) = state.read().clone() {
                        view.borrow_mut().running = now;
                    }
                },
                on_step: move |()| {
                    running.set(false);
                    if let Some(view) = state.read().clone() {
                        let mut view = view.borrow_mut();
                        view.running = false;
                        view.step_once();
                    }
                },
                on_reset: move |()| {
                    if let Some(view) = state.read().clone() {
                        view.borrow_mut().reset();
                    }
                },
                on_reload: move |()| {
                    if let Some(view) = state.read().clone() {
                        view.borrow_mut().reload_room();
                    }
                },
                on_clear: move |()| {
                    if let Some(view) = state.read().clone() {
                        view.borrow_mut().clear_room();
                    }
                },
                on_fit: move |()| {
                    if let Some(view) = state.read().clone() {
                        view.borrow_mut().fit();
                    }
                },
                on_speed: move |ticks: u32| {
                    speed.set(ticks);
                    if let Some(view) = state.read().clone() {
                        view.borrow_mut().speed = ticks;
                    }
                },
                on_kind: move |chosen: BlockType| {
                    kind.set(chosen);
                    if let Some(view) = state.read().clone() {
                        view.borrow_mut().kind = chosen;
                    }
                },
            }
        }
    }
}

/// Zooming about the cursor.
///
/// A hand-rolled listener rather than Dioxus's `onwheel`, because the browser
/// scrolls the page unless the listener says otherwise, and it will only
/// listen to a listener that registered itself as non-passive.
fn listen_for_wheel(view: &Rc<RefCell<View>>) {
    let Some(window) = web_sys::window() else {
        return;
    };

    let target = Rc::clone(view);
    let handler =
        Closure::<dyn FnMut(web_sys::WheelEvent)>::new(move |event: web_sys::WheelEvent| {
            event.prevent_default();

            // A multiplicative step, so zooming out undoes zooming in. The C added
            // a fixed amount per notch, which made the far end of the range crawl
            // and the near end jump.
            let factor = (-event.delta_y() * 0.0015).exp();
            target.borrow_mut().zoom_at(
                f64::from(event.client_x()),
                f64::from(event.client_y()),
                factor,
            );
        });

    let options = web_sys::AddEventListenerOptions::new();
    options.set_passive(false);

    if let Some(canvas) = window
        .document()
        .and_then(|document| document.get_element_by_id("room"))
    {
        let _ = canvas.add_event_listener_with_callback_and_add_event_listener_options(
            "wheel",
            handler.as_ref().unchecked_ref(),
            &options,
        );
    }

    handler.forget();
}

/// The keyboard shortcuts.
fn listen_for_keys(
    view: &Rc<RefCell<View>>,
    mut running: Signal<bool>,
    mut speed: Signal<u32>,
    mut kind: Signal<BlockType>,
) {
    let Some(window) = web_sys::window() else {
        return;
    };

    let target = Rc::clone(view);
    let handler =
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |event: web_sys::KeyboardEvent| {
            let mut view = target.borrow_mut();

            match event.key().as_str() {
                " " => {
                    view.running = !view.running;
                    running.set(view.running);
                }
                "s" | "S" => {
                    view.running = false;
                    running.set(false);
                    view.step_once();
                }
                "r" | "R" => view.reset(),
                "f" | "F" => view.fit(),
                "q" | "Q" => {
                    view.kind = BlockType::Wall;
                    kind.set(BlockType::Wall);
                }
                "w" | "W" => {
                    view.kind = BlockType::Window;
                    kind.set(BlockType::Window);
                }
                "1" | "2" | "5" => {
                    if let Ok(ticks) = event.key().parse::<u32>() {
                        view.speed = ticks;
                        speed.set(ticks);
                    }
                }
                "ArrowLeft" => view.nudge(-1.0, 0.0),
                "ArrowRight" => view.nudge(1.0, 0.0),
                "ArrowUp" => view.nudge(0.0, 1.0),
                "ArrowDown" => view.nudge(0.0, -1.0),
                _ => return,
            }

            // Space scrolls and the arrows scroll; neither should here.
            event.prevent_default();
        });

    let _ = window.add_event_listener_with_callback("keydown", handler.as_ref().unchecked_ref());
    handler.forget();
}
