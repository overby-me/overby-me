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
//!
//! The pipe is a pty, so what comes out of it is a terminal's idea of a line
//! ending: every line feed has a carriage return in front of it. That is not a
//! detail the hacks are insulated from. Two of them feed these bytes to a
//! terminal emulator, which moves down a line on a line feed and back to the
//! left margin only on a carriage return, and the rest are written expecting to
//! see the pair. So the channel puts the return in, as the line discipline it
//! stands in for would.

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
    /// The passage folded to `columns`, empty until a hack asks for a width.
    wrapped: String,
    /// A line feed owed, after the carriage return sent in front of it.
    lf_owed: bool,
    /// What the hack last said about its layout. Upstream passes this down the
    /// pipe so `xscreensaver-text` can wrap to the right width.
    pub(crate) columns: i32,
    pub(crate) max_lines: i32,
    /// When the hack first asked and the host had nothing, so a source that
    /// never answers does not leave a saver with no words at all.
    waiting_since: Option<f64>,
}

/// How long to wait for a host before falling back to the compiled-in
/// passage, in seconds.
///
/// The same reasoning as [`super::image`]'s: long enough to fetch over a slow
/// connection, short enough that a broken source does not look like a hang.
/// Without it a text source that fails leaves the screen empty forever, which
/// is worse than the wrong words.
const PATIENCE: f64 = 20.0;

impl TextChannel {
    /// Host side: take some words, folded to the width the hack asked for.
    ///
    /// The folding is not the host's job to remember. Upstream sends the width
    /// down the pipe (`textclient_reshape`) and `xscreensaver-text` wraps its
    /// output to it, because several hacks lay the words out exactly as they
    /// arrive and would otherwise run a paragraph off the side of the screen.
    /// The compiled-in fallback already wraps; text from a host has to as
    /// well, or the two paths look different and only one of them is right.
    pub(crate) fn deliver(&mut self, s: &str) {
        let folded;
        let text = if self.columns > 0 {
            folded = wrap(s, self.columns as usize);
            &folded
        } else {
            s
        };
        self.pending.extend(text.as_bytes());
    }

    /// `textclient_getc`: the next character, or `None` if there is none to be
    /// had this instant.
    pub(crate) fn getc(&mut self, now: f64) -> Option<u8> {
        if self.lf_owed {
            self.lf_owed = false;
            return Some(b'\n');
        }
        match self.next_byte(now) {
            Some(b'\n') => {
                self.lf_owed = true;
                Some(b'\r')
            }
            other => other,
        }
    }

    fn next_byte(&mut self, now: f64) -> Option<u8> {
        if let Some(c) = self.pending.pop_front() {
            self.waiting_since = None;
            return Some(c);
        }
        if self.host_supplies {
            // Waiting on the host. Upstream returns -1 here and the hacks are
            // written to cope, because a pipe is not always ready.
            self.requested = true;
            let since = *self.waiting_since.get_or_insert(now);
            if now - since < PATIENCE {
                return None;
            }
            // The host has had long enough. Fall through to the passage, and
            // keep the request standing so a source that recovers is used.
        }
        let text = if self.wrapped.is_empty() {
            FALLBACK
        } else {
            &self.wrapped
        };
        let c = text.as_bytes()[self.at % text.len()];
        self.at = (self.at + 1) % text.len();
        Some(c)
    }

    /// `textclient_reshape`: tell the source how wide the page is now.
    ///
    /// Upstream this goes down the pipe and `xscreensaver-text` folds its output
    /// to that width, which several hacks depend on: they lay the words out
    /// exactly as they arrive and would otherwise draw one line off the edge of
    /// the screen. The fallback has to do the same.
    pub(crate) fn reshape(&mut self, columns: i32, max_lines: i32) {
        if columns != self.columns {
            self.wrapped = if columns > 0 {
                wrap(FALLBACK, columns as usize)
            } else {
                String::new()
            };
            self.at = 0;
        }
        self.columns = columns;
        self.max_lines = max_lines;
    }
}

