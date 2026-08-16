//! Shared OOXML unit conversion constants. EMU (English Metric Units) is the
//! coordinate unit for DrawingML transforms (`<a:off>`/`<a:ext>`, ECMA-376
//! §20.1.7.6 `ST_PositiveCoordinate`); several legacy or pixel-space inputs
//! (VML anchors, raw raster dimensions at the CSS-pixel default DPI) need to
//! be converted into it.

/// EMU per pixel at the 96 DPI default used by VML (`x:Anchor` offsets,
/// [MS-OI29500] 2.1.639) and by CSS pixels in general: 914400 EMU/inch ÷ 96
/// px/inch = 9525 EMU/px.
pub const EMU_PER_PX_96DPI: i64 = 9525;
