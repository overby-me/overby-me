//! The data layer on the atproto AppView (roadmap M9).
//!
//! A second implementation of what `crate::graphql` is to the components: the
//! same questions, answered by `crates/appview` through its generated client
//! (`appview_client`) and handed over as the same `crate::model` types. Only
//! built under the `appview` feature; what ships is unchanged until the cutover.

// Until the switch: the layer is written a part at a time, and nothing calls a
// part before the whole stands in for `crate::graphql`. Goes with the switch.
#![allow(dead_code)]

pub mod map;
