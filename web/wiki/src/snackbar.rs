use dioxus::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct SnackbarMessage {
    pub text: String,
    pub id: u64,
    /// Whether this one interrupts. A failure is worth cutting across whatever
    /// a screen reader is currently saying; "sent" is not.
    pub urgent: bool,
}

static SNACKBAR_COUNTER: GlobalSignal<u64> = Signal::global(|| 0);
/// The stack of visible snackbars (newest last), capped at [`MAX_SNACK`].
pub static SNACKBAR: GlobalSignal<Vec<SnackbarMessage>> = Signal::global(Vec::new);

/// How many snackbars stack at once before the oldest is dropped (Notistack's
/// `maxSnack`).
const MAX_SNACK: usize = 3;

/// How long a message stays. A failure gets longer, because it is the one you
/// may need to act on and the one a screen reader is most likely to reach late:
/// `aria-live` waits for the current utterance to finish, and anything removed
/// before it gets there is never spoken at all. Three seconds is not enough to
/// survive that; WCAG's timing guidance would rather it were longer still.
const DISMISS_MS: u32 = 3_000;
const DISMISS_URGENT_MS: u32 = 9_000;

/// Show a snackbar that auto-dismisses. Stacks up to [`MAX_SNACK`] at once, and
/// skips a message identical to the newest one still showing (Notistack's
/// `preventDuplicate`).
pub fn show_snackbar(text: &str) {
    // A generic-failure toast is the one case where NOBODY knows what happened:
    // the reader is told only that something did, and the call sites that raise
    // it are exactly the ones that threw the error away.
    //
    // So it carries the cause out with it. Every classified failure is noted as
    // it passes (`errors::note_failure`), including the ones deliberately kept
    // off the wire -- a refusal, a dropped connection -- because the reasoning
    // that keeps those quiet does not survive them reaching the screen as a
    // shrug. A refusal nobody sees is routine; a refusal a reader walks into is
    // a wall with no sign on it.
    //
    // Reported one report earlier: a reader who is blind hit this twice while
    // adding content, and the record said "error toast shown to user: Noget gik
    // galt!" and nothing else.
    let urgent = text == crate::i18n::t("error.somethingWentWrong");
    if urgent {
        match crate::errors::recent_failure() {
            Some(cause) => log::warn!("error toast shown to user: {text} -- caused by {cause}"),
            // Nothing recent enough to blame. Said plainly rather than left to
            // look like a failure with no cause: it means the toast came from
            // somewhere that never went through the classifier.
            None => log::warn!("error toast shown to user: {text} -- no failure recorded"),
        }
    }
    // preventDuplicate: don't queue the same text twice in a row.
    if SNACKBAR.read().last().is_some_and(|m| m.text == text) {
        return;
    }
    let id = *SNACKBAR_COUNTER.read() + 1;
    *SNACKBAR_COUNTER.write() = id;
    {
        let mut q = SNACKBAR.write();
        q.push(SnackbarMessage {
            text: text.to_string(),
            id,
            urgent,
        });
        // maxSnack: drop the oldest beyond the cap.
        while q.len() > MAX_SNACK {
            q.remove(0);
        }
    }

    // Auto-dismiss this specific message after 3 seconds.
    //
    // On the ROOT scope, never the caller's. `spawn` attaches the task to the
    // component that called it and Dioxus drops it the moment that component
    // unmounts, but a snackbar is usually raised by an action that unmounts its
    // own caller: a dialog closing, a delete, a navigation. The timer was then
    // cancelled before it fired and the message stayed on screen for the rest of
    // the session, taking its text with it (`preventDuplicate` below then
    // silently swallowed every later message with the same text).
    let holds_for = if urgent {
        DISMISS_URGENT_MS
    } else {
        DISMISS_MS
    };
    dioxus::core::spawn_forever(async move {
        gloo_timers::future::TimeoutFuture::new(holds_for).await;
        SNACKBAR.write().retain(|m| m.id != id);
    });
}

/// Snackbar component — render at the root level. Renders the whole stack.
///
/// THE ANNOUNCEMENT IS NOT THE TOAST. The two live regions below are always in
/// the document, empty, and the visible toasts carry no ARIA at all.
///
/// It used to be the other way round: nothing was rendered until a message
/// existed, and each toast was its own `aria-live` node. A live region has to be
/// in the DOM BEFORE its content changes for the change to be noticed -- one
/// that appears with its text already in it is commonly not announced at all --
/// so the arrangement that reads as obvious is the one that says nothing. Three
/// seconds later the node was removed, which is also less time than `polite`
/// may take to reach it, since it waits for the current utterance to finish.
///
/// This came from a report by a reader who uses a screen reader: an action
/// failed, the only feedback was a toast, and the toast is the part that may
/// never have been spoken.
///
/// Two regions, not one, because the register matters: a failure interrupts
/// (`alert`/`assertive`), anything else waits its turn (`status`/`polite`).
/// Both are `aria-atomic` so the whole message is read rather than the diff.
#[component]
pub fn Snackbar() -> Element {
    let messages = SNACKBAR.read().clone();
    let urgent: Vec<&SnackbarMessage> = messages.iter().filter(|m| m.urgent).collect();
    let polite: Vec<&SnackbarMessage> = messages.iter().filter(|m| !m.urgent).collect();
    rsx! {
        div {
            class: "sr-only",
            role: "alert",
            aria_live: "assertive",
            aria_atomic: "true",
            {urgent.iter().map(|m| m.text.clone()).collect::<Vec<_>>().join(". ")}
        }
        div {
            class: "sr-only",
            role: "status",
            aria_live: "polite",
            aria_atomic: "true",
            {polite.iter().map(|m| m.text.clone()).collect::<Vec<_>>().join(". ")}
        }
        div { class: "snackbar-stack", aria_hidden: "true",
            for msg in messages.iter() {
                div { class: "snackbar", key: "{msg.id}", "{msg.text}" }
            }
        }
    }
}
