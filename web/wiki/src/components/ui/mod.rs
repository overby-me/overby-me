//! Accessible form-control primitives that wrap `dioxus-primitives` (checkbox,
//! radio group, switch) — the three that earn a real primitive over a hand-rolled
//! control (keyboard + ARIA state managed by the primitive). The rest of the
//! shadcn-style set was unused and was removed; the app's higher-level, app-
//! agnostic components live in [`crate::components::widgets`], which together with
//! this module forms the single reusable component library.
pub mod checkbox;
pub mod radio_group;
pub mod switch;
