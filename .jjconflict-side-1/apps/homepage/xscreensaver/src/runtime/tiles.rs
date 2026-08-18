//! Where a saver's map tiles come from.
//!
//! [`super::image`] answers "give me a picture", one at a time, because that
//! is all any of the picture-consuming hacks ever wanted. `mapscroller` wants
//! something else: a few dozen specific images at once, each named by where it
//! belongs, arriving in whatever order the network manages, and it has to keep
//! drawing while they are in flight.
//!
//! So this is the same channel idea with a key on it. The hack asks for a URL
//! and says what to call the answer; the host fetches and hands back whatever
//! it has; the hack matches them up. Nothing here fetches anything, for the
//! same reason nothing in [`super::image`] does: this crate has no way to and
//! should not have one.
//!
//! Upstream's arrangement is a perl helper on the end of a pipe, forked
//! because "doing https from C code is untenable". A browser is the one place
//! where that sentence is false.

use super::fb::XImage;

/// How many requests to keep if the host is not draining them. A grid is a few
/// dozen tiles and a saver may ask again as it scrolls; beyond this the oldest
/// are dropped, because a tile nobody has fetched yet has probably scrolled
/// off the screen.
const MAX_WANTED: usize = 256;

/// The runtime's half of the channel.
#[derive(Default)]
pub struct TileChannel {
    /// Set when the host has said it can fetch. Without it a request is simply
    /// dropped and the hack draws whatever it draws for a missing tile, which
    /// is what the native tests see.
    pub(crate) host_supplies: bool,
    /// Asked for by the hack, not yet taken by the host.
    pub(crate) wanted: Vec<(u64, String)>,
    /// Fetched by the host, not yet collected by the hack.
    pub(crate) ready: Vec<(u64, Option<XImage>)>,
}

impl TileChannel {
    /// Hack side: ask for the image at `url`, to be called `key`.
    ///
    /// Asking twice for the same key is not an error and does not queue it
    /// twice: a saver re-examines its grid every frame and will ask for the
    /// same missing tile over and over until it arrives.
    pub(crate) fn request(&mut self, key: u64, url: String) {
        if !self.host_supplies || self.wanted.iter().any(|(k, _)| *k == key) {
            return;
        }
        if self.wanted.len() >= MAX_WANTED {
            self.wanted.remove(0);
        }
        self.wanted.push((key, url));
    }

    /// Hack side: the next answer, if one has arrived.
    ///
    /// `None` inside the pair means the host tried and failed, which the hack
    /// needs to know so it can stop asking and mark the tile.
    pub(crate) fn take(&mut self) -> Option<(u64, Option<XImage>)> {
        if self.ready.is_empty() {
            None
        } else {
            Some(self.ready.remove(0))
        }
    }
}
