//! Mapping between input filenames and the corresponding output filenames
//! for the various compressed formats xz understands.

use std::path::{Path, PathBuf};

use crate::options::Format;

/// Returns the canonical suffix for the format, or `None` if the
/// format has no fixed suffix (raw mode requires an explicit
/// `--suffix=`; lzip is decode-only here).
pub fn suffix_for_format(format: Format) -> Option<&'static str> {
    match format {
        Format::Xz | Format::Auto => Some(".xz"),
        Format::Lzma => Some(".lzma"),
        Format::Lzip => Some(".lz"),
        Format::Raw => None,
    }
}

pub fn compressed_suffixes() -> &'static [&'static str] {
    &[".xz", ".lzma", ".lz", ".txz", ".tlz"]
}

/// Returns the output filename when compressing `input` to `format`.
/// `custom_suffix` (from `--suffix=`) overrides the default if set.
pub fn output_path_compress(
    input: &Path,
    format: Format,
    custom_suffix: Option<&str>,
) -> Option<PathBuf> {
    let suffix = custom_suffix.or_else(|| suffix_for_format(format))?;
    let mut out = input.as_os_str().to_owned();
    out.push(suffix);
    Some(PathBuf::from(out))
}

/// Returns the output filename when decompressing `input`. If a
/// `custom_suffix` is set, it's the only suffix considered. Otherwise
/// the standard list (`.xz`, `.lzma`, `.lz`, `.txz`, `.tlz`) is tried.
pub fn output_path_decompress(input: &Path, custom_suffix: Option<&str>) -> Option<PathBuf> {
    let name = input.to_str()?;
    if let Some(suffix) = custom_suffix {
        return name.strip_suffix(suffix).map(PathBuf::from);
    }
    if let Some(stripped) = name.strip_suffix(".xz") {
        Some(PathBuf::from(stripped))
    } else if let Some(stripped) = name.strip_suffix(".lzma") {
        Some(PathBuf::from(stripped))
    } else if let Some(stripped) = name.strip_suffix(".lz") {
        Some(PathBuf::from(stripped))
    } else {
        name.strip_suffix(".txz")
            .or_else(|| name.strip_suffix(".tlz"))
            .map(|stripped| PathBuf::from(format!("{stripped}.tar")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_xz_suffix() {
        assert_eq!(
            output_path_compress(Path::new("foo"), Format::Xz, None),
            Some(PathBuf::from("foo.xz"))
        );
    }

    #[test]
    fn compress_lzma_suffix() {
        assert_eq!(
            output_path_compress(Path::new("foo"), Format::Lzma, None),
            Some(PathBuf::from("foo.lzma"))
        );
    }

    #[test]
    fn compress_custom_suffix_overrides_default() {
        assert_eq!(
            output_path_compress(Path::new("foo"), Format::Xz, Some(".bar")),
            Some(PathBuf::from("foo.bar"))
        );
    }

    #[test]
    fn compress_raw_without_suffix_returns_none() {
        assert_eq!(
            output_path_compress(Path::new("foo"), Format::Raw, None),
            None
        );
    }

    #[test]
    fn compress_raw_with_custom_suffix() {
        assert_eq!(
            output_path_compress(Path::new("foo"), Format::Raw, Some(".foo")),
            Some(PathBuf::from("foo.foo"))
        );
    }

    #[test]
    fn decompress_strips_xz() {
        assert_eq!(
            output_path_decompress(Path::new("foo.xz"), None),
            Some(PathBuf::from("foo"))
        );
    }

    #[test]
    fn decompress_strips_lzma() {
        assert_eq!(
            output_path_decompress(Path::new("foo.lzma"), None),
            Some(PathBuf::from("foo"))
        );
    }

    #[test]
    fn decompress_strips_lz() {
        assert_eq!(
            output_path_decompress(Path::new("foo.lz"), None),
            Some(PathBuf::from("foo"))
        );
    }

    #[test]
    fn decompress_rewrites_txz_to_tar() {
        assert_eq!(
            output_path_decompress(Path::new("bar.txz"), None),
            Some(PathBuf::from("bar.tar"))
        );
    }

    #[test]
    fn decompress_rewrites_tlz_to_tar() {
        assert_eq!(
            output_path_decompress(Path::new("bar.tlz"), None),
            Some(PathBuf::from("bar.tar"))
        );
    }

    #[test]
    fn decompress_unknown_returns_none() {
        assert_eq!(output_path_decompress(Path::new("foo.bin"), None), None);
    }

    #[test]
    fn decompress_with_custom_suffix() {
        assert_eq!(
            output_path_decompress(Path::new("foo.bar"), Some(".bar")),
            Some(PathBuf::from("foo"))
        );
        // Standard suffixes are NOT recognised when a custom one is set.
        assert_eq!(
            output_path_decompress(Path::new("foo.xz"), Some(".bar")),
            None
        );
    }

    #[test]
    fn compressed_suffixes_table() {
        let s = compressed_suffixes();
        assert!(s.contains(&".xz"));
        assert!(s.contains(&".lzma"));
        assert!(s.contains(&".lz"));
        assert!(s.contains(&".txz"));
        assert!(s.contains(&".tlz"));
    }
}
