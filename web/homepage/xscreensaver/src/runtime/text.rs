//! Where a saver's words come from.
//!
//! Ten or so of the upstream hacks read text and put it on the screen: they
//! glide it in from the edges, rain it down a terminal, scroll it past. On a
//! desktop each of them opens a pipe to `xscreensaver-text`, which prints a
//! file, a URL, the output of a program, or the date, and reads bytes back a
//! character at a time (`utils/textclient.c`).
//!
//! This crate cannot start a process or fetch a URL, and should not: it is
//! dependency-free and runs in a `cargo test` with no browser. So the contract
//! is the same channel [`super::image`] uses. A hack asks for characters, the
//! host pushes text in whenever it has some, and if nothing is going to answer
//! the hack reads the passage compiled in below instead.
//!
//! That keeps the collection testable: the native tests register no host, so
//! every text-consuming saver runs against the same words every time.
//!
//! Upstream hands the hack bytes, not lines, and returns "nothing right now"
//! rather than blocking, because the program at the other end of the pipe may
//! be slow. Both of those are kept: a host that has not answered yet simply
//! yields nothing this frame.

use std::collections::VecDeque;

/// The runtime's half of the channel. Lives on [`super::Dpy`].
#[derive(Default)]
pub struct TextChannel {
    /// Set when the host has said it can supply text. Without it the compiled
    /// in passage is served instead, which is what makes the native tests work
    /// without ceremony.
    pub(crate) host_supplies: bool,
    /// Set when a hack has asked for text and the host has not yet answered.
    pub(crate) requested: bool,
    /// What the host has pushed and the hack has not yet read.
    pub(crate) pending: VecDeque<u8>,
    /// How far through the compiled-in passage the fallback has read.
    at: usize,
    /// What the hack last said about its layout. Upstream passes this down the
    /// pipe so `xscreensaver-text` can wrap to the right width.
    pub(crate) columns: i32,
    pub(crate) max_lines: i32,
}

impl TextChannel {
    /// `textclient_getc`: the next character, or `None` if there is none to be
    /// had this instant.
    pub(crate) fn getc(&mut self) -> Option<u8> {
        if let Some(c) = self.pending.pop_front() {
            return Some(c);
        }
        if self.host_supplies {
            // Waiting on the host. Upstream returns -1 here and the hacks are
            // written to cope, because a pipe is not always ready.
            self.requested = true;
            return None;
        }
        let c = FALLBACK.as_bytes()[self.at];
        self.at = (self.at + 1) % FALLBACK.len();
        Some(c)
    }

    /// `textclient_reshape`: tell the source how wide the page is now.
    pub(crate) fn reshape(&mut self, columns: i32, max_lines: i32) {
        self.columns = columns;
        self.max_lines = max_lines;
    }
}

/// The words a saver gets when nothing else is going to supply any.
///
/// Public domain, out of copyright since well before there were screen savers,
/// and chosen because it is what a page of text is supposed to look like: mixed
/// case, punctuation of several kinds, long words and short ones, and sentences
/// that vary enough in length to show a layout doing its job.
const FALLBACK: &str = "\
Alice was beginning to get very tired of sitting by her sister on the bank, \
and of having nothing to do: once or twice she had peeped into the book her \
sister was reading, but it had no pictures or conversations in it, \"and what \
is the use of a book,\" thought Alice, \"without pictures or conversations?\"\n\
\n\
So she was considering in her own mind, as well as she could, for the hot day \
made her feel very sleepy and stupid, whether the pleasure of making a \
daisy-chain would be worth the trouble of getting up and picking the daisies, \
when suddenly a White Rabbit with pink eyes ran close by her.\n\
\n\
There was nothing so very remarkable in that; nor did Alice think it so very \
much out of the way to hear the Rabbit say to itself, \"Oh dear! Oh dear! I \
shall be late!\" But when the Rabbit actually took a watch out of its \
waistcoat-pocket, and looked at it, and then hurried on, Alice started to her \
feet, for it flashed across her mind that she had never before seen a rabbit \
with either a waistcoat-pocket, or a watch to take out of it, and burning with \
curiosity, she ran across the field after it, and was just in time to see it \
pop down a large rabbit-hole under the hedge.\n\
\n\
In another moment down went Alice after it, never once considering how in the \
world she was to get out again.\n\
\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_no_host_the_passage_repeats_forever() {
        let mut c = TextChannel::default();
        let first: Vec<u8> = (0..FALLBACK.len()).filter_map(|_| c.getc()).collect();
        assert_eq!(first.len(), FALLBACK.len(), "every character came back");
        assert_eq!(first[0], b'A');
        // And it wraps rather than running dry.
        assert_eq!(c.getc(), Some(b'A'));
    }

    /// A host that has said it will supply text but has not yet gets nothing,
    /// rather than a silent fallback that would mix two sources together.
    #[test]
    fn a_host_that_has_not_answered_yields_nothing() {
        let mut c = TextChannel {
            host_supplies: true,
            ..TextChannel::default()
        };
        assert_eq!(c.getc(), None);
        assert!(c.requested, "and the request is visible to the host");

        c.pending.extend(b"hi");
        assert_eq!(c.getc(), Some(b'h'));
        assert_eq!(c.getc(), Some(b'i'));
        assert_eq!(c.getc(), None);
    }
}
