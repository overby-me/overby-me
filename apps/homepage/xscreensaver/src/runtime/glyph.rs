//! Where a saver's letters come from, when one font is not enough.
//!
//! [`super::font`] is one bitmap font compiled in, and [`super::texfont`]
//! magnifies it. That is enough for every saver that puts words on the screen,
//! because they are all writing Latin. `unicrud` is not: it picks a codepoint
//! anywhere from 0 to 0x2F800 and draws it four inches high, so it wants a
//! font covering essentially all of Unicode.
//!
//! Embedding one is not the answer. The host is a browser, which already has
//! fonts and can draw any character it has a glyph for, so this is the same
//! channel [`super::image`] and [`super::tiles`] are: the saver asks for a
//! codepoint at a size, the host draws it and hands back the pixels.
//!
//! That gives better coverage than upstream rather than worse. Upstream is
//! limited to whatever fonts the X server was configured with, and its own
//! answer to a codepoint the font lacks is to notice the glyph came out blank
//! and pick a different character. The same thing happens here, except that
//! the host does the noticing: a codepoint it cannot draw comes back as
//! `None`, and the saver rolls again.
//!
//! With no host at all there is nothing to draw with, so the saver falls back
//! to the compiled-in font and the handful of characters it does have, which
//! is what the native tests see.

use super::fb::XImage;

/// The runtime's half of the channel.
#[derive(Default)]
pub struct GlyphChannel {
    /// Set when the host has said it can draw. Without it a request is
    /// dropped and the saver is told so at once.
    pub(crate) host_supplies: bool,
    /// The codepoint and pixel size asked for, not yet taken by the host.
    pub(crate) wanted: Option<(u32, i32)>,
    /// What the host drew, not yet collected. `None` inside means the host
    /// has no glyph for it, which is an answer rather than a failure.
    pub(crate) ready: Option<(u32, Option<XImage>)>,
}

impl GlyphChannel {
    /// Hack side: ask for a codepoint, drawn about `size` pixels tall.
    ///
    /// Asking again before the first is answered replaces it: a saver that
    /// has moved on to another character has no use for the old one.
    pub(crate) fn request(&mut self, codepoint: u32, size: i32) {
        if self.host_supplies {
            self.wanted = Some((codepoint, size.max(1)));
        }
    }

    /// Hack side: the answer, once there is one.
    pub(crate) fn take(&mut self) -> Option<(u32, Option<XImage>)> {
        self.ready.take()
    }
}
