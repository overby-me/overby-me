//! The two pages that are worth fetching only when they are visited.
//!
//! `cardioid` is a generative toy and `/@handle` is a WebGL force-directed
//! graph with its own renderer, simulation and atproto client. Neither is
//! wanted by anyone who came to read the homepage, and together they are a
//! few thousand lines that every visitor was downloading.
//!
//! The mechanism is `crate::pages::savers`' one, and the note there explains
//! why it is `wasm_split::lazy_loader!` by hand rather than
//! `#[component(lazy)]`: that macro puts every lazy component in a chunk
//! called "lazy", and it needs dioxus's own `wasm-split` feature, which makes
//! the router macro split every route and crashes the splitter.
//!
//! Each chunk's exported function returns the element *tree*, not the rendered
//! component. Building it with `rsx!` means the inner component is reachable
//! only from inside the chunk, so its code goes there, and dioxus still calls
//! it in a scope of its own. Calling the inner function directly instead would
//! run its hooks inside this wrapper's scope, where the hook order changes the
//! moment the chunk finishes loading.

use dioxus::prelude::*;

use super::atproto::AtProtoInner;
use super::cardioid::CardioidInner;

#[cfg(feature = "split")]
fn cardioid_body(_: ()) -> Element {
    rsx! { CardioidInner {} }
}

#[cfg(feature = "split")]
static CARDIOID: wasm_split::LazyLoader<(), Element> =
    wasm_split::lazy_loader!(extern "cardioid" fn cardioid_body(props: ()) -> Element);

#[cfg(feature = "split")]
fn atproto_body(handle: String) -> Element {
    rsx! { AtProtoInner { handle } }
}

#[cfg(feature = "split")]
static ATPROTO: wasm_split::LazyLoader<String, Element> =
    wasm_split::lazy_loader!(extern "atproto" fn atproto_body(props: String) -> Element);

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

#[component]
pub fn AtProto(handle: String) -> Element {
    #[cfg(not(feature = "split"))]
    return rsx! { AtProtoInner { handle } };

    #[cfg(feature = "split")]
    {
        let loaded = use_resource(|| async { ATPROTO.load().await });
        let ready = matches!(&*loaded.read_unchecked(), Some(true));
        if ready {
            ATPROTO.call(handle).unwrap_or_else(|_| rsx! {})
        } else {
            rsx! {}
        }
    }
}
