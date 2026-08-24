//! Wire protocol between the compositor host and the browser frontend.
//!
//! One postcard-encoded message per WebSocket binary frame, in both
//! directions. Pixel payloads are raw RGBA8888 rows of the damage rect, so
//! the browser can hand them to `putImageData` without conversion.

use serde::{Deserialize, Serialize};

/// Bumped on any wire-incompatible change; both ends refuse a mismatch.
pub const VERSION: u32 = 4;

pub type WindowId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HostToClient {
    Hello {
        version: u32,
        host: String,
        output: Size,
    },
    WindowCreated {
        id: WindowId,
        app_id: String,
        title: String,
    },
    WindowTitle {
        id: WindowId,
        title: String,
    },
    WindowClosed {
        id: WindowId,
    },
    /// RGBA pixels of `damage`, tightly packed (stride = damage.width * 4).
    /// `size` is the full surface size the rect is positioned in. With
    /// `compressed`, `pixels` is an lz4 block with the raw length prepended
    /// ([`wire_pixels`] / [`unwire_pixels`]).
    Frame {
        id: WindowId,
        size: Size,
        damage: Rect,
        compressed: bool,
        #[serde(with = "serde_bytes")]
        pixels: Vec<u8>,
    },
    /// One encoded H.264 frame (Annex B) of a surface in video mode. A
    /// keyframe starts or restarts the stream; a plain Frame ends it.
    VideoFrame {
        id: WindowId,
        size: Size,
        keyframe: bool,
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },
    /// A menu, popover or tooltip: rendered as an overlay anchored at
    /// (x, y) in the parent surface's coordinates. Closed via WindowClosed
    /// and painted via Frame, like any window.
    PopupCreated {
        id: WindowId,
        parent: WindowId,
        x: i32,
        y: i32,
    },
    /// A client put text in the clipboard selection.
    Clipboard {
        text: String,
    },
    /// The pointer cursor to show, as a CSS cursor keyword.
    Cursor {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientToHost {
    Hello {
        version: u32,
        /// Whether this page decodes H.264 through WebCodecs; one client
        /// without it keeps every surface on the rect path.
        video: bool,
    },
    /// Surface-local coordinates.
    PointerMotion {
        id: WindowId,
        x: f64,
        y: f64,
    },
    /// `button` is a Linux input event code (BTN_LEFT = 0x110).
    PointerButton {
        id: WindowId,
        button: u32,
        pressed: bool,
    },
    PointerAxis {
        id: WindowId,
        dx: f64,
        dy: f64,
    },
    /// `code` is an evdev keycode.
    Key {
        code: u32,
        pressed: bool,
    },
    Focus {
        id: Option<WindowId>,
    },
    Close {
        id: WindowId,
    },
    Resize {
        id: WindowId,
        size: Size,
    },
    /// The browser clipboard, pushed so clients can paste it.
    Clipboard {
        text: String,
    },
}

impl HostToClient {
    pub fn encode(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }
}

/// Compress pixels for a Frame when it pays; UI content usually collapses,
/// photographic content is sent raw rather than inflated.
pub fn wire_pixels(pixels: &[u8]) -> (bool, Vec<u8>) {
    let compressed = lz4_flex::compress_prepend_size(pixels);
    if compressed.len() < pixels.len() / 10 * 9 {
        (true, compressed)
    } else {
        (false, pixels.to_vec())
    }
}

/// The inverse of [`wire_pixels`], applied by the receiving side.
pub fn unwire_pixels(compressed: bool, pixels: Vec<u8>) -> Option<Vec<u8>> {
    if compressed {
        lz4_flex::decompress_size_prepended(&pixels).ok()
    } else {
        Some(pixels)
    }
}

impl ClientToHost {
    pub fn encode(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrips() -> Result<(), postcard::Error> {
        let msg = HostToClient::Frame {
            id: 7,
            size: Size {
                width: 2,
                height: 2,
            },
            damage: Rect {
                x: 0,
                y: 1,
                width: 2,
                height: 1,
            },
            compressed: false,
            pixels: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        assert_eq!(
            HostToClient::decode(&msg.encode()?)?,
            msg,
            "a frame must roundtrip unchanged"
        );
        Ok(())
    }

    #[test]
    fn pixels_wire_roundtrips() {
        let flat = vec![7_u8; 4096];
        let (compressed, wire) = wire_pixels(&flat);
        assert!(compressed, "solid pixels must take the compressed path");
        assert!(
            wire.len() < flat.len() / 10,
            "solid pixels should collapse by an order of magnitude"
        );
        assert_eq!(
            unwire_pixels(compressed, wire).as_deref(),
            Some(flat.as_slice()),
            "wire pixels must roundtrip unchanged"
        );
    }

    #[test]
    fn hello_roundtrips() -> Result<(), postcard::Error> {
        let msg = ClientToHost::Hello {
            version: VERSION,
            video: true,
        };
        assert_eq!(
            ClientToHost::decode(&msg.encode()?)?,
            msg,
            "a hello must roundtrip unchanged"
        );
        Ok(())
    }
}
