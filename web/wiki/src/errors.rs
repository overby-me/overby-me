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

thread_local! {
    static LAST_SHOWN: std::cell::Cell<f64> = const { std::cell::Cell::new(f64::NEG_INFINITY) };
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
        Failure::Broken => t("error.somethingWentWrong"),
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
}