/// Fold a passage to a column width, breaking between words and keeping the
/// paragraph breaks that are already there.
fn wrap(text: &str, columns: usize) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / columns.max(1));
    for line in text.split('\n') {
        let mut width = 0;
        for word in line.split_whitespace() {
            if width > 0 && width + 1 + word.chars().count() > columns {
                out.push('\n');
                width = 0;
            } else if width > 0 {
                out.push(' ');
                width += 1;
            }
            out.push_str(word);
            width += word.chars().count();
        }
        out.push('\n');
    }
    out
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
        let n = FALLBACK.len() + FALLBACK.matches('\n').count();
        let first: Vec<u8> = (0..n).filter_map(|_| c.getc(0.0)).collect();
        assert_eq!(first.len(), n, "every character came back");
        assert_eq!(first[0], b'A');
        // And it wraps rather than running dry.
        assert_eq!(c.getc(0.0), Some(b'A'));
    }

    /// Text from a host is folded to the width the hack asked for, exactly as
    /// the compiled-in passage is. Without this a saver that lays words out
    /// as they arrive runs its lines off the side of the screen, and the two
    /// paths through the channel disagree.
    #[test]
    fn delivered_text_is_folded_to_the_page() {
        let mut c = TextChannel::default();
        c.host_supplies = true;
        c.reshape(20, 24);
        c.deliver("the quick brown fox jumps over the lazy dog");

        let mut got = String::new();
        while let Some(b) = c.getc(0.0) {
            got.push(b as char);
        }
        // Carriage returns are the channel's terminal line endings; the
        // question here is only where the breaks fell.
        for line in got.replace('\r', "").lines() {
            assert!(
                line.chars().count() <= 20,
                "line ran to {} columns: {line:?}",
                line.chars().count()
            );
        }
        assert!(got.contains("quick"), "the words themselves survived");

        // With no width asked for, it is passed through untouched.
        let mut c = TextChannel::default();
        c.host_supplies = true;
        c.deliver("a b c");
        let mut got = String::new();
        while let Some(b) = c.getc(0.0) {
            got.push(b as char);
        }
        assert_eq!(got, "a b c");
    }

    /// A host that says it supplies text and then never does must not leave a
    /// saver with nothing to read. The image channel falls back to colour
    /// bars; this one falls back to the passage, and keeps the request
    /// standing so a source that recovers is still used.
    #[test]
    fn a_silent_host_eventually_gets_the_passage() {
        let mut c = TextChannel::default();
        c.host_supplies = true;
        assert_eq!(c.getc(0.0), None, "it should wait at first");
        assert_eq!(c.getc(PATIENCE - 0.1), None, "it gave up too early");
        assert_eq!(
            c.getc(PATIENCE + 0.1),
            Some(b'A'),
            "it never gave up, so the screen stays empty"
        );
        assert!(c.requested, "the request should still stand");

        // And a host that does answer is preferred over the passage again.
        c.pending.extend(b"hello");
        assert_eq!(c.getc(PATIENCE + 0.2), Some(b'h'));
    }

    /// Every line ends the way it would coming out of a terminal, because the
    /// hacks reading it are written for a terminal: one of them moves the
    /// cursor down but not back, and would walk off the right edge without
    /// the return.
    #[test]
    fn a_line_ends_with_a_return_and_then_a_feed() {
        let mut c = TextChannel::default();
        c.reshape(40, 15);
        let text: String = (0..500)
            .filter_map(|_| c.getc(0.0))
            .map(char::from)
            .collect();
        assert!(text.contains("\r\n"));
        assert!(!text.contains('\n') || !text.replace("\r\n", "").contains('\n'));

        // Including text the host pushes, which upstream would also have been
        // read back through the terminal.
        let mut c = TextChannel {
            host_supplies: true,
            ..TextChannel::default()
        };
        c.pending.extend(b"one\ntwo\n");
        let text: String = (0..8).filter_map(|_| c.getc(0.0)).map(char::from).collect();
        assert_eq!(text, "one\r\ntwo");
    }

    /// A hack that says how wide its page is gets lines that fit it, because
    /// upstream's text source folds to that width and the hacks trust it.
    #[test]
    fn asking_for_a_width_gets_lines_that_fit() {
        let mut c = TextChannel::default();
        c.reshape(40, 15);
        let text: String = (0..2000)
            .filter_map(|_| c.getc(0.0))
            .map(char::from)
            .collect();
        assert!(text.starts_with("Alice was beginning"));
        let lines: Vec<&str> = text.lines().collect();
        let widest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        assert!(widest <= 40, "too wide: {widest} columns");
        assert!(
            lines.iter().filter(|l| !l.is_empty()).count() > 20,
            "should have been folded into many lines"
        );

        // And with no width asked for, the passage arrives as it is written.
        let mut c = TextChannel::default();
        let text: String = (0..300)
            .filter_map(|_| c.getc(0.0))
            .map(char::from)
            .collect();
        assert!(text.lines().next().is_some_and(|l| l.chars().count() > 40));
    }

    /// A host that has said it will supply text but has not yet gets nothing,
    /// rather than a silent fallback that would mix two sources together.
    #[test]
    fn a_host_that_has_not_answered_yields_nothing() {
        let mut c = TextChannel {
            host_supplies: true,
            ..TextChannel::default()
        };
        assert_eq!(c.getc(0.0), None);
        assert!(c.requested, "and the request is visible to the host");

        c.pending.extend(b"hi");
        assert_eq!(c.getc(0.0), Some(b'h'));
        assert_eq!(c.getc(0.0), Some(b'i'));
        assert_eq!(c.getc(0.0), None);
    }
}
