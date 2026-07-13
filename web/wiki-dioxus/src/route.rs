use dioxus::prelude::*;

use crate::components::{
    auth::{Login, Register, ResetPassword, SetPassword, Unverified},
    layout::Layout,
    loader::{Home, PathPage},
    profile::UserProfile,
};

#[derive(Routable, Clone, Debug, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    // `/` needs its own variant: Dioxus 0.7 serializes the empty catch-all below
    // to a bare "?" (relative, and it does not even re-parse), so `nav.push` and
    // `Link` targets for the root require a concrete route. It shares the same
    // resolver as every other path. Without an app it renders the `wiki/home`
    // welcome; `?app=editor` opens the owner-only root editor.
    #[layout(Layout)]
    #[route("/?:app")]
    Home { app: Option<String> },

    #[route("/user/login")]
    Login {},

    #[route("/user/register")]
    Register {},

    #[route("/user/reset-password")]
    ResetPassword {},

    #[route("/user/set-password")]
    SetPassword {},

    #[route("/user/unverified")]
    Unverified {},

    // Another user's public profile, by user id. Listed before the catch-all so
    // `/profile/<uuid>` resolves here rather than as a node path.
    #[route("/profile/:id")]
    UserProfile { id: String },

    // `app` carries the `?app=` query (vote/speak/member/editor/sort). Modelling
    // it in the route keeps Dioxus from stripping the query on navigation, so
    // the app rail and deep links work; absent, it is None (URL has no `app=`).
    #[route("/:..segments?:app")]
    PathPage {
        segments: Vec<String>,
        app: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_route_round_trips() {
        // The dedicated `/` route must produce an ABSOLUTE url (leading slash)
        // that re-parses back. The empty catch-all produced a relative "?" that
        // left `nav.push` on the current page. A trailing "?" (empty query) is the
        // same shape every no-app path uses (e.g. "/foo?"), so it is fine.
        let home = Route::Home { app: None };
        let s = home.to_string();
        assert!(s.starts_with('/'), "home url must be absolute, got {s:?}");
        assert_eq!(s.parse::<Route>().unwrap(), home);

        let editor = Route::Home {
            app: Some("editor".to_string()),
        };
        assert_eq!(editor.to_string(), "/?app=editor");
        assert_eq!(editor.to_string().parse::<Route>().unwrap(), editor);
    }
}
