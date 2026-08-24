//! Wire protocol between the compositor host and the browser frontend.
//!
//! One postcard-encoded message per WebSocket binary frame, in both
//! directions. Pixel payloads are raw RGBA8888 rows of the damage rect, so
//! the browser can hand them to `putImageData` without conversion.

use serde::{Deserialize, Serialize};

/// Bumped on any wire-incompatible change; both ends refuse a mismatch.
pub const VERSION: u32 = 0;

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

/// Host to browser.
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
    /// `size` is the full surface size the rect is positioned in.
    Frame {
        id: WindowId,
        size: Size,
        damage: Rect,
        #[serde(with = "serde_bytes")]
        pixels: Vec<u8>,
    },
}

/// Browser to host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientToHost {
    Hello {
        version: u32,
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
}

impl HostToClient {
    pub fn encode(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
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
    fn hello_roundtrips() -> Result<(), postcard::Error> {
        let msg = ClientToHost::Hello { version: VERSION };
        assert_eq!(
            ClientToHost::decode(&msg.encode()?)?,
            msg,
            "a hello must roundtrip unchanged"
        );
        Ok(())
    }
}
