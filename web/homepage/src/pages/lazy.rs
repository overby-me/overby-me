//! The page that is worth fetching only when it is visited.
//!
//! `cardioid` is a generative toy with its own canvas, presets and options
//! panel. Nobody who came to read the homepage wants it, so it waits until
//! `/cardioid` is opened.
//!
//! The mechanism is `crate::pages::savers`' one, and the note there explains
//! why it is `wasm_split::lazy_loader!` by hand rather than
//! `#[component(lazy)]`: that macro puts every lazy component in a chunk
//! called "lazy", and it needs dioxus's own `wasm-split` feature, which makes
//! the router macro split every route and crashes the splitter.
//!
//! The chunk's exported function returns the element *tree*, not the rendered
//! component. Building it with `rsx!` means the inner component is reachable
//! only from inside the chunk, so its code goes there, and dioxus still calls
//! it in a scope of its own. Calling the inner function directly instead would
//! run its hooks inside this wrapper's scope, where the hook order changes the
//! moment the chunk finishes loading.
//!
//! # Nothing in a chunk may reach `asset!`
//!
//! `/@handle` was split the same way and had to be undone. Its graph resolves
//! 65 bundled icons through `asset!`, and manganis reads an asset's bundled
//! path by calling through a function pointer into a table that dx patches
//! after the build (`manganis-core`'s `Asset::bundled`). Move those accessors
//! into a chunk, which wasm-split 0.7.9 does as soon as a chunk reaches them,
//! and the copies left in the main module return nothing: **every** `asset!` in
//! the app then resolves to the empty string. The symptom is not a crash but a
//! site with no stylesheet, no font and no icons, which is what shipped in the
//! first deploy. Measured: with `/@handle` split, `<link rel=stylesheet>` had
//! `href=""` and the body kept its 8px user-agent margin; with it resident,
//! both come back. `cardioid` uses no `asset!`, so it splits safely.
//!
//! Anything added here must be checked the same way: build with
//! `--wasm-split`, load `/`, and confirm the stylesheet href is a real
//! `/assets/main-*.css` and that the graph icons are fetched.

use dioxus::prelude::*;

use super::cardioid::CardioidInner;

#[cfg(feature = "split")]
fn cardioid_body(_: ()) -> Element {
    rsx! { CardioidInner {} }
}

#[cfg(feature = "split")]
static CARDIOID: wasm_split::LazyLoader<(), Element> =
    wasm_split::lazy_loader!(extern "cardioid" fn cardioid_body(props: ()) -> Element);

#[component]
pub fn Cardioid() -> Element {
    #[cfg(not(feature = "split"))]
    return rsx! { CardioidInner {} };

    #[cfg(feature = "split")]
    {
        let loaded = use_resource(|| async { CARDIOID.load().await });
        let ready = matches!(&*loaded.read_unchecked(), Some(true));
        if ready {
            CARDIOID.call(()).unwrap_or_else(|_| rsx! {})
        } else {
            rsx! {}
        }
    }
}
