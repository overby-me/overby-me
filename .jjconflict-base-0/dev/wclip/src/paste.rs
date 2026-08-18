//! `wclip -o` — print the current selection to standard output.

use std::io::{self, Write};

use crate::args::{Args, Selection};
use crate::proto::{self, State};
use crate::wire::Connection;

/// Preference order when choosing a MIME type with no explicit `-target`.
const PASTE_PREFS: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
    "TEXT",
];

/// The pseudo-target that lists the available MIME types instead of data.
const TARGETS: &str = "TARGETS";

pub fn run(args: &Args) -> io::Result<()> {
    let mut conn = Connection::connect(args.display.as_deref())?;
    let mut st = State::new();
    proto::setup(&mut conn, &mut st)?;

    let primary = args.selection == Selection::Primary;
    if primary && !st.primary_supported() {
        return Err(io::Error::other(
            "the compositor does not support the primary selection",
        ));
    }

    // A roundtrip delivers the current offers and which one is selected.
    proto::roundtrip(&mut conn, &mut st)?;

    let offer = if primary {
        st.primary_selection
    } else {
        st.selection
    };
    let Some(offer) = offer else {
        // Empty selection: print nothing, like xclip.
        return Ok(());
    };

    let mimes = st.offer_mimes(offer).to_vec();

    if args.target.as_deref() == Some(TARGETS) {
        let mut out = io::stdout().lock();
        for mime in &mimes {
            writeln!(out, "{mime}")?;
        }
        return Ok(());
    }

    let chosen = pick_mime(args.target.as_deref(), &mimes).ok_or_else(|| {
        io::Error::other(match &args.target {
            Some(t) => format!("the selection does not offer the target '{t}'"),
            None => "the selection offers no data".to_string(),
        })
    })?;

    let mut data = proto::receive(&mut conn, offer, &chosen)?;
    if args.rmlastnl && data.last() == Some(&b'\n') {
        data.pop();
    }

    let mut out = io::stdout().lock();
    out.write_all(&data)?;
    out.flush()
}

/// Choose which offered MIME type to request.
///
/// With an explicit target, require an exact match. Otherwise fall back through
/// the common text types, then any `text/*`, then whatever is offered first.
fn pick_mime(target: Option<&str>, mimes: &[String]) -> Option<String> {
    if let Some(t) = target {
        return mimes.iter().find(|m| m.as_str() == t).cloned();
    }
    for pref in PASTE_PREFS {
        if let Some(m) = mimes.iter().find(|m| m.as_str() == *pref) {
            return Some(m.clone());
        }
    }
    if let Some(m) = mimes.iter().find(|m| m.starts_with("text/")) {
        return Some(m.clone());
    }
    mimes.first().cloned()
}
