//! OMML parsing now lives in the shared `ooxml-common` crate (it is identical
//! across docx/pptx/xlsx). Re-exported here so existing `crate::math::…` call
//! sites keep working.

pub use ooxml_common::math::*;
