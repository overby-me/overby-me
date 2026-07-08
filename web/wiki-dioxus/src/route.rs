use dioxus::prelude::*;

use crate::components::{
    auth::{Login, Register, ResetPassword, SetPassword, Unverified},
    home::HomeApp,
    layout::Layout,
    loader::PathPage,
};

#[derive(Routable, Clone, Debug, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[layout(Layout)]
    #[route("/")]
    HomeApp {},

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

    // `app` carries the `?app=` query (vote/speak/member/editor/sort). Modelling
    // it in the route keeps Dioxus from stripping the query on navigation, so
    // the app rail and deep links work; absent, it is None (URL has no `app=`).
    #[route("/:..segments?:app")]
    PathPage {
        segments: Vec<String>,
        app: Option<String>,
    },
}
