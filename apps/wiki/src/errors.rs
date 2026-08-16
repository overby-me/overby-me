//! What to do with a failure the user did not ask about.
//!
//! Almost every query in this app ends in `unwrap_or_default()`, so a failure
//! renders as an empty list and a refusal, a dropped connection and a genuinely
//! empty folder all look the same. That is fine for the last one and wrong for
//! the other two.
//!
//! Rather than change 188 call sites, the judgement lives at the one place every
//! query and mutation already passes through ([`crate::graphql::execute`]): work
//! out what KIND of failure it was, and tell the user only about the kinds they
//! can do something about.

use crate::i18n::t;

/// What a failed GraphQL call means for the person looking at the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// The server was not reached, or did not answer. Worth saying: it will
    /// probably work again in a moment, and nothing they did caused it.
    Offline,
    /// The server answered "no". Expected and constant in normal use: the public
    /// role cannot read most nodes, a member cannot see an owner's tools. The UI
    /// already hides what you may not do, so saying it aloud would be noise on
    /// every page.
    Refused,
    /// Anything else: a malformed response, a server fault, a schema drift. Rare,
    /// and always a bug worth surfacing rather than swallowing.
    Broken,
}

impl Failure {
    /// A short tag for the log line, so a console reader can tell the kinds apart
    /// without re-deriving the classification.
    pub fn label(self) -> &'static str {
        match self {
            Failure::Offline => "offline",
            Failure::Refused => "refused",
            Failure::Broken => "broken",
        }
    }
}

/// Classify a GraphQL failure from its message.
///
/// Pure, so it is testable off the wasm target — which matters, because getting
/// this wrong in either direction is bad: a refusal misread as breakage means a
/// toast on every page for signed-out readers, and breakage misread as a refusal
/// means silence exactly when something is wrong.
pub fn classify(msg: &str) -> Failure {
    let m = msg.to_ascii_lowercase();

    // Hasura's vocabulary for "you may not": permission checks, and the schema
    // itself hiding fields a role cannot select (which reads as "not found").
    // Verified against this deployment rather than guessed: asking for a column
    // the role cannot select answers "field 'deleted_at' not found in type:
    // 'nodes'", and any mutation as the public role answers "no mutations exist".
    const REFUSALS: &[&str] = &[
        "permission",
        "access denied",
        "not allowed",
        "unauthorized",
        "not found in type",
        "no mutations exist",
        "no queries exist",
        "no subscriptions exist",
        "no queries available",
        "validation-failed",
    ];
    if REFUSALS.iter().any(|needle| m.contains(needle)) {
        return Failure::Refused;
    }

    // reqwest's transport failures, and the browser's own wording for a request
    // that never completed.
    const OFFLINE: &[&str] = &[
        "error sending request",
        "failed to fetch",
        "networkerror",
        "network error",
        "timed out",
        "timeout",
        "connection",
        "dns",
        "unreachable",
    ];
    if OFFLINE.iter().any(|needle| m.contains(needle)) {
        return Failure::Offline;
    }

    Failure::Broken
}

/// The shortest interval between two failure toasts, in milliseconds.
///
/// A page mounts a dozen queries and they fail together, so without this a
/// single dropped connection would stack a dozen identical toasts. One is the
/// message; twelve is a fault of its own.
const THROTTLE_MS: f64 = 8000.0;

/// Log a failure the caller has already dealt with on screen, at the level its
/// kind deserves.
///
/// A render site that reaches for `log::error!` directly undoes the judgement
/// this module exists to make. `logging.rs` ships warn and error to the log
/// sink, so a dropped connection filed at error becomes a stored fault report
/// per reader per bad moment, which at a congress is the whole hall reporting
/// that the wifi is bad. Worse, these sit in render bodies, so they fire again
/// on every re-render for as long as the error card is on screen.
///
/// [`crate::graphql::execute`] has already logged the same failure with its
/// classification. This is the caller's own note, at a level that matches.
pub fn log_handled(what: &str, msg: impl std::fmt::Display) {
    let msg = msg.to_string();
    note_failure(format!("{what}: {msg}"));
    match classify(&msg) {
        Failure::Broken => log::error!("{what}: {msg}"),
        failure => log::info!("{what} ({}): {msg}", failure.label()),
    }
}

thread_local! {
    static LAST_SHOWN: std::cell::Cell<f64> = const { std::cell::Cell::new(f64::NEG_INFINITY) };
    /// The last failure anyone classified, and when: `(summary, ms since load)`.
    static LAST_FAILURE: std::cell::RefCell<Option<(String, f64)>> =
        const { std::cell::RefCell::new(None) };
}

/// Milliseconds since the page loaded, for pairing a failure with the toast it
/// produced. `performance.now()`, the same clock `up_ms` in a report uses.
fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(f64::NAN)
}

/// Remember what just went wrong, so that if it ends up as a shrug on screen the
/// shrug can say what it was.
///
/// Every classified failure is noted here, including the quiet ones. A refusal
/// or a dropped connection is deliberately not reported on its own -- at a
/// congress that would be thousands of records saying the hall has bad
/// reception -- but the same failure becomes worth reporting the moment it
/// reaches the reader as "Noget gik galt!", because then nobody knows anything:
/// not the reader, who is told only that something did, and not us.
pub fn note_failure(summary: impl std::fmt::Display) {
    let summary = summary.to_string();
    LAST_FAILURE.with(|c| *c.borrow_mut() = Some((summary, now_ms())));
}

