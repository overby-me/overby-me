//! The readouts and the controls.
//!
//! The C drew this as SDL text over the top-right of the window, one
//! `drawText` call per line at a hand-computed offset. It is DOM here, so the
//! numbers can be selected and copied and the layout is the browser's problem.

// Dioxus's #[component] macro generates a props struct whose accessors share
// names with the component function's own.
#![allow(clippy::same_name_method)]

use dioxus::prelude::*;
use randie_sim::BlockType;

/// Everything the panel shows, sampled from the simulator.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Readout {
    /// Simulated time, in milliseconds.
    pub time_ms: u32,
    /// Frames a second, as drawn.
    pub fps: u32,
    /// How many blocks the room is made of.
    pub blocks: usize,
    /// Where the cursor is, in the room.
    pub mouse: (f64, f64),

    /// Where the drone really is.
    pub pos: (f64, f64),
    /// How high it really is.
    pub height: f64,
    /// Which way it really points, in degrees.
    pub angle: f64,

    /// Where it believes it is, on its own map.
    pub believed: (u16, u16),
    /// Which way it believes it points, in degrees.
    pub believed_angle: f64,
    /// What it is doing.
    pub task: String,

    /// Forward velocity, in centimetres a second.
    pub pitch: f64,
    /// Sideways velocity, in centimetres a second.
    pub roll: f64,
    /// Vertical velocity, in centimetres a second.
    pub throttle: f64,
    /// Rotational velocity, in degrees a second.
    pub yaw: f64,

    /// The laser's readings: left, front, right.
    pub laser: (u16, u16, u16),
    /// The sonar's reading, and whether it is worth anything.
    pub sonar: (u16, bool),
    /// The infrared readings: floor, then ceiling.
    pub ir: (u8, u8),
}

/// The panel down the left of the window.
#[component]
#[allow(clippy::too_many_arguments)]
pub fn Panel(
    readout: Readout,
    running: bool,
    speed: u32,
    kind: BlockType,
    on_run: EventHandler<()>,
    on_step: EventHandler<()>,
    on_reset: EventHandler<()>,
    on_reload: EventHandler<()>,
    on_clear: EventHandler<()>,
    on_fit: EventHandler<()>,
    on_speed: EventHandler<u32>,
    on_kind: EventHandler<BlockType>,
) -> Element {
    let seconds = f64::from(readout.time_ms) / 1000.0;

    rsx! {
        aside { class: "panel",
            header {
                h1 { "Randsim" }
                p { "An indoor navigation drone, and the room it thinks it is in." }
            }

            div { class: "controls",
                button {
                    class: if running { "primary running" } else { "primary" },
                    onclick: move |_| on_run.call(()),
                    if running { "Pause" } else { "Run" }
                }
                button { onclick: move |_| on_step.call(()), "Step" }
                button { onclick: move |_| on_reset.call(()), "Reset" }
            }

            div { class: "controls",
                for option in [1_u32, 2, 5, 20] {
                    button {
                        class: if speed == option { "chip on" } else { "chip" },
                        onclick: move |_| on_speed.call(option),
                        "{option}x"
                    }
                }
            }

            Group { title: "Simulation",
                Row { label: "Time", value: format!("{seconds:.1} s") }
                Row { label: "Frames", value: format!("{} /s", readout.fps) }
                Row { label: "Position", value: format!("{:.0}, {:.0} cm", readout.pos.0, readout.pos.1) }
                Row { label: "Height", value: format!("{:.0} cm", readout.height) }
                Row { label: "Heading", value: format!("{:.0}°", readout.angle) }
                Row { label: "Cursor", value: format!("{:.0}, {:.0} cm", readout.mouse.0, readout.mouse.1) }
            }

            Group { title: "What the drone believes",
                Row { label: "Task", value: readout.task.clone() }
                Row { label: "Position", value: format!("{}, {} cm", readout.believed.0, readout.believed.1) }
                Row { label: "Heading", value: format!("{:.0}°", readout.believed_angle) }
            }

            Group { title: "Flight controller",
                Row { label: "Pitch", value: format!("{:.0} cm/s", readout.pitch) }
                Row { label: "Roll", value: format!("{:.0} cm/s", readout.roll) }
                Row { label: "Throttle", value: format!("{:.0} cm/s", readout.throttle) }
                Row { label: "Yaw", value: format!("{:.0}°/s", readout.yaw) }
            }

            Group { title: "Sensors",
                Row { label: "Laser left", value: reading(readout.laser.0) }
                Row { label: "Laser front", value: reading(readout.laser.1) }
                Row { label: "Laser right", value: reading(readout.laser.2) }
                Row {
                    label: "Sonar",
                    value: if readout.sonar.1 {
                        format!("{} cm", readout.sonar.0)
                    } else {
                        "nothing".to_string()
                    },
                }
                Row { label: "Infrared down", value: format!("{} cm", readout.ir.0) }
                Row { label: "Infrared up", value: format!("{} cm", readout.ir.1) }
            }

            Group { title: "The room",
                Row { label: "Blocks", value: format!("{}", readout.blocks) }
                div { class: "controls",
                    button {
                        class: if kind == BlockType::Wall { "chip on" } else { "chip" },
                        onclick: move |_| on_kind.call(BlockType::Wall),
                        "Wall"
                    }
                    button {
                        class: if kind == BlockType::Window { "chip on" } else { "chip" },
                        onclick: move |_| on_kind.call(BlockType::Window),
                        "Window"
                    }
                }
                div { class: "controls",
                    button { onclick: move |_| on_reload.call(()), "Default room" }
                    button { onclick: move |_| on_clear.call(()), "Empty room" }
                    button { onclick: move |_| on_fit.call(()), "Fit view" }
                }
            }

            footer {
                p {
                    "Drag to draw a wall, drag from one to rub it out. Scroll to zoom, "
                    "arrows or right-drag to pan."
                }
                p {
                    "Keys: "
                    kbd { "space" }
                    " run, "
                    kbd { "s" }
                    " step, "
                    kbd { "r" }
                    " reset, "
                    kbd { "f" }
                    " fit, "
                    kbd { "q" }
                    " wall, "
                    kbd { "w" }
                    " window."
                }
                p { class: "note",
                    "The green fan is the sonar and the three solid lines are the laser. "
                    "The map in the corner is the drone's own, built from what it has flown "
                    "past; the red mark is where it believes it is."
                }
            }
        }
    }
}

/// A laser reading, or the fact that nothing came back.
fn reading(centimetres: u16) -> String {
    if centimetres >= randie_firmware::laser::LASER_MAX_DISTANCE_CM {
        "nothing".to_string()
    } else {
        format!("{centimetres} cm")
    }
}

/// A titled block of readouts.
#[component]
fn Group(title: String, children: Element) -> Element {
    rsx! {
        section {
            h2 { "{title}" }
            {children}
        }
    }
}

/// One labelled number.
#[component]
fn Row(label: String, value: String) -> Element {
    rsx! {
        div { class: "row",
            span { class: "label", "{label}" }
            span { class: "value", "{value}" }
        }
    }
}
