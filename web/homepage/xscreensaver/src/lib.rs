//! Rust ports of the [XScreenSaver](https://www.jwz.org/xscreensaver/) hacks.
//!
//! Upstream hacks are C programs written against Xlib: each one exports
//! `init` / `draw` / `reshape` / `event` / `free`, draws with `XFillRectangle`
//! and friends, and returns the number of microseconds it would like to sleep
//! before the next frame. This crate keeps that shape so a port stays close to
//! its original, and supplies the Xlib side in software: [`runtime::Fb`] is a
//! `Vec<u32>` that the drawing primitives rasterise into, which the host blits
//! to a canvas once per frame.
//!
//! Software rasterisation rather than Canvas2D is a deliberate choice. The
//! hacks read pixels back (`XGetImage`), draw in XOR (`XSetFunction`) and copy
//! between pixmaps constantly; all of that is awkward and slow through a canvas
//! context but trivial over a pixel buffer. It also means the entire collection
//! is testable without a browser.
//!
//! Every ported hack keeps its upstream copyright header.

pub mod hacks2d;
pub mod runtime;

pub use runtime::{Dpy, SaverDef, Screenhack};

/// Every ported saver, in the order they were added.
///
/// Deliberately absent on wasm. This table names every `SaverDef`, and a
/// `SaverDef` names its hack's constructor, so anything that can reach this can
/// reach the whole collection. On the web each saver is meant to arrive in its
/// own lazily-loaded chunk, and one reachable reference to this table would
/// keep them all in the main module instead. The web host goes through
/// `pages::savers` instead, which holds only slugs and function pointers.
#[cfg(not(target_arch = "wasm32"))]
pub fn all() -> &'static [&'static SaverDef] {
    hacks2d::ALL
}

/// Look a saver up by its URL slug. Native only, for the same reason as
/// [`all`].
#[cfg(not(target_arch = "wasm32"))]
pub fn find(slug: &str) -> Option<&'static SaverDef> {
    all().iter().copied().find(|d| d.slug == slug)
}