/// What went wrong just before now, if anything did recently enough to be the
/// cause of it.
///
/// The window is short on purpose. A failure from a minute ago is not the reason
/// for a message on screen now, and guessing that it is would file a plausible
/// wrong answer -- which is worse than filing none, because it reads as evidence.
pub fn recent_failure() -> Option<String> {
    let now = now_ms();
    LAST_FAILURE.with(|c| {
        c.borrow()
            .as_ref()
            .and_then(|(summary, at)| still_the_cause(*at, now).then(|| summary.clone()))
    })
}

/// Whether a failure noted at `noted_at` is recent enough to be the cause of
/// something happening at `now`. Pure, so the window is testable off-wasm.
fn still_the_cause(noted_at: f64, now: f64) -> bool {
    const STILL_THE_CAUSE_MS: f64 = 10_000.0;
    // A NaN clock (no `window`, i.e. not a browser) must blame nothing rather
    // than everything: every comparison against NaN is false, which is the
    // answer wanted here, but saying so is better than relying on it.
    if !now.is_finite() || !noted_at.is_finite() {
        return false;
    }
    (0.0..STILL_THE_CAUSE_MS).contains(&(now - noted_at))
}

/// Tell the user about a failed call, if it is one of the kinds worth telling
/// them about and one has not just been shown.
///
/// The message is always logged by the caller regardless; this only decides what
/// reaches the screen.
pub fn report(failure: Failure) {
    let text = match failure {
        Failure::Refused => return,
        Failure::Offline => t("error.offline"),
        // Broken is always a bug, and one that is now filed automatically (see
        // `graphql::execute`). Saying so is the difference between a dead end and
        // a message the reader can act on: they know it is known, and they can
        // quote it. What the app cannot do is explain a schema drift to a
        // delegate, so it does not try.
        Failure::Broken => t("error.somethingWentWrongReported"),
    };
    let now = js_sys::Date::now();
    let show = LAST_SHOWN.with(|last| {
        if now - last.get() < THROTTLE_MS {
            false
        } else {
            last.set(now);
            true
        }
    });
    if show {
        crate::snackbar::show_snackbar(&text);
    }
}

/// Say that what is on screen is a copy from before the connection went.
///
/// Separate from [`report`] only in wording: a reader who can still see the
/// page needs to know it may have moved on without them, which is a different
/// message from "that did not work". Shares the throttle, so a page that
/// restores several reads from the cache says it once.
pub fn report_offline_copy() {
    let now = js_sys::Date::now();
    let show = LAST_SHOWN.with(|last| {
        if now - last.get() < THROTTLE_MS {
            false
        } else {
            last.set(now);
            true
        }
    });
    if show {
        crate::snackbar::show_snackbar(&t("error.offlineCopy"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_stay_quiet() {
        // The first two are verbatim from this deployment, asked as the public
        // role: a column it may not select, and any mutation at all.
        for msg in [
            "field 'deleted_at' not found in type: 'nodes'",
            "no mutations exist",
            "permission denied for relation nodes",
            "check constraint of an insert permission has failed",
            "Access denied",
        ] {
            assert_eq!(classify(msg), Failure::Refused, "{msg}");
        }
    }

    #[test]
    fn transport_failures_are_offline() {
        for msg in [
            "error sending request for url (https://x/v1/graphql)",
            "TypeError: Failed to fetch",
            "operation timed out",
        ] {
            assert_eq!(classify(msg), Failure::Offline, "{msg}");
        }
    }

    #[test]
    fn anything_else_is_broken() {
        // A server fault and a schema drift both need to be heard about.
        assert_eq!(classify("internal server error"), Failure::Broken);
        assert_eq!(
            classify("expected Int, found String at data.index"),
            Failure::Broken
        );
    }

    #[test]
    fn classification_is_case_insensitive() {
        assert_eq!(classify("PERMISSION DENIED"), Failure::Refused);
        assert_eq!(classify("Network Error"), Failure::Offline);
    }

    /// What a generic "something went wrong" toast is allowed to blame.
    ///
    /// The point of the window is that a wrong cause is worse than none: it
    /// reads as evidence, and someone will act on it. A failure from a minute
    /// ago did not produce the message on screen now.
    #[test]
    fn only_a_failure_from_just_now_may_be_blamed() {
        assert!(super::still_the_cause(1_000.0, 1_000.0), "the same instant");
        assert!(
            super::still_the_cause(1_000.0, 9_000.0),
            "8s later, still it"
        );
        assert!(!super::still_the_cause(1_000.0, 61_000.0), "a minute later");
        // Time cannot run backwards; a note from the future is a bug, not a cause.
        assert!(
            !super::still_the_cause(9_000.0, 1_000.0),
            "noted after the toast"
        );
        // No clock at all (not a browser) blames nothing rather than everything.
        assert!(!super::still_the_cause(f64::NAN, 1_000.0));
        assert!(!super::still_the_cause(1_000.0, f64::NAN));
    }
}
