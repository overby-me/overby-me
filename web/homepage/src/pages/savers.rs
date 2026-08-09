//! The saver registry: one lazily-loaded wasm chunk per screensaver.
//!
//! # Why this file looks the way it does
//!
//! `#[component(lazy)]` is not usable here. It hardcodes the split module name
//! to `"lazy"`, so every lazy component in the app would share one chunk and
//! opening any saver would download all of them. The `lazy_loader!` macro
//! underneath it does take a module name, so each saver declares its own.
//!
//! The chunk hands back **data, not UI**: a [`SaverDef`] is a constructor
//! function pointer plus two static tables. Everything else, the canvas, the
//! frame loop, the software framebuffer, the whole Xlib runtime, stays in the
//! main module and is shared. That is deliberate: `wasm-split` 0.7.9 never
//! emits a shared chunk (`build_split_chunks` computes an empty set), so code
//! reachable from two split modules but not from `main` is *copied into both*.
//! Returning a bare `SaverDef` keeps each chunk down to the one hack in it.
//!
//! For the same reason nothing here may touch `xscreensaver::all()` or
//! `xscreensaver::find()`: those reference every `SaverDef`, which would drag
//! the entire collection into the main module.

use std::future::Future;
use std::pin::Pin;

use xscreensaver::SaverDef;

type DefFuture = Pin<Box<dyn Future<Output = Option<&'static SaverDef>>>>;

/// One saver, as the router and the picker see it before its code is loaded.
pub struct Entry {
    pub slug: &'static str,
    pub label: &'static str,
    /// Downloads the saver's chunk if it is not already resident, then returns
    /// its definition. Returns `None` if the chunk could not be fetched.
    pub load: fn() -> DefFuture,
}

/// Declare a saver: the body function that the splitter uses as a chunk entry
/// point, and the loader that awaits it.
///
/// Without the `split` feature this compiles to a direct call, so `dx serve`
/// works normally and the native build has no wasm machinery in it at all.
macro_rules! saver {
    ($slug:literal, $label:literal, $body:ident, $load:ident, $path:path) => {
        fn $body(_: ()) -> &'static SaverDef {
            &$path
        }

        #[cfg(feature = "split")]
        fn $load() -> DefFuture {
            Box::pin(async {
                // The module name is the slug, so the emitted chunk is
                // recognisable in the network tab and in the bundle.
                static MODULE: wasm_split::LazyLoader<(), &'static SaverDef> =
                    wasm_split::lazy_loader!(
                        extern $slug fn $body(props: ()) -> &'static SaverDef
                    );
                if MODULE.load().await {
                    MODULE.call(()).ok()
                } else {
                    None
                }
            })
        }

        #[cfg(not(feature = "split"))]
        fn $load() -> DefFuture {
            Box::pin(async { Some($body(())) })
        }
    };
}

saver!(
    "greynetic",
    "Greynetic",
    greynetic_body,
    greynetic_load,
    xscreensaver::hacks2d::greynetic::DEF
);
saver!(
    "munch",
    "Munch",
    munch_body,
    munch_load,
    xscreensaver::hacks2d::munch::DEF
);
saver!(
    "rorschach",
    "Rorschach",
    rorschach_body,
    rorschach_load,
    xscreensaver::hacks2d::rorschach::DEF
);

/// Every saver, by slug. Only the slug, the label and a function pointer live
/// here; the code behind each one arrives on demand.
pub static SAVERS: &[Entry] = &[
    Entry {
        slug: "greynetic",
        label: "Greynetic",
        load: greynetic_load,
    },
    Entry {
        slug: "munch",
        label: "Munch",
        load: munch_load,
    },
    Entry {
        slug: "rorschach",
        label: "Rorschach",
        load: rorschach_load,
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
