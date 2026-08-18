//! gzenv implements a compressed format using json+zlib+base64.
//! Compatible with the Go direnv gzenv package.

use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use serde::{Serialize, de::DeserializeOwned};
use std::io::{Read, Write};

/// Marshal encodes an object into the gzenv format (json -> zlib -> base64url)
pub fn marshal<T: Serialize>(obj: &T) -> String {
    let json_data = serde_json::to_vec(obj).expect("gzenv marshal: json encoding failed");

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&json_data)
        .expect("gzenv marshal: zlib write failed");
    let zlib_data = encoder.finish().expect("gzenv marshal: zlib finish failed");

    URL_SAFE.encode(&zlib_data)
}

/// Unmarshal decodes the gzenv format back into an object (base64url -> zlib -> json)
pub fn unmarshal<T: DeserializeOwned>(gzenv_str: &str) -> Result<T, String> {
    let gzenv_str = gzenv_str.trim();

    let data = URL_SAFE
        .decode(gzenv_str)
        .map_err(|e| format!("unmarshal() base64 decoding: {e}"))?;

    let mut decoder = ZlibDecoder::new(&data[..]);
    let mut json_data = Vec::new();
    decoder
        .read_to_end(&mut json_data)
        .map_err(|e| format!("unmarshal() zlib decoding: {e}"))?;

    serde_json::from_slice(&json_data).map_err(|e| format!("unmarshal() json parsing: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_roundtrip() {
        let mut data: HashMap<String, String> = HashMap::new();
        data.insert("FOO".into(), "bar".into());
        data.insert("BAZ".into(), "qux".into());

        let encoded = marshal(&data);
        let decoded: HashMap<String, String> = unmarshal(&encoded).unwrap();
        assert_eq!(data, decoded);
    }
}
