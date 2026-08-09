//! The saver registry: one lazily-loaded wasm chunk per screensaver.
//!
//! # Why this file looks the way it does
//!
//! `#[component(lazy)]` is not usable here. It hardcodes the split module name
//! to `"lazy"`, so every lazy component in the app would share one chunk and
//! opening any saver would download all of them. The `lazy_loader!` macro
//! underneath it does take a module name, so each saver declares its own.
//!
//! Each chunk exports exactly one function, the saver's own `start`, which
//! **runs** rather than returning a pointer to something the host will run.
//! That direction is what makes splitting work: the splitter follows real calls
//! out of the exported function, so the hack's code and data are reachable from
//! that chunk and nowhere else. An earlier version handed back a `SaverDef`
//! containing a constructor pointer, and every hack stayed in the main module
//! because the only thing that ever *called* it was main-resident code.
//!
//! The shared runtime (framebuffer, Xlib façade, `Runner`, the panel) stays in
//! the main module on purpose. `wasm-split` 0.7.9 never emits a shared chunk
//! (`build_split_chunks` computes an empty set), so anything reachable from two
//! split modules but not from `main` would be copied into *both*.
//!
//! For the same reason nothing here may touch `xscreensaver::all()` or
//! `xscreensaver::find()`: those name every saver's entry point, which would
//! drag the entire collection back into the main module.

use std::future::Future;
use std::pin::Pin;

use xscreensaver::runtime::{Runner, StartArgs};

type RunnerFuture = Pin<Box<dyn Future<Output = Option<Runner>>>>;

/// One saver, as the router sees it before its code is loaded.
pub struct Entry {
    pub slug: &'static str,
    pub label: &'static str,
    /// Downloads the saver's chunk if it is not already resident, then starts
    /// it. Resolves immediately once the chunk is in memory, so restarting a
    /// saver after a settings change costs nothing extra. `None` if the chunk
    /// could not be fetched.
    pub start: fn(StartArgs) -> RunnerFuture,
}

/// Declare a saver: the entry point its chunk exports, and the loader that
/// awaits it.
///
/// Without the `split` feature this compiles to a direct call, so `dx serve`
/// works normally and there is no wasm machinery in the build at all.
macro_rules! saver {
    ($slug:literal, $body:ident, $load:ident, $path:path) => {
        fn $body(args: StartArgs) -> Runner {
            $path(args)
        }

        #[cfg(feature = "split")]
        fn $load(args: StartArgs) -> RunnerFuture {
            Box::pin(async {
                // The module name is the slug, so the emitted chunk is
                // recognisable in the network tab and in the bundle.
                static MODULE: wasm_split::LazyLoader<StartArgs, Runner> =
            wasm_split::lazy_loader!(extern $slug fn $body(props: StartArgs) -> Runner);
                if MODULE.load().await {
                    MODULE.call(args).ok()
                } else {
                    None
                }
            })
        }

        #[cfg(not(feature = "split"))]
        fn $load(args: StartArgs) -> RunnerFuture {
            Box::pin(async { Some($body(args)) })
        }
    };
}

saver!(
    "coral",
    coral_body,
    coral_start,
    xscreensaver::hacks2d::coral::start
);
saver!(
    "decayscreen",
    decayscreen_body,
    decayscreen_start,
    xscreensaver::hacks2d::decayscreen::start
);
saver!(
    "greynetic",
    greynetic_body,
    greynetic_start,
    xscreensaver::hacks2d::greynetic::start
);
saver!(
    "moire",
    moire_body,
    moire_start,
    xscreensaver::hacks2d::moire::start
);
saver!(
    "munch",
    munch_body,
    munch_start,
    xscreensaver::hacks2d::munch::start
);
saver!(
    "rorschach",
    rorschach_body,
    rorschach_start,
    xscreensaver::hacks2d::rorschach::start
);

saver!(
    "sierpinski",
    sierpinski_body,
    sierpinski_start,
    xscreensaver::hacks2d::sierpinski::start
);
saver!(
    "vines",
    vines_body,
    vines_start,
    xscreensaver::hacks2d::vines::start
);

/// Every saver, by slug. Only the slug, the label and a function pointer live
/// here; the code behind each one arrives on demand.
pub static SAVERS: &[Entry] = &[
    Entry {
        slug: "coral",
        label: "Coral",
        start: coral_start,
    },
    Entry {
        slug: "decayscreen",
        label: "Decay Screen",
        start: decayscreen_start,
    },
    Entry {
        slug: "greynetic",
        label: "Greynetic",
        start: greynetic_start,
    },
    Entry {
        slug: "moire",
        label: "Moiré",
        start: moire_start,
    },
    Entry {
        slug: "munch",
        label: "Munch",
        start: munch_start,
    },
    Entry {
        slug: "rorschach",
        label: "Rorschach",
        start: rorschach_start,
    },
    Entry {
        slug: "sierpinski",
        label: "Sierpinski",
        start: sierpinski_start,
    },
    Entry {
        slug: "vines",
        label: "Vines",
        start: vines_start,
    },
];

/// Look a saver up by its URL slug.
pub fn find(slug: &str) -> Option<&'static Entry> {
    SAVERS.iter().find(|e| e.slug == slug)
}

/// Pick one at random. Used by `/screensaver`, which then redirects to it.
pub fn random() -> &'static Entry {
    let i = (js_sys::Math::random() * SAVERS.len() as f64) as usize;
    &SAVERS[i.min(SAVERS.len() - 1)]
}
