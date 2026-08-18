//! What the drawer shows someone who is not signed in.
//!
//! It used to show nothing: the whole navigation body was gated on being signed
//! in, so a visitor got a bar reading Home, an account row, and no indication
//! that the site contains anything at all. For a wiki that is the one thing the
//! first screen has to answer.
//!
//! So: the places that are open to everyone, from the permission rows that make
//! them open (see `graphql::query_public_places`), and one line saying plainly
//! that the rest needs an account. An empty list that explains itself beats a
//! void, and it beats a login wall, which answers a question nobody asked.
//!
//! Same shape as the signed-in picker next to it ([`super::HomeList`]): a
//! section header with an icon avatar, then rows. Two states of one drawer, not
//! two designs.

use dioxus::prelude::*;

use crate::graphql;
use crate::i18n::t;
use crate::route::Route;

/// The open places, as the drawer's body (`as_cards: false`) or as a home-page
/// card (`as_cards: true`, which is where a phone sees it: the drawer is behind
/// a button there, and a visitor has no reason to press it).
#[component]
pub fn PublicPlaces(#[props(default)] as_cards: bool) -> Element {
    // No token on purpose. This asks what is open to anyone, and the answer must
    // not depend on who is asking.
    let places = crate::use_data_resource!(|()| async move {
        graphql::query_public_places(None).await.unwrap_or_default()
    });
    let places = places.read().clone().unwrap_or_default();

    let rows = rsx! {
        if places.is_empty() {
            p { class: "body-medium list-subheader", "{t(\"layout.openToAllEmpty\")}" }
        } else {
            for place in places.iter() {
                Link {
                    key: "{place.id}",
                    to: Route::PathPage {
                        segments: place.path.split('/').map(str::to_string).collect(),
                        app: None,
                    },
                    class: "list-item list-item-flush",
                    div { class: "avatar small",
                        {crate::components::loader::icon_el(&place.mime_id)}
                    }
                    div { class: "list-item-text",
                        div { class: "list-item-primary", "{place.name}" }
                    }
                }
            }
        }
        p { class: "body-small public-places-hint", "{t(\"layout.openToAllHint\")}" }
    };

    if as_cards {
        return rsx! {
            div { class: "card",
                div { class: "card-header",
                    div { class: "avatar small", span { class: "material-icons", "public" } }
                    h3 { class: "title-large", "{t(\"layout.openToAll\")}" }
                }
                div { class: "home-section-body", {rows} }
            }
        };
    }

    rsx! {
        div { class: "mt-2",
            div { class: "list-section-header",
                div { class: "avatar small", span { class: "material-icons", "public" } }
                h4 { class: "title-medium", "{t(\"layout.openToAll\")}" }
            }
            {rows}
        }
    }
}
